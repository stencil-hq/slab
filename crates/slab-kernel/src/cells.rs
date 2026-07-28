//! Terminal-cell rasterization for flattened frames.
//!
//! One terminal cell represents 8 by 16 layout units. Geometry snaps to that
//! grid with round-half-even rounding. Rectangles become box-drawing borders
//! and background fills; gradients are sampled per cell. The document default
//! ink maps to terminal foreground. Other translucent paint below half coverage
//! over an unknown terminal default drops because cells cannot blend.
//! Paths, rotation, blur, and backdrops degrade according to the capability
//! notes accumulated on the grid.
//! The railyard research fixture is the byte-exact reference for this renderer.

use crate::{dispatch, edit, flatten, frame, graphemes, hit, list, slir, slir::Doc};

fn index(value: i32) -> usize {
    usize::try_from(value).expect("cell index must be nonnegative")
}

fn count(value: usize) -> i32 {
    i32::try_from(value).expect("cell collection exceeds i32 capacity")
}

/// Implements Rust's saturating float-to-integer cast without `as`.
fn truncate_i32(value: f64) -> i32 {
    if value.is_nan() {
        return 0;
    }
    if value >= f64::from(i32::MAX) {
        return i32::MAX;
    }
    if value <= f64::from(i32::MIN) {
        return i32::MIN;
    }

    let bits = value.to_bits();
    let exponent = i32::try_from((bits >> 52) & 0x7ff).expect("f64 exponent fits i32") - 1023;
    if exponent < 0 {
        return 0;
    }
    let significand = (bits & ((1_u64 << 52) - 1)) | (1_u64 << 52);
    let magnitude = if exponent < 52 {
        significand >> u32::try_from(52 - exponent).expect("nonnegative right shift")
    } else {
        significand << u32::try_from(exponent - 52).expect("nonnegative left shift")
    };
    let magnitude = i32::try_from(magnitude).expect("bounded f64 magnitude fits i32");
    if value.is_sign_negative() {
        magnitude.wrapping_neg()
    } else {
        magnitude
    }
}

/// Width of one terminal cell in layout units.
pub const CW: f64 = 8.0;

/// Height of one terminal cell in layout units.
pub const CH: f64 = 16.0;

/// Sentinel for no color.
///
/// Cell colors are packed as `0xRRGGBB`, so a value with its high byte set
/// cannot be a valid color.
pub const NO_COLOR: u32 = 0xFF00_0000;

// The inherited document ink is an internal style default, not authored paint.
const DOCUMENT_DEFAULT_INK: u32 = 0x1111_11FF;

/// Marks a cell occupied by the right half of a wide grapheme cluster.
pub const CONT: u32 = 0xFFFF_FFFF;

/// Indicates that a cell has an explicit foreground color.
pub const CF_FG: u32 = 1;

/// Indicates that a cell has an explicit background color.
pub const CF_BG: u32 = 2;

/// A row-major terminal cell grid and its rendering diagnostics.
#[derive(Clone, Debug)]
pub struct CellGrid {
    /// Grid width in cells.
    pub cols: i32,
    /// Grid height in cells.
    pub rows: i32,
    /// Primary code point for each cell, in row-major order.
    pub ch: Vec<u32>,
    /// Full grapheme cluster when the primary code point is insufficient.
    pub cl: Vec<String>,
    /// Packed `0xRRGGBB` foreground colors, valid when [`CF_FG`] is set.
    pub fg: Vec<u32>,
    /// Packed `0xRRGGBB` background colors, valid when [`CF_BG`] is set.
    pub bg: Vec<u32>,
    /// Per-cell [`CF_FG`] and [`CF_BG`] bits.
    pub flags: Vec<u32>,
    /// Capability-degradation codes, one entry per degradation class.
    pub diag_code: Vec<String>,
    /// Human-readable messages corresponding to `diag_code`.
    pub diag_msg: Vec<String>,
    /// Left edges of the cell clip stack; the last entry is current.
    pub clip_x0: Vec<i32>,
    /// Top edges of the cell clip stack; the last entry is current.
    pub clip_y0: Vec<i32>,
    /// Right edges of the cell clip stack; the last entry is current.
    pub clip_x1: Vec<i32>,
    /// Bottom edges of the cell clip stack; the last entry is current.
    pub clip_y1: Vec<i32>,
}

/// Rounds like Python's `round`: ties go to the nearest even integer.
pub fn rhe(v: f64) -> i32 {
    let floor = v.floor();
    let fraction = v - floor;
    if fraction > 0.5 || (fraction == 0.5 && (floor / 2.0).floor() != floor / 2.0) {
        truncate_i32(floor).wrapping_add(1)
    } else {
        truncate_i32(floor)
    }
}

/// Converts a horizontal layout coordinate to its nearest cell column.
pub fn cell_col(x: f64) -> i32 {
    rhe(x / CW)
}

/// Converts a vertical layout coordinate to its nearest cell row.
pub fn cell_row(y: f64) -> i32 {
    rhe(y / CH)
}

/// Counts terminal columns occupied by complete grapheme clusters before `end`.
pub fn text_columns_before(text: &str, end: i32) -> i32 {
    let end = end.clamp(0, crate::rt::str_len(text));
    let mut boundaries = Vec::new();
    graphemes::boundaries(text, &mut boundaries);
    let mut columns: i32 = 0;
    for boundary_pair in boundaries.windows(2) {
        if boundary_pair[1] > end {
            break;
        }
        columns = columns.wrapping_add(
            if graphemes::cluster_wide(text, boundary_pair[0], boundary_pair[1]) {
                2
            } else {
                1
            },
        );
    }
    columns
}

/// Converts an SLIR RGBA word (red in the low byte) to `0xRRGGBB`.
pub fn rgb_of(v: u32) -> u32 {
    ((v & 0xFF) << 16) | (((v >> 8) & 0xFF) << 8) | ((v >> 16) & 0xFF)
}

/// Returns the effective integer alpha after applying an opacity multiplier.
pub fn alpha_of(v: u32, opacity: f64) -> u32 {
    let alpha = truncate_i32((f64::from((v >> 24) & 0xFF) * opacity).round());
    u32::try_from(alpha.clamp(0, 255)).expect("clamped alpha fits u32")
}

/// Composites a `0xRRGGBB` source over a destination at alpha `0..=255`.
pub fn blend_rgb(dst: u32, src: u32, alpha: u32) -> u32 {
    let inverse = 255u32.wrapping_sub(alpha);
    let blend_channel = |shift: u32| {
        (((src >> shift) & 0xFF)
            .wrapping_mul(alpha)
            .wrapping_add(((dst >> shift) & 0xFF).wrapping_mul(inverse))
            .wrapping_add(127))
        .wrapping_div(255)
    };
    (blend_channel(16) << 16) | (blend_channel(8) << 8) | blend_channel(0)
}

