//! Cell quantization edges, round-half-even behavior, the greater-than-50%
//! transparency drop rule, hairlines and borders, text placement, and the
//! plain/ANSI serialization contract.

use crate::{
    cells::{self, CellGrid},
    flatten::{self, FrameOp, OpRect, OpText},
    rt::str_eq,
    slir,
};

/// Builds a solid rectangle operation for cell-rendering tests.
#[allow(clippy::too_many_arguments)]
pub fn solid_rect(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: f64,
    bg: u32,
    bg_kind: u32,
    stroke: u32,
    stroke_kind: u32,
    dash: bool,
) -> OpRect {
    OpRect {
        node: 0,
        x,
        y,
        w,
        h,
        radius,
        bg_kind,
        bg,
        stroke_kind,
        stroke,
        stroke_w: 1.0,
        stroke_align: 0,
        stroke_sides: 15,
        dash_on: 0.0,
        dash_off: 0.0,
        has_dash: dash,
        shadow_off: 0,
        shadow_len: 0,
        opacity: 1.0,
        smooth: 0.0,
        grain_amount: 0.0,
        grain_size: 1.0,
    }
}

/// Builds a text operation for cell-rendering tests.
pub fn text_op(x: f64, y_baseline: f64, str_ref: i32, color: u32) -> OpText {
    OpText {
        node: 0,
        x,
        y_baseline,
        str_ref,
        measured_w: 0.0,
        font: -1,
        size: 14.0,
        weight: 400,
        tracking: 0.0,
        color,
        opacity: 1.0,
        color_kind: 1,
        gx: 0.0,
        gy: 0.0,
        gw: 0.0,
        gh: 0.0,
    }
}

fn text_frame(runs: &[(&str, f64)]) -> flatten::Frame {
    let mut frame = flatten::frame_new();
    for &(text, x) in runs {
        let str_ref = i32::try_from(frame.strings.len()).expect("test string count fits i32");
        frame.strings.push(text.to_owned());
        frame.ops.push(FrameOp::Text(text_op(
            x,
            12.0,
            str_ref,
            rgba(255, 255, 255, 255),
        )));
    }
    frame
}

/// Returns the code point stored at a cell coordinate.
pub fn ch_at(g: &CellGrid, c: i32, r: i32) -> u32 {
    let index = usize::try_from(r.wrapping_mul(g.cols).wrapping_add(c))
        .expect("cell coordinate must be nonnegative");
    g.ch[index]
}

/// Returns the flags stored at a cell coordinate.
pub fn flags_at(g: &CellGrid, c: i32, r: i32) -> u32 {
    let index = usize::try_from(r.wrapping_mul(g.cols).wrapping_add(c))
        .expect("cell coordinate must be nonnegative");
    g.flags[index]
}

/// Packs RGBA8 with red in the low byte and alpha in the high byte.
pub fn rgba(r: u32, green: u32, b: u32, a: u32) -> u32 {
    r | green.wrapping_shl(8) | b.wrapping_shl(16) | a.wrapping_shl(24)
}

/// Verifies Python-style round-half-even coordinate quantization.
///
/// Exact halves round to the even integer.
pub fn test_quantization_half_even() {
    assert_eq!(cells::cell_col(4.0), 0, "4/8 = 0.5 -> 0");
    assert_eq!(cells::cell_col(12.0), 2, "12/8 = 1.5 -> 2");
    assert_eq!(cells::cell_col(20.0), 2, "20/8 = 2.5 -> 2");
    assert_eq!(cells::cell_col(28.0), 4, "28/8 = 3.5 -> 4");
    assert_eq!(cells::cell_col(-4.0), 0, "-0.5 -> 0");
    assert_eq!(cells::cell_col(-12.0), -2, "-1.5 -> -2");
    assert_eq!(cells::cell_row(8.0), 0, "8/16 = 0.5 -> 0");
    assert_eq!(cells::cell_row(24.0), 2, "24/16 = 1.5 -> 2");
    assert_eq!(cells::cell_col(9.0), 1, "1.125 -> 1");
    assert_eq!(cells::cell_col(15.9), 2, "1.9875 -> 2");
}

/// Verifies grid dimensions, including minimum dimensions.
pub fn test_grid_dims() {
    let d = slir::doc_new();
    let fr = flatten::frame_new();
    let g = cells::cells_from_frame(&d, &fr, 320.0, 512.0);
    assert!(
        ((g.cols == 40) && (g.rows == 32)),
        "320x512u -> 40x32 cells"
    );
    let tiny = cells::cells_from_frame(&d, &fr, 3.0, 3.0);
    assert!(((tiny.cols == 1) && (tiny.rows == 1)), "min 1x1 grid");
    let g2 = cells::cells_from_frame(&d, &fr, 100.0, 100.0);
    assert!(
        g2.cols == 12 && g2.rows == 6,
        "100/8=12.5->12, 100/16=6.25->6"
    );
}

/// Verifies alpha blending and the unknown-terminal transparency rule.
pub fn test_alpha_compositing() {
    assert_eq!(
        cells::blend_rgb(0, 0xFF_FF_FF, 255),
        0xFF_FF_FF,
        "full alpha replaces"
    );
    assert_eq!(
        cells::blend_rgb(0, 0xFF_FF_FF, 0),
        0,
        "zero alpha keeps dst"
    );
    assert_eq!(
        cells::blend_rgb(0, 0xFF_FF_FF, 128),
        0x80_80_80,
        "half blend"
    );
    let d = slir::doc_new();
    let mut fr = flatten::frame_new();
    // Paint an opaque red base, then a 100/255 green wash over its left half.
    fr.ops.push(FrameOp::Rect(solid_rect(
        0.0,
        0.0,
        64.0,
        32.0,
        0.0,
        rgba(255, 0, 0, 255),
        1,
        0,
        0,
        false,
    )));
    fr.ops.push(FrameOp::Rect(solid_rect(
        0.0,
        0.0,
        32.0,
        32.0,
        0.0,
        rgba(0, 255, 0, 100),
        1,
        0,
        0,
        false,
    )));
    let g = cells::cells_from_frame(&d, &fr, 64.0, 32.0);
    assert!(flags_at(&g, 1, 1) & cells::CF_BG != 0, "base painted");
    let left = usize::try_from(g.cols + 1).expect("positive cell index");
    assert_eq!(
        g.bg[left],
        cells::blend_rgb(0xFF_00_00, 0x00_FF_00, 100),
        "wash composites over base"
    );
    let right = usize::try_from(g.cols + 5).expect("positive cell index");
    assert_eq!(g.bg[right], 0xFF_00_00, "right half stays red");
    let mut fr2 = flatten::frame_new();
    // A translucent wash without an underlying background is dropped because
    // the terminal's current background is unknown.
    fr2.ops.push(FrameOp::Rect(solid_rect(
        0.0,
        0.0,
        32.0,
        32.0,
        0.0,
        rgba(0, 255, 0, 100),
        1,
        0,
        0,
        false,
    )));
    let g2 = cells::cells_from_frame(&d, &fr2, 64.0, 32.0);
    assert!(
        flags_at(&g2, 1, 1) & cells::CF_BG == 0,
        "wash over nothing drops"
    );
}

/// Verifies linear and radial gradient sampling.
pub fn test_gradient_sampling() {
    let mut d = slir::doc_new();
    d.grad_kind.push(0);
    // A 180-degree linear gradient runs toward the bottom.
    d.grad_angle.push(180.0);
    d.grad_stop_off.push(0);
    d.grad_stop_len.push(2);
    d.grad_stop_pos.push(0.0);
    d.grad_stop_pos.push(1.0);
    d.grad_stop_rgba.push(rgba(0, 0, 0, 255));
    d.grad_stop_rgba.push(rgba(200, 100, 50, 255));
    // The top and bottom edges select the endpoint stops; the middle samples
    // their midpoint.
    let top = cells::paint_rgba_at(&d, 2, 0, 50.0, 0.0, 0.0, 0.0, 100.0, 100.0);
    let bot = cells::paint_rgba_at(&d, 2, 0, 50.0, 100.0, 0.0, 0.0, 100.0, 100.0);
    let mid = cells::paint_rgba_at(&d, 2, 0, 50.0, 50.0, 0.0, 0.0, 100.0, 100.0);
    assert_eq!(top, rgba(0, 0, 0, 255), "linear top = first stop");
    assert_eq!(bot, rgba(200, 100, 50, 255), "linear bottom = last stop");
    assert_eq!(mid, rgba(100, 50, 25, 255), "linear middle = srgb midpoint");
    assert_eq!(
        cells::paint_rgba_at(&d, 0, 0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0),
        0,
        "paint none"
    );
    // A radial gradient samples its first stop at the center and its last stop
    // at a corner.
    d.grad_kind.push(1);
    d.grad_angle.push(0.0);
    d.grad_stop_off.push(0);
    d.grad_stop_len.push(2);
    let ctr = cells::paint_rgba_at(&d, 2, 1, 50.0, 50.0, 0.0, 0.0, 100.0, 100.0);
    let cor = cells::paint_rgba_at(&d, 2, 1, 0.0, 0.0, 0.0, 0.0, 100.0, 100.0);
    assert_eq!(ctr, rgba(0, 0, 0, 255), "radial center = first stop");
    assert_eq!(cor, rgba(200, 100, 50, 255), "radial corner = last stop");
}