/// Interpolates two SLIR RGBA words linearly in sRGB, per the stop-ramp rule.
pub fn rgba_lerp(a: u32, b: u32, t: f64) -> u32 {
    let mut out = 0;
    for shift in [0, 8, 16, 24] {
        let from = f64::from((a >> shift) & 0xFF);
        let to = f64::from((b >> shift) & 0xFF);
        let channel = truncate_i32((from + (to - from) * t).round());
        out |= u32::from_ne_bytes(channel.to_ne_bytes()) << shift;
    }
    out
}

/// Samples a gradient at normalized position `t`.
pub fn grad_sample(doc: &Doc, gradient: i32, t: f64) -> u32 {
    let gradient = index(gradient);
    let offset = doc.grad_stop_off[gradient];
    let stop_count = doc.grad_stop_len[gradient];
    if stop_count <= 0 {
        return 0;
    }
    if t <= doc.grad_stop_pos[index(offset)] {
        return doc.grad_stop_rgba[index(offset)];
    }
    for stop in 1..stop_count {
        let stop_index = offset.wrapping_add(stop);
        if t <= doc.grad_stop_pos[index(stop_index)] {
            let previous = stop_index.wrapping_sub(1);
            let p0 = doc.grad_stop_pos[index(previous)];
            let p1 = doc.grad_stop_pos[index(stop_index)];
            let local_t = if p1 > p0 { (t - p0) / (p1 - p0) } else { 0.0 };
            return rgba_lerp(
                doc.grad_stop_rgba[index(previous)],
                doc.grad_stop_rgba[index(stop_index)],
                local_t,
            );
        }
    }
    doc.grad_stop_rgba[index(offset.wrapping_add(stop_count).wrapping_sub(1))]
}

/// Samples a paint at page point `(px, py)` inside an operation box.
///
/// Solid paints pass through unchanged. Linear gradients use their CSS angle
/// and normalize the gradient line over the box projection; radial gradients
/// use the farthest corner; conic gradients sweep clockwise from up around
/// the box center. An alpha of zero means there is nothing to paint.
#[allow(clippy::too_many_arguments)]
pub fn paint_rgba_at(
    doc: &Doc,
    kind: u32,
    handle: u32,
    px: f64,
    py: f64,
    box_x: f64,
    box_y: f64,
    box_width: f64,
    box_height: f64,
) -> u32 {
    if kind == 1 {
        return handle;
    }
    if kind != 2 {
        return 0;
    }

    let gradient = i32::from_ne_bytes(handle.to_ne_bytes());
    if gradient < 0
        || gradient >= count(doc.grad_kind.len())
        || doc.grad_stop_len[index(gradient)] <= 0
    {
        return 0;
    }

    let gradient_index = index(gradient);
    let center_x = box_x + box_width / 2.0;
    let center_y = box_y + box_height / 2.0;
    let t = if doc.grad_kind[gradient_index] == 0 {
        let dx = hit::sin_deg(doc.grad_angle[gradient_index]);
        let dy = -hit::cos_deg(doc.grad_angle[gradient_index]);
        let line_length = (box_width * dx).abs() + (box_height * dy).abs();
        if line_length <= 0.0 {
            return doc.grad_stop_rgba[index(doc.grad_stop_off[gradient_index])];
        }
        ((px - (center_x - dx * line_length / 2.0)) * dx
            + (py - (center_y - dy * line_length / 2.0)) * dy)
            / line_length
    } else if doc.grad_kind[gradient_index] == 2 {
        let angle = (px - center_x).atan2(center_y - py).to_degrees();
        (angle - doc.grad_angle[gradient_index]).rem_euclid(360.0) / 360.0
    } else {
        let farthest_corner = (box_width * box_width / 4.0 + box_height * box_height / 4.0).sqrt();
        if farthest_corner <= 0.0 {
            return doc.grad_stop_rgba[index(doc.grad_stop_off[gradient_index])];
        }
        ((px - center_x) * (px - center_x) + (py - center_y) * (py - center_y)).sqrt()
            / farthest_corner
    };
    grad_sample(doc, gradient, t.clamp(0.0, 1.0))
}