/// Verifies hairline placement and rounded box borders.
pub fn test_hairlines_and_borders() {
    let d = slir::doc_new();
    let mut fr = flatten::frame_new();
    // Height 8 <= 9.6 and width > 8 produces a horizontal hairline at
    // cell_row(y + h / 2 - 8).
    fr.ops.push(FrameOp::Rect(solid_rect(
        0.0,
        16.0,
        32.0,
        8.0,
        0.0,
        rgba(255, 255, 255, 255),
        1,
        0,
        0,
        false,
    )));
    // Width 7 <= 7.2 and height > 16 produces a dashed vertical hairline.
    fr.ops.push(FrameOp::Rect(solid_rect(
        64.0,
        0.0,
        7.0,
        48.0,
        0.0,
        rgba(255, 255, 255, 255),
        1,
        0,
        0,
        true,
    )));
    // A stroked box with radius >= 4 uses rounded corners.
    fr.ops.push(FrameOp::Rect(solid_rect(
        0.0,
        48.0,
        64.0,
        48.0,
        6.0,
        0,
        0,
        rgba(0, 0, 255, 255),
        1,
        false,
    )));
    let g = cells::cells_from_frame(&d, &fr, 96.0, 96.0);
    // cell_row(16 + 4 - 8) = round_half_even(0.75) = 1.
    assert!(
        ch_at(&g, 0, 1) == 0x2500 && ch_at(&g, 3, 1) == 0x2500,
        "horizontal hairline"
    );
    // cell_col(64 + 3.5 - 4) = round_half_even(7.9375) = 8.
    assert!(
        ch_at(&g, 8, 0) == 0x254E && ch_at(&g, 8, 2) == 0x254E,
        "dashed vertical hairline"
    );
    // Box bounds in cells are c0 = 0, r0 = 3, c1 = 8, r1 = 6.
    assert_eq!(ch_at(&g, 0, 3), 0x256D, "round tl");
    assert_eq!(ch_at(&g, 7, 3), 0x256E, "round tr");
    assert_eq!(ch_at(&g, 0, 5), 0x2570, "round bl");
    assert_eq!(ch_at(&g, 7, 5), 0x256F, "round br");
    assert_eq!(ch_at(&g, 3, 3), 0x2500, "top border");
    assert_eq!(ch_at(&g, 0, 4), 0x2502, "left border");
}

/// Verifies the shallow-outline degradation: one- and two-row stroked boxes
/// become three-sided wells whose horizontal border avoids the text row,
/// while an empty two-row box keeps its closed border.
pub fn test_shallow_outline_wells() {
    let doc = slir::doc_new();
    let stroke = rgba(0, 0, 255, 255);
    // Rows 0..2 (y 0, h 28); radius >= 4 selects rounded corners.
    let shallow_box = || solid_rect(0.0, 0.0, 64.0, 28.0, 6.0, 0, 0, stroke, 1, false);

    // Text on the top row: side verticals up top, corner-capped bottom.
    let mut fr = flatten::frame_new();
    fr.strings.push("Hi".to_owned());
    fr.ops.push(FrameOp::Rect(shallow_box()));
    fr.ops.push(FrameOp::Text(text_op(16.0, 12.0, 0, stroke)));
    let g = cells::cells_from_frame(&doc, &fr, 96.0, 96.0);
    assert!(
        ch_at(&g, 0, 0) == 0x2502 && ch_at(&g, 7, 0) == 0x2502,
        "verticals flank the text row"
    );
    assert!(
        ch_at(&g, 0, 1) == 0x2570 && ch_at(&g, 7, 1) == 0x256F && ch_at(&g, 3, 1) == 0x2500,
        "bottom row keeps the capped border"
    );
    assert_eq!(ch_at(&g, 2, 0), 72, "text survives inside the well");

    // Text on the bottom row mirrors the well: cap on top, verticals below.
    let mut fr = flatten::frame_new();
    fr.strings.push("Hi".to_owned());
    fr.ops.push(FrameOp::Rect(shallow_box()));
    fr.ops.push(FrameOp::Text(text_op(16.0, 28.0, 0, stroke)));
    let g = cells::cells_from_frame(&doc, &fr, 96.0, 96.0);
    assert!(
        ch_at(&g, 0, 0) == 0x256D && ch_at(&g, 7, 0) == 0x256E && ch_at(&g, 3, 0) == 0x2500,
        "top row keeps the capped border"
    );
    assert!(
        ch_at(&g, 0, 1) == 0x2502 && ch_at(&g, 7, 1) == 0x2502,
        "verticals flank the bottom text row"
    );

    // No text on either row: the two-row box stays closed.
    let mut fr = flatten::frame_new();
    fr.ops.push(FrameOp::Rect(shallow_box()));
    let g = cells::cells_from_frame(&doc, &fr, 96.0, 96.0);
    assert!(
        ch_at(&g, 0, 0) == 0x256D && ch_at(&g, 7, 0) == 0x256E && ch_at(&g, 3, 0) == 0x2500,
        "empty shallow box keeps its top border"
    );
    assert!(
        ch_at(&g, 0, 1) == 0x2570 && ch_at(&g, 7, 1) == 0x256F && ch_at(&g, 3, 1) == 0x2500,
        "empty shallow box keeps its bottom border"
    );

    // A one-row outline (h 18 -> a single cell row, too tall for a hairline)
    // draws side verticals instead of disappearing.
    let mut fr = flatten::frame_new();
    fr.ops.push(FrameOp::Rect(solid_rect(
        0.0, 0.0, 64.0, 18.0, 6.0, 0, 0, stroke, 1, false,
    )));
    let g = cells::cells_from_frame(&doc, &fr, 96.0, 96.0);
    assert!(
        ch_at(&g, 0, 0) == 0x2502 && ch_at(&g, 7, 0) == 0x2502,
        "one-row outline keeps side verticals"
    );
    assert_eq!(ch_at(&g, 3, 0), 32, "one-row outline has no horizontal run");
}