/// One active fade mask: paint kind/handle and the border box it maps over.
#[derive(Clone, Copy, Debug)]
pub struct MaskCtx {
    pub kind: u32,
    pub handle: u32,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Combined mask coverage at a page point: the product of every active
/// mask's paint alpha over its box, and zero outside any mask box.
pub fn mask_alpha_at(doc: &Doc, masks: &[MaskCtx], px: f64, py: f64) -> f64 {
    let mut factor = 1.0;
    for mask in masks {
        if px < mask.x || px > mask.x + mask.w || py < mask.y || py > mask.y + mask.h {
            return 0.0;
        }
        let rgba = paint_rgba_at(
            doc,
            mask.kind,
            mask.handle,
            px,
            py,
            mask.x,
            mask.y,
            mask.w,
            mask.h,
        );
        factor *= f64::from(rgba >> 24) / 255.0;
    }
    factor
}

/// Minimum blend strength for stroke-derived outline characters.
///
/// A one-unit hairline at 8% alpha is visible on a pixel display but vanishes
/// as a cell glyph, so terminal outlines never fall below roughly 28%.
pub const STROKE_FLOOR: u32 = 72;

/// Records one terminal capability degradation.
pub fn note(grid: &mut CellGrid, code: &str, message: &str) {
    grid.diag_code.push(code.to_owned());
    grid.diag_msg.push(message.to_owned());
}

fn cell_index(grid: &CellGrid, column: i32, row: i32) -> Option<usize> {
    ((0..grid.cols).contains(&column) && (0..grid.rows).contains(&row))
        .then(|| index(row.wrapping_mul(grid.cols).wrapping_add(column)))
}

fn is_clipped_in(grid: &CellGrid, column: i32, row: i32) -> bool {
    let top = grid.clip_x0.len() - 1;
    column >= grid.clip_x0[top]
        && column < grid.clip_x1[top]
        && row >= grid.clip_y0[top]
        && row < grid.clip_y1[top]
}

fn blank_glyph_raw(grid: &mut CellGrid, index: usize) {
    grid.ch[index] = 32;
    grid.cl[index].clear();
    grid.flags[index] &= !CF_FG;
}

fn clear_wide_touching(grid: &mut CellGrid, column: i32, row: i32) {
    let Some(target) = cell_index(grid, column, row) else {
        return;
    };
    if grid.ch[target] == CONT {
        blank_glyph_raw(grid, target);
        if let Some(leading) = cell_index(grid, column.wrapping_sub(1), row) {
            blank_glyph_raw(grid, leading);
        }
    } else if let Some(continuation) = cell_index(grid, column.wrapping_add(1), row)
        && grid.ch[continuation] == CONT
    {
        blank_glyph_raw(grid, target);
        blank_glyph_raw(grid, continuation);
    }
}

fn put_raw(grid: &mut CellGrid, index: usize, ch: u32, fg: u32, bg: u32) {
    grid.ch[index] = ch;
    grid.cl[index].clear();
    if fg != NO_COLOR {
        grid.fg[index] = fg;
        grid.flags[index] |= CF_FG;
    }
    if bg != NO_COLOR {
        grid.bg[index] = bg;
        grid.flags[index] |= CF_BG;
    }
}

/// Writes one cell when it lies inside the grid and current clip.
pub fn put(grid: &mut CellGrid, column: i32, row: i32, ch: u32, fg: u32, bg: u32) {
    if !is_clipped_in(grid, column, row) {
        return;
    }
    let Some(index) = cell_index(grid, column, row) else {
        return;
    };
    clear_wide_touching(grid, column, row);
    put_raw(grid, index, ch, fg, bg);
}

/// Writes a complete grapheme cluster into one cell.
pub fn put_cluster(grid: &mut CellGrid, column: i32, row: i32, ch: u32, cluster: &str, fg: u32) {
    put(grid, column, row, ch, fg, NO_COLOR);
    if !is_clipped_in(grid, column, row) {
        return;
    }
    if let Some(index) = cell_index(grid, column, row) {
        grid.cl[index].clear();
        grid.cl[index].push_str(cluster);
    }
}

fn put_wide_cluster(grid: &mut CellGrid, column: i32, row: i32, ch: u32, cluster: &str, fg: u32) {
    let continuation_column = column.wrapping_add(1);
    if !is_clipped_in(grid, column, row) || !is_clipped_in(grid, continuation_column, row) {
        return;
    }
    let Some(leading) = cell_index(grid, column, row) else {
        return;
    };
    let Some(continuation) = cell_index(grid, continuation_column, row) else {
        return;
    };
    clear_wide_touching(grid, column, row);
    clear_wide_touching(grid, continuation_column, row);
    put_raw(grid, leading, ch, fg, NO_COLOR);
    grid.cl[leading].push_str(cluster);
    put_raw(grid, continuation, CONT, fg, NO_COLOR);
}

/// Returns a cell's known background, or [`NO_COLOR`] for the terminal default.
pub fn under_bg(grid: &CellGrid, column: i32, row: i32) -> u32 {
    let Some(index) = cell_index(grid, column, row) else {
        return NO_COLOR;
    };
    if grid.flags[index] & CF_BG != 0 {
        grid.bg[index]
    } else {
        NO_COLOR
    }
}

/// Composites a paint into one cell's background.
///
/// Majority-opaque paint wipes the glyph because paint order covers it.
/// Translucent washes tint a known background while retaining the glyph;
/// paint over an unknown terminal default is skipped.
pub fn bg_composite(grid: &mut CellGrid, column: i32, row: i32, rgb: u32, alpha: u32) {
    if !is_clipped_in(grid, column, row) || alpha == 0 {
        return;
    }
    let Some(index) = cell_index(grid, column, row) else {
        return;
    };
    if alpha >= 128 {
        clear_wide_touching(grid, column, row);
    }
    if grid.flags[index] & CF_BG != 0 {
        grid.bg[index] = blend_rgb(grid.bg[index], rgb, alpha);
    } else {
        if alpha < 128 {
            return;
        }
        grid.bg[index] = rgb;
        grid.flags[index] |= CF_BG;
    }
    if alpha >= 128 {
        blank_glyph_raw(grid, index);
    }
}

/// Pushes the intersection of a cell clip and the current clip.
pub fn push_clip(grid: &mut CellGrid, x0: i32, y0: i32, x1: i32, y1: i32) {
    let top = grid.clip_x0.len() - 1;
    grid.clip_x0.push(grid.clip_x0[top].max(x0));
    grid.clip_y0.push(grid.clip_y0[top].max(y0));
    grid.clip_x1.push(grid.clip_x1[top].min(x1));
    grid.clip_y1.push(grid.clip_y1[top].min(y1));
}

/// Pops the current cell clip, retaining the root grid clip.
pub fn pop_clip(grid: &mut CellGrid) {
    if grid.clip_x0.len() > 1 {
        grid.clip_x0.pop();
        grid.clip_y0.pop();
        grid.clip_x1.pop();
        grid.clip_y1.pop();
    }
}

/// Composites a stroke over a cell without dropping below the legibility floor.
pub fn stroke_fg(grid: &CellGrid, column: i32, row: i32, rgba: u32, opacity: f64) -> u32 {
    let alpha = alpha_of(rgba, opacity);
    if alpha < 8 {
        return NO_COLOR;
    }
    let alpha = alpha.max(STROKE_FLOOR);
    let base = under_bg(grid, column, row);
    if base == NO_COLOR {
        rgb_of(rgba)
    } else {
        blend_rgb(base, rgb_of(rgba), alpha)
    }
}
/// Rasterizes a rectangle as a fill, outline, or one-cell hairline.
///
/// Fills sample gradients and composite alpha per cell. Outlines use
/// box-drawing characters, with rounded corners for radii of at least four
/// layout units and dashed variants when requested.
pub fn draw_rect(
    doc: &Doc,
    grid: &mut CellGrid,
    rect: &flatten::OpRect,
    opacity: f64,
    masks: &[MaskCtx],
) {
    let left = cell_col(rect.x);
    let top = cell_row(rect.y);
    let right = cell_col(rect.x + rect.w);
    let bottom = cell_row(rect.y + rect.h);
    let effective_opacity = opacity * rect.opacity;

    // A short, wide rectangle degrades to a horizontal hairline. Its
    // background paint wins when visible; otherwise the stroke is sampled.
    if rect.h <= CH * 0.6 && rect.w > CW {
        let (mut paint_kind, mut paint) = (rect.bg_kind, rect.bg);
        if alpha_of(
            paint_rgba_at(
                doc, paint_kind, paint, rect.x, rect.y, rect.x, rect.y, rect.w, rect.h,
            ),
            effective_opacity,
        ) < 8
        {
            (paint_kind, paint) = (rect.stroke_kind, rect.stroke);
        }
        let row = cell_row(rect.y + rect.h / 2.0 - CH / 2.0);
        let ch = if rect.has_dash { 0x254C } else { 0x2500 }; // ╌ or ─
        for column in left..right {
            let point_x = (f64::from(column) + 0.5) * CW;
            let rgba = paint_rgba_at(
                doc, paint_kind, paint, point_x, rect.y, rect.x, rect.y, rect.w, rect.h,
            );
            let masked =
                effective_opacity * mask_alpha_at(doc, masks, point_x, (f64::from(row) + 0.5) * CH);
            let foreground = stroke_fg(grid, column, row, rgba, masked);
            if foreground != NO_COLOR {
                put(grid, column, row, ch, foreground, NO_COLOR);
            }
        }
        return;
    }

    // A narrow, tall rectangle similarly degrades to a vertical hairline.
    if rect.w <= CW * 0.9 && rect.h > CH {
        let (mut paint_kind, mut paint) = (rect.bg_kind, rect.bg);
        if alpha_of(
            paint_rgba_at(
                doc, paint_kind, paint, rect.x, rect.y, rect.x, rect.y, rect.w, rect.h,
            ),
            effective_opacity,
        ) < 8
        {
            (paint_kind, paint) = (rect.stroke_kind, rect.stroke);
        }
        let column = cell_col(rect.x + rect.w / 2.0 - CW / 2.0);
        let ch = if rect.has_dash { 0x254E } else { 0x2502 }; // ╎ or │
        for row in top..bottom {
            let point_y = (f64::from(row) + 0.5) * CH;
            let rgba = paint_rgba_at(
                doc, paint_kind, paint, rect.x, point_y, rect.x, rect.y, rect.w, rect.h,
            );
            let masked = effective_opacity
                * mask_alpha_at(doc, masks, (f64::from(column) + 0.5) * CW, point_y);
            let foreground = stroke_fg(grid, column, row, rgba, masked);
            if foreground != NO_COLOR {
                put(grid, column, row, ch, foreground, NO_COLOR);
            }
        }
        return;
    }

    // Fill is gradient-aware and alpha-composited at each cell center.
    if rect.bg_kind != 0 {
        for row in top..bottom {
            for column in left..right {
                let point_x = (f64::from(column) + 0.5) * CW;
                let point_y = (f64::from(row) + 0.5) * CH;
                let rgba = paint_rgba_at(
                    doc,
                    rect.bg_kind,
                    rect.bg,
                    point_x,
                    point_y,
                    rect.x,
                    rect.y,
                    rect.w,
                    rect.h,
                );
                bg_composite(
                    grid,
                    column,
                    row,
                    rgb_of(rgba),
                    alpha_of(
                        rgba,
                        effective_opacity * mask_alpha_at(doc, masks, point_x, point_y),
                    ),
                );
            }
        }
    }

    draw_rect_outline(doc, grid, rect, opacity, masks, OutlineClaim::Any);
}

/// Which ring cells an outline pass may claim.
#[derive(Clone, Copy, PartialEq)]
enum OutlineClaim {
    /// The initial paint claims every ring cell; later ops may cover it.
    Any,
    /// The post-clip re-assertion refills only cells left without a glyph,
    /// so children that legitimately painted over the ring keep their ink.
    Vacant,
}

fn claims(grid: &CellGrid, claim: OutlineClaim, column: i32, row: i32) -> bool {
    match claim {
        OutlineClaim::Any => true,
        OutlineClaim::Vacant => cell_index(grid, column, row)
            .is_some_and(|index| grid.ch[index] == 32 && grid.flags[index] & CF_FG == 0),
    }
}

/// Rasterizes a rectangle's stroke outline — the box-drawing edge and corner
/// pass of [`draw_rect`]. Outlines use rounded corners at radius >= 4.
///
/// [`cells_from_frame`] re-runs it with [`OutlineClaim::Vacant`] when a
/// stroked clip container closes: the stroke band is sub-cell, so a child
/// fill flush with the box (artwork, images) wipes the border glyphs and
/// would otherwise leave the frame open.
fn draw_rect_outline(
    doc: &Doc,
    grid: &mut CellGrid,
    rect: &flatten::OpRect,
    opacity: f64,
    masks: &[MaskCtx],
    claim: OutlineClaim,
) {
    let left = cell_col(rect.x);
    let top = cell_row(rect.y);
    let right = cell_col(rect.x + rect.w);
    let bottom = cell_row(rect.y + rect.h);
    let effective_opacity = opacity * rect.opacity;
    if rect.stroke_kind == 0 || right.wrapping_sub(left) < 2 || bottom.wrapping_sub(top) < 2 {
        return;
    }
    let (top_left, top_right, bottom_left, bottom_right) = if rect.radius >= 4.0 {
        (0x256D, 0x256E, 0x2570, 0x256F) // ╭ ╮ ╰ ╯
    } else {
        (0x250C, 0x2510, 0x2514, 0x2518) // ┌ ┐ └ ┘
    };
    let (horizontal, vertical) = if rect.has_dash {
        (0x254C, 0x254E) // ╌ ╎
    } else {
        (0x2500, 0x2502) // ─ │
    };
    let last_row = bottom.wrapping_sub(1);
    let last_column = right.wrapping_sub(1);

    for column in left.wrapping_add(1)..last_column {
        let point_x = (f64::from(column) + 0.5) * CW;
        if claims(grid, claim, column, top) {
            let top_rgba = paint_rgba_at(
                doc,
                rect.stroke_kind,
                rect.stroke,
                point_x,
                rect.y,
                rect.x,
                rect.y,
                rect.w,
                rect.h,
            );
            let top_fg = stroke_fg(
                grid,
                column,
                top,
                top_rgba,
                effective_opacity * mask_alpha_at(doc, masks, point_x, rect.y),
            );
            if top_fg != NO_COLOR {
                put(grid, column, top, horizontal, top_fg, NO_COLOR);
            }
        }

        if claims(grid, claim, column, last_row) {
            let bottom_rgba = paint_rgba_at(
                doc,
                rect.stroke_kind,
                rect.stroke,
                point_x,
                rect.y + rect.h,
                rect.x,
                rect.y,
                rect.w,
                rect.h,
            );
            let bottom_fg = stroke_fg(
                grid,
                column,
                last_row,
                bottom_rgba,
                effective_opacity * mask_alpha_at(doc, masks, point_x, rect.y + rect.h),
            );
            if bottom_fg != NO_COLOR {
                put(grid, column, last_row, horizontal, bottom_fg, NO_COLOR);
            }
        }
    }

    for row in top.wrapping_add(1)..last_row {
        let point_y = (f64::from(row) + 0.5) * CH;
        if claims(grid, claim, left, row) {
            let left_rgba = paint_rgba_at(
                doc,
                rect.stroke_kind,
                rect.stroke,
                rect.x,
                point_y,
                rect.x,
                rect.y,
                rect.w,
                rect.h,
            );
            let left_fg = stroke_fg(
                grid,
                left,
                row,
                left_rgba,
                effective_opacity * mask_alpha_at(doc, masks, rect.x, point_y),
            );
            if left_fg != NO_COLOR {
                put(grid, left, row, vertical, left_fg, NO_COLOR);
            }
        }

        if claims(grid, claim, last_column, row) {
            let right_rgba = paint_rgba_at(
                doc,
                rect.stroke_kind,
                rect.stroke,
                rect.x + rect.w,
                point_y,
                rect.x,
                rect.y,
                rect.w,
                rect.h,
            );
            let right_fg = stroke_fg(
                grid,
                last_column,
                row,
                right_rgba,
                effective_opacity * mask_alpha_at(doc, masks, rect.x + rect.w, point_y),
            );
            if right_fg != NO_COLOR {
                put(grid, last_column, row, vertical, right_fg, NO_COLOR);
            }
        }
    }

    // All four corners intentionally share the top-left sample so a gradient
    // cannot fragment the visual weight of one box outline.
    let corner_rgba = paint_rgba_at(
        doc,
        rect.stroke_kind,
        rect.stroke,
        rect.x,
        rect.y,
        rect.x,
        rect.y,
        rect.w,
        rect.h,
    );
    let corner_fg = stroke_fg(
        grid,
        left,
        top,
        corner_rgba,
        effective_opacity * mask_alpha_at(doc, masks, rect.x, rect.y),
    );
    if corner_fg != NO_COLOR {
        if claims(grid, claim, left, top) {
            put(grid, left, top, top_left, corner_fg, NO_COLOR);
        }
        if claims(grid, claim, last_column, top) {
            put(grid, last_column, top, top_right, corner_fg, NO_COLOR);
        }
        if claims(grid, claim, left, last_row) {
            put(grid, left, last_row, bottom_left, corner_fg, NO_COLOR);
        }
        if claims(grid, claim, last_column, last_row) {
            put(
                grid,
                last_column,
                last_row,
                bottom_right,
                corner_fg,
                NO_COLOR,
            );
        }
    }
}

/// Draws grapheme clusters with terminal-width clipping and composited color.
///
/// Unauthored document ink becomes [`NO_COLOR`], which preserves terminal
/// foreground. Gradient color and active fade masks sample per cell.
pub fn draw_text(
    doc: &Doc,
    grid: &mut CellGrid,
    frame: &flatten::Frame,
    text_op: &flatten::OpText,
    opacity: f64,
    masks: &[MaskCtx],
) {
    let start_column = cell_col(text_op.x);
    // A terminal cell baseline sits 12 layout units into the cell.
    let row = cell_row(text_op.y_baseline - 12.0);
    let base_opacity = opacity * text_op.opacity;
    let terminal_default = text_op.color_kind == 1 && text_op.color == DOCUMENT_DEFAULT_INK;
    if !terminal_default
        && text_op.color_kind != 2
        && masks.is_empty()
        && alpha_of(text_op.color, base_opacity) < 8
    {
        return;
    }

    let text = &frame.strings[index(text_op.str_ref)];
    let mut boundaries = Vec::new();
    graphemes::boundaries(text, &mut boundaries);
    let mut column_offset = 0;
    for boundary_pair in boundaries.windows(2) {
        let start = boundary_pair[0];
        let end = boundary_pair[1];
        let wide = graphemes::cluster_wide(text, start, end);
        let column = start_column.wrapping_add(column_offset);
        let clip = grid.clip_x0.len() - 1;

        if wide
            && (column < 0
                || column.wrapping_add(1) >= grid.cols
                || column < grid.clip_x0[clip]
                || column.wrapping_add(1) >= grid.clip_x1[clip]
                || row < grid.clip_y0[clip]
                || row >= grid.clip_y1[clip])
        {
            if column >= grid.clip_x0[clip] && row >= grid.clip_y0[clip] && row < grid.clip_y1[clip]
            {
                break;
            }
            column_offset = column_offset.wrapping_add(2);
            continue;
        }

        let point_x = (f64::from(column) + 0.5) * CW;
        let point_y = (f64::from(row) + 0.5) * CH;
        let rgba = if text_op.color_kind == 2 {
            paint_rgba_at(
                doc,
                2,
                text_op.color,
                point_x,
                point_y,
                text_op.gx,
                text_op.gy,
                text_op.gw,
                text_op.gh,
            )
        } else {
            text_op.color
        };
        let coverage = base_opacity * mask_alpha_at(doc, masks, point_x, point_y);
        let mut foreground = if terminal_default {
            NO_COLOR
        } else {
            rgb_of(rgba)
        };
        let background = under_bg(grid, column, row);
        if terminal_default {
            if coverage < 0.5 {
                column_offset = column_offset.wrapping_add(if wide { 2 } else { 1 });
                continue;
            }
        } else {
            let alpha = alpha_of(rgba, coverage);
            if background != NO_COLOR {
                foreground = blend_rgb(background, foreground, alpha);
            } else if alpha < 128 {
                column_offset = column_offset.wrapping_add(if wide { 2 } else { 1 });
                continue;
            }
        }

        let cluster = crate::rt::str_slice(text, start, end);
        let first = cluster.chars().next().map_or(32, u32::from);
        let full_cluster = if end.wrapping_sub(start) > 1 {
            cluster
        } else {
            String::new()
        };
        if wide {
            put_wide_cluster(grid, column, row, first, &full_cluster, foreground);
        } else {
            put_cluster(grid, column, row, first, &full_cluster, foreground);
        }
        column_offset = column_offset.wrapping_add(if wide { 2 } else { 1 });
    }
}

/// Flattens one path-bank entry, offset by `(dx, dy)`, into polylines.
///
/// `starts` records each subpath's first point. Cubic and quadratic curves are
/// subdivided into eight chords. Close-path is implicit: fills treat every
/// subpath as closed, and the stroke walker rejoins its start in draw order.
pub fn path_polylines(
    doc: &Doc,
    path: i32,
    dx: f64,
    dy: f64,
    xs: &mut Vec<f64>,
    ys: &mut Vec<f64>,
    starts: &mut Vec<i32>,
) {
    let Ok(path) = usize::try_from(path) else {
        return;
    };
    let Some((&verb_offset, &verb_count)) =
        doc.path_verb_off.get(path).zip(doc.path_verb_len.get(path))
    else {
        return;
    };
    let Some((&coord_offset, &coord_count)) = doc
        .path_coord_off
        .get(path)
        .zip(doc.path_coord_len.get(path))
    else {
        return;
    };
    let Some(verbs) = usize::try_from(verb_offset)
        .ok()
        .zip(usize::try_from(verb_count).ok())
        .and_then(|(offset, length)| doc.path_verbs.get(offset..offset.checked_add(length)?))
    else {
        return;
    };
    let Some(coords) = usize::try_from(coord_offset)
        .ok()
        .zip(usize::try_from(coord_count).ok())
        .and_then(|(offset, length)| doc.path_coords.get(offset..offset.checked_add(length)?))
    else {
        return;
    };
    path_polylines_data(verbs, coords, dx, dy, xs, ys, starts);
}

#[allow(clippy::too_many_arguments)]
fn path_polylines_data<T: Copy + Into<u32>>(
    verbs: &[T],
    coords: &[f64],
    dx: f64,
    dy: f64,
    xs: &mut Vec<f64>,
    ys: &mut Vec<f64>,
    starts: &mut Vec<i32>,
) {
    let mut coordinate = 0usize;
    for &raw_verb in verbs {
        let verb: u32 = raw_verb.into();
        match verb {
            0 => {
                starts.push(count(xs.len()));
                xs.push(coords[coordinate] + dx);
                ys.push(coords[coordinate + 1] + dy);
                coordinate += 2;
            }
            1 => {
                xs.push(coords[coordinate] + dx);
                ys.push(coords[coordinate + 1] + dy);
                coordinate += 2;
            }
            2 | 3 => {
                let x0 = *xs.last().expect("curve must follow a path point");
                let y0 = *ys.last().expect("curve must follow a path point");
                let control_x1 = coords[coordinate] + dx;
                let control_y1 = coords[coordinate + 1] + dy;
                let (control_x2, control_y2, end_x, end_y, consumed) = if verb == 2 {
                    (
                        coords[coordinate + 2] + dx,
                        coords[coordinate + 3] + dy,
                        coords[coordinate + 4] + dx,
                        coords[coordinate + 5] + dy,
                        6,
                    )
                } else {
                    (
                        control_x1,
                        control_y1,
                        coords[coordinate + 2] + dx,
                        coords[coordinate + 3] + dy,
                        4,
                    )
                };
                coordinate += consumed;
                for step in 1..9 {
                    let t = f64::from(step) / 8.0;
                    let u = 1.0 - t;
                    let (x, y) = if verb == 2 {
                        (
                            u * u * u * x0
                                + 3.0 * u * u * t * control_x1
                                + 3.0 * u * t * t * control_x2
                                + t * t * t * end_x,
                            u * u * u * y0
                                + 3.0 * u * u * t * control_y1
                                + 3.0 * u * t * t * control_y2
                                + t * t * t * end_y,
                        )
                    } else {
                        (
                            u * u * x0 + 2.0 * u * t * control_x1 + t * t * end_x,
                            u * u * y0 + 2.0 * u * t * control_y1 + t * t * end_y,
                        )
                    };
                    xs.push(x);
                    ys.push(y);
                }
            }
            _ => {}
        }
    }
}

/// Degrades a path into an even-odd cell-center fill and slope-character strokes.
///
/// Strokes use `─`, `│`, `╱`, and `╲` cell runs. Sub-cell detail is beyond the
/// terminal medium, but this representation keeps waveforms, ridges, and
/// sparklines legible.
pub fn draw_path(
    doc: &Doc,
    frame: &flatten::Frame,
    grid: &mut CellGrid,
    path: &flatten::OpPath,
    opacity: f64,
    masks: &[MaskCtx],
) {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut starts = Vec::new();
    if path.path < 0 {
        let Some(runtime) = usize::try_from(!path.path)
            .ok()
            .and_then(|index| frame.paths_rt.get(index))
        else {
            return;
        };
        path_polylines_data(
            &runtime.verbs,
            &runtime.coords,
            path.dx,
            path.dy,
            &mut xs,
            &mut ys,
            &mut starts,
        );
    } else {
        path_polylines(
            doc,
            path.path,
            path.dx,
            path.dy,
            &mut xs,
            &mut ys,
            &mut starts,
        );
    }
    if xs.len() < 2 {
        return;
    }

    let effective_opacity = opacity * path.opacity;
    let mut min_x = xs[0];
    let mut min_y = ys[0];
    let mut max_x = xs[0];
    let mut max_y = ys[0];
    for (&x, &y) in xs.iter().zip(&ys).skip(1) {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    if path.bg_kind != 0 {
        let first_column = truncate_i32((min_x / CW).floor());
        let first_row = truncate_i32((min_y / CH).floor());
        let last_column = truncate_i32((max_x / CW).ceil()).wrapping_add(1);
        let last_row = truncate_i32((max_y / CH).ceil()).wrapping_add(1);
        for row in first_row..last_row {
            for column in first_column..last_column {
                let point_x = (f64::from(column) + 0.5) * CW;
                let point_y = (f64::from(row) + 0.5) * CH;
                // Even-odd crossings against every closed subpath edge.
                let mut inside = false;
                for (subpath_index, start) in starts.iter().enumerate() {
                    let start = index(*start);
                    let end = starts
                        .get(subpath_index + 1)
                        .map_or(xs.len(), |next| index(*next));
                    for edge in start..end {
                        let next = if edge + 1 == end { start } else { edge + 1 };
                        let y1 = ys[edge];
                        let y2 = ys[next];
                        if (y1 > point_y) != (y2 > point_y) {
                            let intersection_x =
                                xs[edge] + (point_y - y1) * (xs[next] - xs[edge]) / (y2 - y1);
                            if point_x < intersection_x {
                                inside = !inside;
                            }
                        }
                    }
                }
                if inside {
                    let rgba = paint_rgba_at(
                        doc,
                        path.bg_kind,
                        path.bg,
                        point_x,
                        point_y,
                        min_x,
                        min_y,
                        max_x - min_x,
                        max_y - min_y,
                    );
                    bg_composite(
                        grid,
                        column,
                        row,
                        rgb_of(rgba),
                        alpha_of(
                            rgba,
                            effective_opacity * mask_alpha_at(doc, masks, point_x, point_y),
                        ),
                    );
                }
            }
        }
    }

    if path.stroke_kind == 0 {
        return;
    }
    for (subpath_index, start) in starts.iter().enumerate() {
        let start = index(*start);
        let end = starts
            .get(subpath_index + 1)
            .map_or(xs.len(), |next| index(*next));
        for edge in start..end.saturating_sub(1) {
            let x1 = xs[edge];
            let y1 = ys[edge];
            let x2 = xs[edge + 1];
            let y2 = ys[edge + 1];
            let horizontal_cells = (x2 - x1).abs() / CW;
            let vertical_cells = (y2 - y1).abs() / CH;
            let ch = if vertical_cells <= horizontal_cells * 0.5 {
                if path.has_dash { 0x254C } else { 0x2500 } // ╌ or ─
            } else if horizontal_cells <= vertical_cells * 0.5 {
                if path.has_dash { 0x254E } else { 0x2502 } // ╎ or │
            } else if (x2 - x1) * (y2 - y1) < 0.0 {
                0x2571 // ╱
            } else {
                0x2572 // ╲
            };
            let steps = truncate_i32(horizontal_cells.max(vertical_cells).ceil()).max(1);
            for step in 0..steps.wrapping_add(1) {
                let t = f64::from(step) / f64::from(steps);
                let point_x = x1 + (x2 - x1) * t;
                let point_y = y1 + (y2 - y1) * t;
                let column = truncate_i32((point_x / CW).floor());
                let row = truncate_i32((point_y / CH).floor());
                let rgba = paint_rgba_at(
                    doc,
                    path.stroke_kind,
                    path.stroke,
                    point_x,
                    point_y,
                    min_x,
                    min_y,
                    max_x - min_x,
                    max_y - min_y,
                );
                let foreground = stroke_fg(
                    grid,
                    column,
                    row,
                    rgba,
                    effective_opacity * mask_alpha_at(doc, masks, point_x, point_y),
                );
                if foreground != NO_COLOR {
                    put(grid, column, row, ch, foreground, NO_COLOR);
                }
            }
        }
    }
}

/// Draws an image placeholder and a centered, truncated source basename.
///
/// Placeholder cells are opaque puts, so fade masks degrade to a half-alpha
/// threshold: cells whose combined mask coverage falls below 0.5 drop.
pub fn draw_image(doc: &Doc, grid: &mut CellGrid, image: &flatten::OpImage, masks: &[MaskCtx]) {
    let left = cell_col(image.x);
    let top = cell_row(image.y);
    let right = cell_col(image.x + image.w);
    let bottom = cell_row(image.y + image.h);
    for row in top..bottom {
        for column in left..right {
            let point_x = (f64::from(column) + 0.5) * CW;
            let point_y = (f64::from(row) + 0.5) * CH;
            if mask_alpha_at(doc, masks, point_x, point_y) < 0.5 {
                continue;
            }
            // Medium-shade placeholder, rgb(120, 128, 140).
            put(grid, column, row, 0x2592, 0x78_80_8C, NO_COLOR);
        }
    }

    let source = if image.img >= 0 && image.img < count(doc.img_src.len()) {
        crate::slir::str_at(doc, doc.img_src[index(image.img)])
    } else {
        String::new()
    };
    let source_codepoints: Vec<u32> = source.chars().map(u32::from).collect();
    let basename_start = source_codepoints
        .iter()
        .rposition(|codepoint| *codepoint == u32::from('/'))
        .map_or(0, |index| index + 1);
    let max_label_len = index(right.wrapping_sub(left).wrapping_sub(2).max(0));
    let mut label = if basename_start == source_codepoints.len() {
        "img".chars().map(u32::from).collect::<Vec<_>>()
    } else {
        source_codepoints[basename_start..].to_vec()
    };
    label.truncate(max_label_len);

    let label_column = left.wrapping_add(
        right
            .wrapping_sub(left)
            .wrapping_sub(count(label.len()))
            .wrapping_div(2)
            .max(0),
    );
    let label_row = top.wrapping_add(bottom.wrapping_sub(top).wrapping_div(2).max(0));
    for (offset, codepoint) in label.into_iter().enumerate() {
        // Image label foreground, rgb(230, 233, 238).
        put(
            grid,
            label_column.wrapping_add(count(offset)),
            label_row,
            codepoint,
            0xE6_E9_EE,
            NO_COLOR,
        );
    }
}

/// Rasterizes a frame into a terminal cell grid.
///
/// `width` and `height` are the solved document box, normally the frame's own
/// dimensions. Frame operations paint in array order. Blur remains beyond the
/// cell medium, while group opacity multiplies through nested groups.
pub fn cells_from_frame(doc: &Doc, frame: &flatten::Frame, width: f64, height: f64) -> CellGrid {
    let cols = rhe(width / CW).max(1);
    let rows = rhe(height / CH).max(1);
    let cell_count = index(cols.wrapping_mul(rows));
    let mut grid = CellGrid {
        cols,
        rows,
        ch: vec![32; cell_count],
        cl: vec![String::new(); cell_count],
        fg: vec![0; cell_count],
        bg: vec![0; cell_count],
        flags: vec![0; cell_count],
        diag_code: Vec::new(),
        diag_msg: Vec::new(),
        clip_x0: vec![0],
        clip_y0: vec![0],
        clip_x1: vec![cols],
        clip_y1: vec![rows],
    };
    let mut rotation_depth: i32 = 0;
    let mut warned_rotate = false;
    let mut warned_backdrop = false;
    let mut warned_grain = false;
    let mut opacity_stack = vec![1.0];
    let mut masks: Vec<MaskCtx> = Vec::new();
    // Parallel to `opacity_stack` minus the root: whether each group pushed a mask.
    let mut group_masked: Vec<bool> = Vec::new();
    // Stroked clip containers re-assert their outline when the clip closes:
    // the stroke band is sub-cell, so a child fill flush with the box wipes
    // the border glyphs and would otherwise leave the frame open.
    let mut clip_outlines: Vec<Option<&flatten::OpRect>> = Vec::new();

    for (op_index, operation) in frame.ops.iter().enumerate() {
        match operation {
            flatten::FrameOp::RotatePush(_) => {
                rotation_depth = rotation_depth.wrapping_add(1);
                if !warned_rotate {
                    note(
                        &mut grid,
                        "cap-transform",
                        "tui renderer cannot rotate; rotated subtree skipped (redesign with `when tui`)",
                    );
                    warned_rotate = true;
                }
            }
            flatten::FrameOp::RotatePop => {
                if rotation_depth > 0 {
                    rotation_depth = rotation_depth.wrapping_sub(1);
                }
            }
            flatten::FrameOp::TiltPush(_) => {
                rotation_depth = rotation_depth.wrapping_add(1);
                if !warned_rotate {
                    note(
                        &mut grid,
                        "cap-transform",
                        "tui renderer cannot tilt; tilted subtree skipped (redesign with `when tui`)",
                    );
                    warned_rotate = true;
                }
            }
            flatten::FrameOp::TiltPop => {
                if rotation_depth > 0 {
                    rotation_depth = rotation_depth.wrapping_sub(1);
                }
            }
            flatten::FrameOp::ScalePush(_) => {
                if !warned_rotate {
                    note(
                        &mut grid,
                        "cap-transform",
                        "tui renderer ignores scaling; vector paths render at authored coordinates",
                    );
                    warned_rotate = true;
                }
            }
            flatten::FrameOp::ScalePop => {}
            flatten::FrameOp::Rect(rect) if rotation_depth == 0 => {
                if rect.grain_amount > 0.0 && !warned_grain {
                    note(
                        &mut grid,
                        "cap-grain",
                        "tui renderer cannot speckle; grain overlay skipped",
                    );
                    warned_grain = true;
                }
                draw_rect(
                    doc,
                    &mut grid,
                    rect,
                    *opacity_stack.last().expect("opacity stack has a root"),
                    &masks,
                );
            }
            flatten::FrameOp::Text(text) if rotation_depth == 0 => {
                draw_text(
                    doc,
                    &mut grid,
                    frame,
                    text,
                    *opacity_stack.last().expect("opacity stack has a root"),
                    &masks,
                );
            }
            flatten::FrameOp::Image(image) if rotation_depth == 0 => {
                draw_image(doc, &mut grid, image, &masks);
            }
            flatten::FrameOp::PathDraw(path) if rotation_depth == 0 => {
                draw_path(
                    doc,
                    frame,
                    &mut grid,
                    path,
                    *opacity_stack.last().expect("opacity stack has a root"),
                    &masks,
                );
            }
            flatten::FrameOp::ClipPush(clip) if rotation_depth == 0 => {
                let outlined = match op_index.checked_sub(1).map(|i| &frame.ops[i]) {
                    Some(flatten::FrameOp::Rect(rect))
                        if rect.stroke_kind != 0
                            && rect.x == clip.x
                            && rect.y == clip.y
                            && rect.w == clip.w
                            && rect.h == clip.h =>
                    {
                        Some(rect)
                    }
                    _ => None,
                };
                clip_outlines.push(outlined);
                push_clip(
                    &mut grid,
                    cell_col(clip.x),
                    cell_row(clip.y),
                    cell_col(clip.x + clip.w),
                    cell_row(clip.y + clip.h),
                );
            }
            flatten::FrameOp::ClipPop if rotation_depth == 0 => {
                pop_clip(&mut grid);
                if let Some(Some(rect)) = clip_outlines.pop() {
                    draw_rect_outline(
                        doc,
                        &mut grid,
                        rect,
                        *opacity_stack.last().expect("opacity stack has a root"),
                        &masks,
                        OutlineClaim::Vacant,
                    );
                }
            }
            flatten::FrameOp::GroupPush(group) => {
                opacity_stack
                    .push(*opacity_stack.last().expect("opacity stack has a root") * group.opacity);
                let masked = group.mask_kind != 0;
                if masked {
                    masks.push(MaskCtx {
                        kind: group.mask_kind,
                        handle: group.mask,
                        x: group.mx,
                        y: group.my,
                        w: group.mw,
                        h: group.mh,
                    });
                }
                group_masked.push(masked);
            }
            flatten::FrameOp::GroupPop => {
                if opacity_stack.len() > 1 {
                    opacity_stack.pop();
                }
                if group_masked.pop() == Some(true) {
                    masks.pop();
                }
            }
            flatten::FrameOp::Backdrop(_) if rotation_depth == 0 && !warned_backdrop => {
                note(
                    &mut grid,
                    "cap-backdrop",
                    "tui renderer cannot blur backdrops; panel paints flat",
                );
                warned_backdrop = true;
            }
            _ => {}
        }
    }
    grid
}

/// Appends the decimal representation of a value in `0..=255`.
pub fn push_dec(line: &mut Vec<u32>, value: u32) {
    if value >= 100 {
        line.push(u32::from('0') + value / 100);
        line.push(u32::from('0') + (value / 10) % 10);
    } else if value >= 10 {
        line.push(u32::from('0') + value / 10);
    }
    line.push(u32::from('0') + value % 10);
}

/// Appends a truecolor SGR payload such as `38;2;R;G;B`.
pub fn push_sgr(line: &mut Vec<u32>, base: u32, rgb: u32) {
    push_dec(line, base);
    line.extend([u32::from(';'), u32::from('2'), u32::from(';')]);
    push_dec(line, (rgb >> 16) & 0xFF);
    line.push(u32::from(';'));
    push_dec(line, (rgb >> 8) & 0xFF);
    line.push(u32::from(';'));
    push_dec(line, rgb & 0xFF);
}

/// Reports whether a code point is stripped from the end of a rendered row.
pub fn is_trailing_ws(codepoint: u32) -> bool {
    matches!(codepoint, 32 | 9 | 0xA0)
}

/// Appends one cell's grapheme text, skipping wide-cluster continuation cells.
pub fn push_cell_text(line: &mut Vec<u32>, grid: &CellGrid, cell: i32) {
    let index = index(cell);
    if grid.ch[index] == CONT {
        return;
    }
    if grid.cl[index].is_empty() {
        line.push(grid.ch[index]);
    } else {
        line.extend(grid.cl[index].chars().map(u32::from));
    }
}

/// Serializes a grid as plain characters or truecolor ANSI text.
///
/// Plain output is the raw research-fixture format. Rows are right-stripped,
/// and trailing blank rows collapse to one final newline in both formats.
pub fn cells_to_text(grid: &CellGrid, plain: bool) -> String {
    let mut output = Vec::new();
    for row in 0..grid.rows {
        let mut line = Vec::new();
        if plain {
            for column in 0..grid.cols {
                push_cell_text(
                    &mut line,
                    grid,
                    row.wrapping_mul(grid.cols).wrapping_add(column),
                );
            }
        } else {
            let mut current_fg = NO_COLOR;
            let mut current_bg = NO_COLOR;
            for column in 0..grid.cols {
                let index = index(row.wrapping_mul(grid.cols).wrapping_add(column));
                let flags = grid.flags[index];
                let foreground = if flags & CF_FG != 0 {
                    grid.fg[index]
                } else {
                    NO_COLOR
                };
                let background = if flags & CF_BG != 0 {
                    grid.bg[index]
                } else {
                    NO_COLOR
                };
                if foreground != current_fg || background != current_bg {
                    // ESC [ 0, followed by optional truecolor foreground/background.
                    line.extend([27, u32::from('['), u32::from('0')]);
                    if foreground != NO_COLOR {
                        line.push(u32::from(';'));
                        push_sgr(&mut line, 38, foreground);
                    }
                    if background != NO_COLOR {
                        line.push(u32::from(';'));
                        push_sgr(&mut line, 48, background);
                    }
                    line.push(u32::from('m'));
                    current_fg = foreground;
                    current_bg = background;
                }
                push_cell_text(
                    &mut line,
                    grid,
                    i32::try_from(index).expect("cell index exceeds i32 capacity"),
                );
            }
            line.extend([27, u32::from('['), u32::from('0'), u32::from('m')]);
        }
        while line
            .last()
            .is_some_and(|codepoint| is_trailing_ws(*codepoint))
        {
            line.pop();
        }
        output.extend(line);
        output.push(u32::from('\n'));
    }
    while output.last() == Some(&u32::from('\n')) {
        output.pop();
    }
    output.push(u32::from('\n'));
    crate::rt::str_from_chars(&output)
}

/// Cell grid for a solved frame with the kernel caret overlaid — the grid a
/// terminal client paints (`slab-tui`, the playground's TUI view).
pub fn cells_with_caret(inst: &frame::Instance, fr: &flatten::Frame) -> CellGrid {
    let mut grid = cells_from_frame(&inst.doc, fr, fr.width, fr.height);
    overlay_caret(inst, fr, &mut grid);
    grid
}

/// Surfaces the kernel caret as a cell — the one Effects-surfaced visual the
/// cell grid itself lacks.
///
/// Single-line fields use the display caret index anchored at their Text op so
/// terminal columns stay exact despite vector-font advances. Multiline fields
/// use the kernel's line-aware Effects geometry. Cells already holding a glyph
/// are left alone.
fn overlay_caret(inst: &frame::Instance, fr: &flatten::Frame, grid: &mut CellGrid) {
    let focus = inst.ds.fs.focus;
    if focus == slir::NONE {
        return;
    }
    let edit_index = dispatch::ed_ix(&inst.ds, focus);
    if edit_index < 0 {
        return;
    }
    let base = list::base(&inst.st.lists, &inst.doc, focus);
    let multiline = inst.doc.node_flags[usize::try_from(base).expect("node index exceeds usize")]
        & slir::F_MULTILINE
        != 0;
    let mut effects = dispatch::effects_new();
    dispatch::caret_effects(
        &inst.doc,
        &inst.st,
        &inst.lay,
        &inst.sc,
        &inst.ds,
        &mut effects,
    );
    if !effects.has_caret {
        return;
    }
    let mut cell = None;
    if !multiline {
        let state = &inst.ds.ed[index(edit_index)];
        let display = edit::display_str(state);
        let columns = text_columns_before(&display, edit::display_caret(state));
        for op in &fr.ops {
            if let flatten::FrameOp::Text(text) = op
                && text.node == focus
            {
                cell = Some((cell_col(text.x) + columns, cell_row(text.y_baseline - 12.0)));
                break;
            }
        }
    }
    let (col, row) = cell.unwrap_or((
        cell_col(effects.caret_x),
        cell_row(effects.caret_y + effects.caret_h / 2.0 - CH / 2.0),
    ));
    if col < 0 || row < 0 || col >= grid.cols || row >= grid.rows {
        return;
    }
    let at = index(row.wrapping_mul(grid.cols).wrapping_add(col));
    // Blank cells and box-drawing border cells may host the caret (a
    // just-2-rows-tall field puts its text in the border row and the caret
    // cell lands on a `─`); real glyphs are never clobbered.
    if grid.ch[at] == 32 || (0x2500..=0x257F).contains(&grid.ch[at]) {
        grid.ch[at] = 0x258F; // ▏ left one-eighth block
        grid.fg[at] = 0x00E8_EEF6;
        grid.flags[at] |= CF_FG;
    }
}