/// Verifies text placement, background fills, and translucent glyphs.
pub fn test_text_and_fill_bg() {
    let doc = slir::doc_new();
    let mut frame = flatten::frame_new();
    frame.strings.push("Hi".to_owned());
    frame
        .ops
        .push(FrameOp::Text(text_op(8.0, 28.0, 0, rgba(1, 2, 3, 255))));
    let grid = cells::cells_from_frame(&doc, &frame, 64.0, 48.0);

    // cell_col(8) = 1 and cell_row(28 - 12) = 1.
    assert!(
        ch_at(&grid, 1, 1) == 72 && ch_at(&grid, 2, 1) == 105,
        "text placed at baseline cell"
    );
    let text_cell = usize::try_from(grid.cols + 1).expect("positive cell index");
    assert!(
        flags_at(&grid, 1, 1) & cells::CF_FG != 0 && grid.fg[text_cell] == 0x01_02_03,
        "text fg set"
    );

    // A later background fill wipes the glyph and clears its foreground.
    let mut filled_frame = flatten::frame_new();
    filled_frame.strings.push("Hi".to_owned());
    filled_frame
        .ops
        .push(FrameOp::Text(text_op(8.0, 28.0, 0, rgba(1, 2, 3, 255))));
    filled_frame.ops.push(FrameOp::Rect(solid_rect(
        0.0,
        0.0,
        64.0,
        48.0,
        0.0,
        rgba(0, 0, 0, 255),
        1,
        0,
        0,
        false,
    )));
    let filled_grid = cells::cells_from_frame(&doc, &filled_frame, 64.0, 48.0);
    assert_eq!(ch_at(&filled_grid, 1, 1), 32, "fill_bg blanks the cell");
    assert!(
        flags_at(&filled_grid, 1, 1) & cells::CF_FG == 0,
        "fill_bg clears fg"
    );
    assert!(
        flags_at(&filled_grid, 1, 1) & cells::CF_BG != 0,
        "fill_bg sets bg"
    );

    // Half-transparent text is skipped over an unknown terminal background,
    // but composited over a known cell background.
    let mut transparent_frame = flatten::frame_new();
    transparent_frame.strings.push("x".to_owned());
    transparent_frame
        .ops
        .push(FrameOp::Text(text_op(0.0, 12.0, 0, rgba(1, 2, 3, 50))));
    let transparent_grid = cells::cells_from_frame(&doc, &transparent_frame, 32.0, 16.0);
    assert_eq!(
        ch_at(&transparent_grid, 0, 0),
        32,
        "translucent glyph over nothing skipped"
    );

    let mut composited_frame = flatten::frame_new();
    composited_frame.strings.push("x".to_owned());
    composited_frame.ops.push(FrameOp::Rect(solid_rect(
        0.0,
        0.0,
        32.0,
        16.0,
        0.0,
        rgba(255, 255, 255, 255),
        1,
        0,
        0,
        false,
    )));
    composited_frame
        .ops
        .push(FrameOp::Text(text_op(0.0, 12.0, 0, rgba(1, 2, 3, 50))));
    let composited_grid = cells::cells_from_frame(&doc, &composited_frame, 32.0, 16.0);
    assert_eq!(
        ch_at(&composited_grid, 0, 0),
        120,
        "translucent glyph over bg placed"
    );
    assert_eq!(
        composited_grid.fg[0],
        cells::blend_rgb(0xFF_FF_FF, 0x01_02_03, 50),
        "text fg composited"
    );
}

/// Verifies wide, combining, and presentation-modified grapheme clusters.
pub fn test_wide_grapheme_clusters() {
    assert_eq!(cells::text_columns_before("中", 1), 2, "CJK width");
    assert_eq!(
        cells::text_columns_before("e\u{301}", 2),
        1,
        "combining cluster width"
    );
    assert_eq!(
        cells::text_columns_before("👩\u{200D}👩\u{200D}👧", 5),
        2,
        "ZWJ family width"
    );
    assert_eq!(
        cells::text_columns_before("👍\u{FE0E}", 2),
        1,
        "VS15 text presentation width"
    );
    assert_eq!(
        cells::text_columns_before("ae\u{301}b", 2),
        1,
        "partial cluster is excluded"
    );
    assert_eq!(
        cells::text_columns_before("ae\u{301}b", 3),
        2,
        "complete cluster is included"
    );
    assert_eq!(
        cells::text_columns_before("ae\u{301}b", -1),
        0,
        "negative prefix clamps to zero"
    );
    assert_eq!(
        cells::text_columns_before("ae\u{301}b", 99),
        3,
        "oversized prefix clamps to text length"
    );
    let doc = slir::doc_new();
    let mut frame = flatten::frame_new();
    for text in ["中", "👍", "é", "👍︎", "🇺🇸"] {
        frame.strings.push(text.to_owned());
    }
    for (str_ref, baseline) in [(0, 12.0), (1, 28.0), (2, 44.0), (3, 60.0), (4, 76.0)] {
        frame.ops.push(FrameOp::Text(text_op(
            0.0,
            baseline,
            str_ref,
            rgba(255, 255, 255, 255),
        )));
    }

    let grid = cells::cells_from_frame(&doc, &frame, 48.0, 96.0);
    assert!(
        ch_at(&grid, 0, 0) == 0x4E2D && ch_at(&grid, 1, 0) == cells::CONT,
        "CJK uses two cells"
    );
    assert!(
        ch_at(&grid, 0, 1) == 0x1F44D && ch_at(&grid, 1, 1) == cells::CONT,
        "emoji uses two cells"
    );
    let combining = usize::try_from(2_i32.wrapping_mul(grid.cols)).expect("positive cell index");
    assert!(
        ch_at(&grid, 0, 2) == 101 && str_eq(&grid.cl[combining], "é") && ch_at(&grid, 1, 2) == 32,
        "combining cluster uses one cell and retains full text"
    );
    let text_presentation =
        usize::try_from(3_i32.wrapping_mul(grid.cols)).expect("positive cell index");
    assert!(
        ch_at(&grid, 0, 3) == 0x1F44D
            && str_eq(&grid.cl[text_presentation], "👍︎")
            && ch_at(&grid, 1, 3) == 32,
        "VS15 forces narrow text presentation"
    );
    let flag = usize::try_from(4_i32.wrapping_mul(grid.cols)).expect("positive cell index");
    assert!(
        ch_at(&grid, 0, 4) == 0x1F1FA
            && str_eq(&grid.cl[flag], "🇺🇸")
            && ch_at(&grid, 1, 4) == cells::CONT,
        "regional-indicator pair uses two cells"
    );
    assert!(
        str_eq(&cells::cells_to_text(&grid, true), "中\n👍\né\n👍︎\n🇺🇸\n"),
        "serialization emits clusters without continuation sentinels"
    );

    let mut edge = flatten::frame_new();
    edge.strings.push("中A".to_owned());
    edge.ops.push(FrameOp::Text(text_op(
        0.0,
        12.0,
        0,
        rgba(255, 255, 255, 255),
    )));
    let clipped = cells::cells_from_frame(&doc, &edge, 8.0, 16.0);
    assert_eq!(
        ch_at(&clipped, 0, 0),
        32,
        "wide cluster at final column is dropped with its run"
    );
}

/// Verifies that later paints never leave an orphaned half of a wide cluster.
pub fn test_wide_overwrite_cleanup() {
    let doc = slir::doc_new();

    let narrow_over_leading =
        cells::cells_from_frame(&doc, &text_frame(&[("中", 0.0), ("a", 0.0)]), 32.0, 16.0);
    assert_eq!(
        &narrow_over_leading.ch[..2],
        &[u32::from('a'), 32],
        "narrow glyph over the leading cell clears the continuation"
    );

    let narrow_over_continuation =
        cells::cells_from_frame(&doc, &text_frame(&[("中", 0.0), ("b", 8.0)]), 32.0, 16.0);
    assert_eq!(
        &narrow_over_continuation.ch[..2],
        &[32, u32::from('b')],
        "narrow glyph over the continuation clears the leading cell"
    );

    let mut background = cells::cells_from_frame(&doc, &text_frame(&[("中", 0.0)]), 32.0, 16.0);
    cells::bg_composite(&mut background, 1, 0, 0x12_34_56, 128);
    assert_eq!(
        &background.ch[..2],
        &[32, 32],
        "majority-opaque background clears both cluster cells"
    );

    let shifted_wide =
        cells::cells_from_frame(&doc, &text_frame(&[("中", 0.0), ("界", 8.0)]), 32.0, 16.0);
    assert_eq!(
        &shifted_wide.ch[..4],
        &[32, u32::from('界'), cells::CONT, 32],
        "new wide glyph replaces the touched pair without stale halves"
    );
}

/// Verifies plain-text and ANSI serialization.
pub fn test_serialize() {
    let doc = slir::doc_new();
    let mut frame = flatten::frame_new();
    frame.strings.push("A".to_owned());
    frame
        .ops
        .push(FrameOp::Text(text_op(0.0, 12.0, 0, rgba(255, 0, 0, 255))));

    let grid = cells::cells_from_frame(&doc, &frame, 32.0, 32.0);
    let plain = cells::cells_to_text(&grid, true);
    assert!(
        str_eq(&plain, "A\n"),
        "plain: rows right-stripped, trailing blank rows collapse"
    );

    // On a 1x1 grid, ANSI output emits one SGR sequence at the color change
    // and appends a reset at the row end.
    let one_cell = cells::cells_from_frame(&doc, &frame, 8.0, 16.0);
    let ansi = cells::cells_to_text(&one_cell, false);
    assert!(
        str_eq(&ansi, "\u{1B}[0;38;2;255;0;0mA\u{1B}[0m\n"),
        "ansi: sgr on change, reset at row end"
    );
}
