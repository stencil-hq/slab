//! CPU rasterizer: kernel FrameOps -> pixels via tiny-skia (research png.py
//! port). Geometry/paints/strokes/paths/gradients render through tiny-skia;
//! glyph outlines come from the vendored TTFs via ttf-parser (positions and
//! advances stay kernel-owned: SLIR FONT cmap + advances); shadows, layer
//! blur, and backdrop use the research 3-pass separable box blur with the
//! same radius mapping (`rad = max(1, blur/2)`). PNG and APNG encode through
//! the `png` crate (`Encoder::set_animated`).
//!
//! Modern FX land as direct pixel work where tiny-skia has no primitive:
//! conic sweeps, grain speckle, mask fades, banded progressive backdrop
//! blur, and the tilt homography warp blend premultiplied RGBA in place;
//! squircle outlines come from the kernel's canonical constructor.

// The shadow/backdrop helpers are geometry ports; keep their research
// signatures rather than inventing structs for one call site each.
#![allow(clippy::too_many_arguments)]

use crate::render::RegisteredFont;
use slab_fonts;
use slab_kernel::{
    flatten::{Frame, FrameOp, OpRect, OpText},
    graphemes,
};
use slab_slir::Slir;
use tiny_skia::{
    Color, FillRule, GradientStop, IntSize, LinearGradient, Mask, Paint, Path, PathBuilder, Pixmap,
    PixmapPaint, Point, RadialGradient, SpreadMode, Stroke, StrokeDash, Transform,
};

const KAPPA: f64 = 0.552_284_749_830_793_4;

fn rgba8(v: u32, opacity: f64) -> Color {
    let [r, g, b, a] = v.to_le_bytes();
    Color::from_rgba8(
        r,
        g,
        b,
        (a as f64 * opacity).round().clamp(0.0, 255.0) as u8,
    )
}

fn base_paint() -> Paint<'static> {
    Paint {
        anti_alias: true,
        ..Paint::default()
    }
}

/// Rounded-rect path; `r` clamps to the half extent.
fn rrect_path(x0: f64, y0: f64, x1: f64, y1: f64, r: f64) -> Option<Path> {
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut pb = PathBuilder::new();
    if r <= 0.0 {
        pb.push_rect(tiny_skia::Rect::from_ltrb(
            x0 as f32, y0 as f32, x1 as f32, y1 as f32,
        )?);
        return pb.finish();
    }
    let k = r * KAPPA;
    let (x0f, y0f, x1f, y1f, rf, kf) = (x0, y0, x1, y1, r, k);
    pb.move_to((x0f + rf) as f32, y0f as f32);
    pb.line_to((x1f - rf) as f32, y0f as f32);
    pb.cubic_to(
        (x1f - rf + kf) as f32,
        y0f as f32,
        x1f as f32,
        (y0f + rf - kf) as f32,
        x1f as f32,
        (y0f + rf) as f32,
    );
    pb.line_to(x1f as f32, (y1f - rf) as f32);
    pb.cubic_to(
        x1f as f32,
        (y1f - rf + kf) as f32,
        (x1f - rf + kf) as f32,
        y1f as f32,
        (x1f - rf) as f32,
        y1f as f32,
    );
    pb.line_to((x0f + rf) as f32, y1f as f32);
    pb.cubic_to(
        (x0f + rf - kf) as f32,
        y1f as f32,
        x0f as f32,
        (y1f - rf + kf) as f32,
        x0f as f32,
        (y1f - rf) as f32,
    );
    pb.line_to(x0f as f32, (y0f + rf) as f32);
    pb.cubic_to(
        x0f as f32,
        (y0f + rf - kf) as f32,
        (x0f + rf - kf) as f32,
        y0f as f32,
        (x0f + rf) as f32,
        y0f as f32,
    );
    pb.close();
    pb.finish()
}

/// Rounded-rect outline, upgraded to the kernel's canonical squircle when
/// both `r` and `smooth` are positive (SPEC §7 `smooth`; document verb
/// encoding: 0 move, 1 line, 2 cubic, 3 quad, 4 close).
fn shape_path(x0: f64, y0: f64, x1: f64, y1: f64, r: f64, smooth: f64) -> Option<Path> {
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    if r <= 0.0 || smooth <= 0.0 {
        return rrect_path(x0, y0, x1, y1, r);
    }
    let (verbs, coords) = slab_kernel::squircle::squircle_path(w, h, r, smooth);
    let mut pb = PathBuilder::new();
    let mut k = 0usize;
    for &v in &verbs {
        match v {
            0 => {
                pb.move_to((x0 + coords[k]) as f32, (y0 + coords[k + 1]) as f32);
                k += 2;
            }
            1 => {
                pb.line_to((x0 + coords[k]) as f32, (y0 + coords[k + 1]) as f32);
                k += 2;
            }
            2 => {
                pb.cubic_to(
                    (x0 + coords[k]) as f32,
                    (y0 + coords[k + 1]) as f32,
                    (x0 + coords[k + 2]) as f32,
                    (y0 + coords[k + 3]) as f32,
                    (x0 + coords[k + 4]) as f32,
                    (y0 + coords[k + 5]) as f32,
                );
                k += 6;
            }
            3 => {
                pb.quad_to(
                    (x0 + coords[k]) as f32,
                    (y0 + coords[k + 1]) as f32,
                    (x0 + coords[k + 2]) as f32,
                    (y0 + coords[k + 3]) as f32,
                );
                k += 4;
            }
            _ => pb.close(),
        }
    }
    pb.finish()
}

/// Draws a rect stroke's geometry (per-side fills, dashed outlines, or the
/// full rrect/squircle ring) with one paint — shared by the direct path and
/// the conic coverage capture in `draw_rect`.
fn rect_stroke_geo(
    pix: &mut Pixmap,
    clip: Option<&Mask>,
    paint: &Paint,
    r: &OpRect,
    s: f64,
    sx0: f64,
    sy0: f64,
    sx1: f64,
    sy1: f64,
    hw: f64,
    off: f64,
    dash: Option<StrokeDash>,
) {
    if r.stroke_sides != 15 {
        let w_px = r.stroke_w * s;
        // t, b, l, r (research seg order)
        let segs: [(u32, (f64, f64, f64, f64)); 4] = [
            (1, (sx0, sy0 - hw, sx1, sy0 + hw)),
            (4, (sx0, sy1 - hw, sx1, sy1 + hw)),
            (8, (sx0 - hw, sy0, sx0 + hw, sy1)),
            (2, (sx1 - hw, sy0, sx1 + hw, sy1)),
        ];
        for &(bit, (a, b, c, d)) in &segs {
            if r.stroke_sides & bit == 0 {
                continue;
            }
            if r.has_dash {
                let mut pb = PathBuilder::new();
                if bit == 1 || bit == 4 {
                    pb.move_to(a as f32, ((b + d) / 2.0) as f32);
                    pb.line_to(c as f32, ((b + d) / 2.0) as f32);
                } else {
                    pb.move_to(((a + c) / 2.0) as f32, b as f32);
                    pb.line_to(((a + c) / 2.0) as f32, d as f32);
                }
                if let Some(path) = pb.finish() {
                    let stroke = Stroke {
                        width: w_px as f32,
                        dash: dash.clone(),
                        ..Stroke::default()
                    };
                    pix.stroke_path(&path, paint, &stroke, Transform::identity(), clip);
                }
            } else if let Some(path) = rrect_path(a, b, c, d, 0.0) {
                pix.fill_path(&path, paint, FillRule::Winding, Transform::identity(), clip);
            }
        }
    } else if r.has_dash {
        let mut pb = PathBuilder::new();
        pb.move_to(sx0 as f32, sy0 as f32);
        pb.line_to(sx1 as f32, sy0 as f32);
        pb.line_to(sx1 as f32, sy1 as f32);
        pb.line_to(sx0 as f32, sy1 as f32);
        pb.close();
        if let Some(path) = pb.finish() {
            let stroke = Stroke {
                width: (r.stroke_w * s) as f32,
                dash,
                ..Stroke::default()
            };
            pix.stroke_path(&path, paint, &stroke, Transform::identity(), clip);
        }
    } else {
        let r_adj = (r.radius * s - off).max(0.0);
        if let Some(path) = shape_path(sx0, sy0, sx1, sy1, r_adj, r.smooth) {
            let stroke = Stroke {
                width: (r.stroke_w * s) as f32,
                ..Stroke::default()
            };
            pix.stroke_path(&path, paint, &stroke, Transform::identity(), clip);
        }
    }
}

/// One separable box-blur pass (H then V), zero padding outside — the
/// research `_box_blur` verbatim, per channel.
fn box_blur_pass(data: &mut [u8], w: usize, h: usize, rad: usize, tmp: &mut [u8]) {
    let div = (2 * rad + 1) as u32;
    // horizontal
    for y in 0..h {
        let row = y * w;
        let mut sum: u32 = 0;
        for x in 0..rad.min(w) {
            sum += data[row + x] as u32;
        }
        for x in 0..w {
            if x + rad < w {
                sum += data[row + x + rad] as u32;
            }
            tmp[row + x] = (sum / div) as u8;
            if x >= rad {
                sum -= data[row + x - rad] as u32;
            }
        }
    }
    // vertical
    for x in 0..w {
        let mut sum: u32 = 0;
        for y in 0..rad.min(h) {
            sum += tmp[y * w + x] as u32;
        }
        for y in 0..h {
            if y + rad < h {
                sum += tmp[(y + rad) * w + x] as u32;
            }
            data[y * w + x] = (sum / div) as u8;
            if y >= rad {
                sum -= tmp[(y - rad) * w + x] as u32;
            }
        }
    }
}

/// Research blur: 3 box-blur passes over premultiplied RGBA.
fn blur_rgba(data: &mut [u8], w: usize, h: usize, rad: usize) {
    if w == 0 || h == 0 || rad == 0 {
        return;
    }
    let mut chan = vec![0u8; w * h];
    let mut tmp = vec![0u8; w * h];
    for c in 0..4 {
        for (i, px) in chan.iter_mut().enumerate() {
            *px = data[i * 4 + c];
        }
        for _ in 0..3 {
            box_blur_pass(&mut chan, w, h, rad, &mut tmp);
        }
        for (i, px) in chan.iter().enumerate() {
            data[i * 4 + c] = *px;
        }
    }
}

fn blur_rad(blur: f64) -> usize {
    ((blur / 2.0) as i64).max(1) as usize
}

/// Gradient stop ramp — kernel `grad_sample` semantics: clamped ends,
/// per-segment sRGB lerp (`slab_kernel::cells::rgba_lerp`).
fn ramp(stops: &[(f64, u32)], t: f64) -> u32 {
    let Some(&(first_pos, first)) = stops.first() else {
        return 0;
    };
    if t <= first_pos {
        return first;
    }
    for i in 1..stops.len() {
        let (p1, c1) = stops[i];
        if t <= p1 {
            let (p0, c0) = stops[i - 1];
            let local = if p1 > p0 { (t - p0) / (p1 - p0) } else { 0.0 };
            return slab_kernel::cells::rgba_lerp(c0, c1, local);
        }
    }
    stops[stops.len() - 1].1
}

/// pcg2d hash (Jarzynski–Olano), seedless — the contract §6.2 speckle hash;
/// all arithmetic wraps in u32.
fn pcg2d(ix: u32, iy: u32) -> u32 {
    let mut vx = ix.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    let mut vy = iy.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    vx = vx.wrapping_add(vy.wrapping_mul(1_664_525));
    vy = vy.wrapping_add(vx.wrapping_mul(1_664_525));
    vx ^= vx >> 16;
    vy ^= vy >> 16;
    vx = vx.wrapping_add(vy.wrapping_mul(1_664_525));
    // the reference's trailing vy mixes are dead — the hash is vx
    vx ^= vx >> 16;
    vx
}

/// Heckbert unit-square→quad homography (row-major 3×3): maps the corners
/// (0,0),(1,0),(1,1),(0,1) onto `q` in order. NaN-filled when degenerate.
fn square_to_quad(q: [(f64, f64); 4]) -> [f64; 9] {
    let px = q[0].0 - q[1].0 + q[2].0 - q[3].0;
    let py = q[0].1 - q[1].1 + q[2].1 - q[3].1;
    if px == 0.0 && py == 0.0 {
        return [
            q[1].0 - q[0].0,
            q[2].0 - q[1].0,
            q[0].0,
            q[1].1 - q[0].1,
            q[2].1 - q[1].1,
            q[0].1,
            0.0,
            0.0,
            1.0,
        ];
    }
    let (dx1, dy1) = (q[1].0 - q[2].0, q[1].1 - q[2].1);
    let (dx2, dy2) = (q[3].0 - q[2].0, q[3].1 - q[2].1);
    let den = dx1 * dy2 - dx2 * dy1;
    if den == 0.0 {
        return [f64::NAN; 9];
    }
    let g = (px * dy2 - py * dx2) / den;
    let h = (dx1 * py - dy1 * px) / den;
    [
        q[1].0 - q[0].0 + g * q[1].0,
        q[3].0 - q[0].0 + h * q[3].0,
        q[0].0,
        q[1].1 - q[0].1 + g * q[1].1,
        q[3].1 - q[0].1 + h * q[3].1,
        q[0].1,
        g,
        h,
        1.0,
    ]
}

/// 3×3 adjugate — the inverse up to scale, which is all a homography needs
/// (no division keeps near-degenerate quads deterministic).
fn adjugate(m: [f64; 9]) -> [f64; 9] {
    [
        m[4] * m[8] - m[5] * m[7],
        m[2] * m[7] - m[1] * m[8],
        m[1] * m[5] - m[2] * m[4],
        m[5] * m[6] - m[3] * m[8],
        m[0] * m[8] - m[2] * m[6],
        m[2] * m[3] - m[0] * m[5],
        m[3] * m[7] - m[4] * m[6],
        m[1] * m[6] - m[0] * m[7],
        m[0] * m[4] - m[1] * m[3],
    ]
}

fn mat3_mul(a: [f64; 9], b: [f64; 9]) -> [f64; 9] {
    let mut out = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            out[r * 3 + c] = a[r * 3] * b[c] + a[r * 3 + 1] * b[3 + c] + a[r * 3 + 2] * b[6 + c];
        }
    }
    out
}

/// Homography sending quad `from` onto quad `to` corner-for-corner, built as
/// square→to ∘ (square→from)⁻¹; `None` when either quad is degenerate. Used
/// to inverse-map tilt output pixels back into the flattened layer.
fn quad_to_quad(from: [(f64, f64); 4], to: [(f64, f64); 4]) -> Option<[f64; 9]> {
    let a = square_to_quad(from);
    let b = square_to_quad(to);
    if a.iter().chain(b.iter()).any(|v| !v.is_finite()) {
        return None;
    }
    let adj = adjugate(a);
    let det = a[0] * adj[0] + a[1] * adj[3] + a[2] * adj[6];
    if det == 0.0 {
        return None;
    }
    Some(mat3_mul(b, adj))
}

/// Bilinear tap on premultiplied RGBA at continuous coords (pixel centers at
/// +0.5); anything outside the buffer samples transparent.
fn sample_bilinear(data: &[u8], w: i64, h: i64, x: f64, y: f64) -> [f64; 4] {
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (bx, by) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - bx, fy - by);
    let (bx, by) = (bx as i64, by as i64);
    let mut out = [0.0f64; 4];
    let mut tap = |xi: i64, yi: i64, wgt: f64| {
        if wgt <= 0.0 || xi < 0 || yi < 0 || xi >= w || yi >= h {
            return;
        }
        let i = ((yi * w + xi) * 4) as usize;
        for (o, px) in out.iter_mut().zip(&data[i..i + 4]) {
            *o += f64::from(*px) * wgt;
        }
    };
    tap(bx, by, (1.0 - tx) * (1.0 - ty));
    tap(bx + 1, by, tx * (1.0 - ty));
    tap(bx, by + 1, (1.0 - tx) * ty);
    tap(bx + 1, by + 1, tx * ty);
    out
}

/// Alpha-fade mask carried by a group layer (contract §6.3); box in device px.
struct GroupMask {
    kind: u32,
    handle: u32,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

enum LayerKind {
    Base,
    Group {
        opacity: f64,
        blur: f64,
        mask: Option<GroupMask>,
    },
    Rotate {
        deg: f64,
        cx: f64,
        cy: f64,
    },
    Scale {
        cx: f64,
        cy: f64,
        sx: f64,
        sy: f64,
    },
    Tilt {
        cx: f64,
        cy: f64,
        rx: f64,
        ry: f64,
        depth: f64,
    },
}

fn scale_layer_kind(scale: &slab_kernel::flatten::OpScale, device_scale: f64) -> LayerKind {
    LayerKind::Scale {
        cx: scale.cx * device_scale,
        cy: scale.cy * device_scale,
        sx: scale.sx,
        sy: scale.sy,
    }
}

struct Layer {
    pix: Pixmap,
    clips: Vec<Mask>,
    kind: LayerKind,
}

/// Composites a tilted layer into its parent (contract §6.5): project the
/// full-canvas quad, then warp by the inverse homography with bilinear
/// sampling so the subtree flattens into one perspective plane.
fn tilt_composite(
    parent: &mut Layer,
    src: &Pixmap,
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    depth: f64,
) {
    let (w, h) = (f64::from(src.width()), f64::from(src.height()));
    if !(depth.is_finite() && depth > 0.0) {
        // degenerate depth never projects; composite untransformed
        parent.pix.draw_pixmap(
            0,
            0,
            src.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            parent.clips.last(),
        );
        return;
    }
    let (sry, cry) = ry.to_radians().sin_cos();
    let (srx, crx) = rx.to_radians().sin_cos();
    // §6.5: p = (x−cx, y−cy, 0), rotY then rotX, then perspective with the
    // near-plane clamp zc ≤ 0.95·depth so degenerate tilts stay finite
    let project = |x: f64, y: f64| -> (f64, f64) {
        let (px, py) = (x - cx, y - cy);
        let x1 = px * cry;
        let z1 = -px * sry;
        let y2 = py * crx - z1 * srx;
        let z2 = py * srx + z1 * crx;
        let zc = z2.min(0.95 * depth);
        let k = depth / (depth - zc);
        (cx + x1 * k, cy + y2 * k)
    };
    let src_q = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
    let dst_q = [
        project(0.0, 0.0),
        project(w, 0.0),
        project(w, h),
        project(0.0, h),
    ];
    // edge-on plane (collapsed quad): nothing visible
    let Some(hm) = quad_to_quad(dst_q, src_q) else {
        return;
    };
    let (pw, ph) = (parent.pix.width() as i64, parent.pix.height() as i64);
    let (sw, sh) = (src.width() as i64, src.height() as i64);
    let bx0 = dst_q.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let by0 = dst_q.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let bx1 = dst_q.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let by1 = dst_q.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    let ix0 = (bx0.floor() as i64).max(0);
    let iy0 = (by0.floor() as i64).max(0);
    let ix1 = (bx1.ceil() as i64).min(pw);
    let iy1 = (by1.ceil() as i64).min(ph);
    if ix0 >= ix1 || iy0 >= iy1 {
        return;
    }
    // w's sign at the quad centroid anchors the visible sheet: pixels past
    // the horizon flip the sign and would sample a mirrored ghost
    let ccx = dst_q.iter().map(|p| p.0).sum::<f64>() / 4.0;
    let ccy = dst_q.iter().map(|p| p.1).sum::<f64>() / 4.0;
    let wc = hm[6] * ccx + hm[7] * ccy + hm[8];
    let sdata = src.data();
    let cdata = parent.clips.last().map(|m| m.data());
    let pdata = parent.pix.data_mut();
    for oy in iy0..iy1 {
        for ox in ix0..ix1 {
            let cov = match cdata {
                Some(m) => f64::from(m[(oy * pw + ox) as usize]) / 255.0,
                None => 1.0,
            };
            if cov <= 0.0 {
                continue;
            }
            let (fx, fy) = (ox as f64 + 0.5, oy as f64 + 0.5);
            let ww = hm[6] * fx + hm[7] * fy + hm[8];
            if ww == 0.0 || ww * wc < 0.0 {
                continue;
            }
            let su = (hm[0] * fx + hm[1] * fy + hm[2]) / ww;
            let sv = (hm[3] * fx + hm[4] * fy + hm[5]) / ww;
            let px = sample_bilinear(sdata, sw, sh, su, sv);
            let alpha = px[3] * cov;
            if alpha <= 0.0 {
                continue;
            }
            let inv = 1.0 - alpha / 255.0;
            let di = ((oy * pw + ox) * 4) as usize;
            for (k, chan) in px.iter().enumerate() {
                let v = chan * cov + f64::from(pdata[di + k]) * inv;
                pdata[di + k] = v.round().min(255.0) as u8;
            }
        }
    }
}

pub struct Raster<'a> {
    s: &'a Slir,
    images: &'a [Vec<u8>],
    runtime_images: &'a [crate::render::RuntimeImage<'a>],
    registered_fonts: &'a [RegisteredFont],
    scale: f64,
}

struct GlyphSink {
    pb: PathBuilder,
    s: f32,
    dx: f32,
    dy: f32,
}

impl ttf_parser::OutlineBuilder for GlyphSink {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pb.move_to(self.dx + x * self.s, self.dy - y * self.s);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.pb.line_to(self.dx + x * self.s, self.dy - y * self.s);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.pb.quad_to(
            self.dx + x1 * self.s,
            self.dy - y1 * self.s,
            self.dx + x * self.s,
            self.dy - y * self.s,
        );
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.pb.cubic_to(
            self.dx + x1 * self.s,
            self.dy - y1 * self.s,
            self.dx + x2 * self.s,
            self.dy - y2 * self.s,
            self.dx + x * self.s,
            self.dy - y * self.s,
        );
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

impl<'a> Raster<'a> {
    pub fn new(
        s: &'a Slir,
        images: &'a [Vec<u8>],
        runtime_images: &'a [crate::render::RuntimeImage<'a>],
        registered_fonts: &'a [RegisteredFont],
        scale: f64,
    ) -> Self {
        Raster {
            s,
            runtime_images,
            images,
            registered_fonts,
            scale,
        }
    }

    fn face(&self, font_ix: i32) -> Option<ttf_parser::Face<'_>> {
        let font = self.s.fonts.get(font_ix.max(0) as usize)?;
        let bytes = self
            .registered_fonts
            .iter()
            .enumerate()
            .filter(|(_, registered)| {
                registered
                    .name
                    .eq_ignore_ascii_case(self.s.str_at(font.family))
            })
            .min_by_key(|(index, registered)| {
                (
                    registered.metrics.weight.abs_diff(font.weight),
                    usize::MAX - *index,
                )
            })
            .map(|(_, registered)| registered.bytes.as_slice())
            .unwrap_or_else(|| slab_fonts::asset(font.class, font.weight).bytes);
        ttf_parser::Face::parse(bytes, 0).ok()
    }

    /// Solid or gradient Paint for a `(kind, handle)` SLIR paint; geometry in
    /// device px.
    fn paint(
        &self,
        kind: u32,
        h: u32,
        x: f64,
        y: f64,
        w: f64,
        hh: f64,
        opacity: f64,
    ) -> Option<Paint<'static>> {
        let mut paint = base_paint();
        match kind {
            1 => {
                paint.set_color(rgba8(h, opacity));
                Some(paint)
            }
            2 => {
                let gr = self.s.grads.get(h as usize)?;
                let stops: Vec<GradientStop> = gr
                    .stops
                    .iter()
                    .map(|&(pos, rgba)| GradientStop::new(pos as f32, rgba8(rgba, opacity)))
                    .collect();
                let shader = match gr.kind {
                    0 => {
                        let th = gr.angle.to_radians();
                        let (dx, dy) = (th.sin(), -th.cos());
                        let ln = (w * dx).abs() + (hh * dy).abs();
                        let (cx, cy) = (x + w / 2.0, y + hh / 2.0);
                        LinearGradient::new(
                            Point::from_xy(
                                (cx - dx * ln / 2.0) as f32,
                                (cy - dy * ln / 2.0) as f32,
                            ),
                            Point::from_xy(
                                (cx + dx * ln / 2.0) as f32,
                                (cy + dy * ln / 2.0) as f32,
                            ),
                            stops,
                            SpreadMode::Pad,
                            Transform::identity(),
                        )?
                    }
                    1 => {
                        let c = Point::from_xy((x + w / 2.0) as f32, (y + hh / 2.0) as f32);
                        let r = ((w * w + hh * hh).sqrt() / 2.0) as f32;
                        RadialGradient::new(
                            c,
                            0.0,
                            c,
                            r.max(0.001),
                            stops,
                            SpreadMode::Pad,
                            Transform::identity(),
                        )?
                    }
                    // conic has no tiny-skia shader; those call sites route
                    // through `conic_through` instead
                    _ => return None,
                };
                paint.shader = shader;
                Some(paint)
            }
            _ => None,
        }
    }

    fn fill(&self, surf: &mut Layer, path: &Path, paint: &Paint) {
        surf.pix.fill_path(
            path,
            paint,
            FillRule::Winding,
            Transform::identity(),
            surf.clips.last(),
        );
    }

    /// Drop (outset) shadow: rrect/squircle coverage blurred 3x, drawn
    /// offset beneath.
    fn box_shadow(
        &self,
        surf: &mut Layer,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        r: f64,
        smooth: f64,
        sdx: f64,
        sdy: f64,
        blur: f64,
        spread: f64,
        rgba: u32,
    ) {
        if rgba >> 24 == 0 {
            return;
        }
        let (x0, y0, x1, y1) = (x0 - spread, y0 - spread, x1 + spread, y1 + spread);
        let rad = blur_rad(blur);
        let pad = (rad * 3 + 1) as f64;
        let (w, h) = (x1 - x0, y1 - y0);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let mw = (w + 2.0 * pad).ceil() as u32;
        let mh = (h + 2.0 * pad).ceil() as u32;
        let Some(mut tmp) = Pixmap::new(mw, mh) else {
            return;
        };
        let Some(path) = shape_path(pad, pad, pad + w, pad + h, (r + spread).max(0.0), smooth)
        else {
            return;
        };
        let mut paint = base_paint();
        paint.set_color(rgba8(rgba, 1.0));
        tmp.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        blur_rgba(tmp.data_mut(), mw as usize, mh as usize, rad);
        // CSS box-shadow semantics: an outer shadow is never painted under the
        // element's border-box (hard knockout after the blur) — matches the web
        // driver and keeps translucent glass fills from darkening themselves.
        if let Some(inner) = shape_path(
            pad + spread - sdx,
            pad + spread - sdy,
            pad + spread - sdx + (w - 2.0 * spread),
            pad + spread - sdy + (h - 2.0 * spread),
            r,
            smooth,
        ) {
            let mut clear = base_paint();
            clear.set_color(Color::TRANSPARENT);
            clear.blend_mode = tiny_skia::BlendMode::Source;
            tmp.fill_path(
                &inner,
                &clear,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        surf.pix.draw_pixmap(
            (x0 - pad + sdx).round() as i32,
            (y0 - pad + sdy).round() as i32,
            tmp.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            surf.clips.last(),
        );
    }

    /// Inner shadow: blurred inverse coverage, clipped to the rrect/squircle.
    fn inset_shadow(
        &self,
        surf: &mut Layer,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        r: f64,
        smooth: f64,
        sdx: f64,
        sdy: f64,
        blur: f64,
        rgba: u32,
    ) {
        if rgba >> 24 == 0 {
            return;
        }
        let rad = blur_rad(blur);
        let band = (rad * 3 + 3) as f64 + sdx.abs().max(sdy.abs());
        let (w, h) = (x1 - x0, y1 - y0);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let mw = (w + 2.0 * band).ceil() as u32;
        let mh = (h + 2.0 * band).ceil() as u32;
        let Some(mut tmp) = Pixmap::new(mw, mh) else {
            return;
        };
        tmp.fill(rgba8(rgba, 1.0));
        // punch out the (offset) rrect: what remains is the inverse coverage
        if let Some(path) = shape_path(
            band + sdx,
            band + sdy,
            band + sdx + w,
            band + sdy + h,
            r,
            smooth,
        ) {
            let mut clear = base_paint();
            clear.set_color(Color::TRANSPARENT);
            clear.blend_mode = tiny_skia::BlendMode::Source;
            tmp.fill_path(
                &path,
                &clear,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
        blur_rgba(tmp.data_mut(), mw as usize, mh as usize, rad);
        // clip to the rrect interior (intersected with the active clip)
        let Some(inner) = shape_path(x0, y0, x1, y1, r, smooth) else {
            return;
        };
        let (pw, ph) = (surf.pix.width(), surf.pix.height());
        let mut mask = match surf.clips.last() {
            Some(m) => m.clone(),
            None => {
                let mut m = Mask::new(pw, ph).unwrap();
                m.fill_path(&inner, FillRule::Winding, true, Transform::identity());
                surf.pix.draw_pixmap(
                    (x0 - band).round() as i32,
                    (y0 - band).round() as i32,
                    tmp.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    Some(&m),
                );
                return;
            }
        };
        mask.intersect_path(&inner, FillRule::Winding, true, Transform::identity());
        surf.pix.draw_pixmap(
            (x0 - band).round() as i32,
            (y0 - band).round() as i32,
            tmp.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            Some(&mask),
        );
    }

    /// Glass: blur/saturate/brighten what is already painted beneath the
    /// rrect/squircle. With a progressive mask paint the effect is quantized
    /// into 6 bands (contract §6.6): band i covers pixels whose mask alpha
    /// falls in [i/6, (i+1)/6) and applies `blur·α_i` with saturate and
    /// brightness lerped toward identity, `α_i = (i+0.5)/6`.
    fn backdrop(
        &self,
        surf: &mut Layer,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        r: f64,
        smooth: f64,
        blur: f64,
        sat: f64,
        bright: f64,
        mask: Option<(u32, u32)>,
    ) {
        const BANDS: usize = 6;
        let band_alpha = |i: usize| (i as f64 + 0.5) / BANDS as f64;
        let max_blur = if mask.is_some() {
            blur * band_alpha(BANDS - 1)
        } else {
            blur
        };
        let rad_max = blur_rad(max_blur);
        let pad = (rad_max * 3 + 1) as i64;
        let (pw, ph) = (surf.pix.width() as i64, surf.pix.height() as i64);
        let rx0 = (x0.floor() as i64 - pad).max(0);
        let ry0 = (y0.floor() as i64 - pad).max(0);
        let rx1 = (x1.ceil() as i64 + pad).min(pw);
        let ry1 = (y1.ceil() as i64 + pad).min(ph);
        let (rw, rh) = ((rx1 - rx0) as usize, (ry1 - ry0) as usize);
        if rw == 0 || rh == 0 {
            return;
        }
        // snapshot the region once (premultiplied); every band blurs the
        // pristine copy so band order never bleeds
        let mut region = vec![0u8; rw * rh * 4];
        let data = surf.pix.data();
        for yy in 0..rh {
            let src = (((ry0 as usize + yy) * pw as usize) + rx0 as usize) * 4;
            let dst = yy * rw * 4;
            region[dst..dst + rw * 4].copy_from_slice(&data[src..src + rw * 4]);
        }
        let Some(path) = shape_path(x0, y0, x1, y1, r, smooth) else {
            return;
        };
        let mut mask_cov = match surf.clips.last() {
            Some(m) => m.clone(),
            None => Mask::new(surf.pix.width(), surf.pix.height()).unwrap(),
        };
        if surf.clips.last().is_some() {
            mask_cov.intersect_path(&path, FillRule::Winding, true, Transform::identity());
        } else {
            mask_cov.fill_path(&path, FillRule::Winding, true, Transform::identity());
        }
        // per-pixel progressive band index: mask-paint alpha over the
        // backdrop rect at pixel centers
        let band_ix: Option<Vec<u8>> = mask.map(|(mk, mh)| {
            (0..rw * rh)
                .map(|p| {
                    let px = (rx0 + (p % rw) as i64) as f64 + 0.5;
                    let py = (ry0 + (p / rw) as i64) as f64 + 0.5;
                    let a = self.paint_alpha(mk, mh, px, py, x0, y0, x1 - x0, y1 - y0);
                    (a * BANDS as f64).floor().clamp(0.0, BANDS as f64 - 1.0) as u8
                })
                .collect()
        });
        let mdata = mask_cov.data();
        let passes = if band_ix.is_some() { BANDS } else { 1 };
        for band in 0..passes {
            let (blur_i, sat_i, bright_i) = if band_ix.is_some() {
                let a = band_alpha(band);
                (blur * a, 1.0 + (sat - 1.0) * a, 1.0 + (bright - 1.0) * a)
            } else {
                (blur, sat, bright)
            };
            let mut buf = region.clone();
            blur_rgba(&mut buf, rw, rh, blur_rad(blur_i));
            if sat_i != 1.0 || bright_i != 1.0 {
                for px in buf.chunks_exact_mut(4) {
                    let a = px[3] as f64;
                    if a == 0.0 {
                        continue;
                    }
                    let (br, bg, bb) = (
                        px[0] as f64 * 255.0 / a,
                        px[1] as f64 * 255.0 / a,
                        px[2] as f64 * 255.0 / a,
                    );
                    let luma = 0.2126 * br + 0.7152 * bg + 0.0722 * bb;
                    // saturate about luma, then brightness — CSS filter-list
                    // order blur() saturate() brightness()
                    let adj = |v: f64| {
                        ((luma + (v - luma) * sat_i) * bright_i)
                            .round()
                            .clamp(0.0, 255.0)
                    };
                    px[0] = (adj(br) * a / 255.0).round() as u8;
                    px[1] = (adj(bg) * a / 255.0).round() as u8;
                    px[2] = (adj(bb) * a / 255.0).round() as u8;
                }
            }
            // Composite the treated region back, weighted by rrect coverage
            // (research `backdrop`: replace at full coverage, lerp at edges).
            // NOT draw_pixmap(BlendMode::Source, mask): tiny-skia's pipeline
            // applies a clip mask to Source by storing src*coverage WITHOUT
            // loading dst, so any SIMD batch straddling the rrect edge would
            // punch transparent holes into already-painted pixels.
            let data = surf.pix.data_mut();
            for yy in 0..rh {
                let row = (ry0 as usize + yy) * pw as usize + rx0 as usize;
                for xx in 0..rw {
                    if let Some(bix) = &band_ix
                        && bix[yy * rw + xx] != band as u8
                    {
                        continue;
                    }
                    let c = u32::from(mdata[row + xx]);
                    if c == 0 {
                        continue;
                    }
                    let si = (yy * rw + xx) * 4;
                    let di = (row + xx) * 4;
                    if c == 255 {
                        data[di..di + 4].copy_from_slice(&buf[si..si + 4]);
                    } else {
                        for k in 0..4 {
                            let s = u32::from(buf[si + k]);
                            let d = u32::from(data[di + k]);
                            data[di + k] = ((s * c + d * (255 - c) + 127) / 255) as u8;
                        }
                    }
                }
            }
        }
    }

    fn draw_rect(&mut self, surf: &mut Layer, r: &OpRect) {
        let s = self.scale;
        let (x0, y0) = (r.x * s, r.y * s);
        let (x1, y1) = ((r.x + r.w) * s, (r.y + r.h) * s);
        for i in 0..r.shadow_len.max(0) as usize {
            let sh = &self.s.shadows[r.shadow_off as usize + i];
            if sh.inset == 0 {
                self.box_shadow(
                    surf,
                    x0,
                    y0,
                    x1,
                    y1,
                    r.radius * s,
                    r.smooth,
                    sh.x * s,
                    sh.y * s,
                    (sh.blur * s).max(1.0),
                    sh.spread * s,
                    sh.rgba,
                );
            }
        }
        if self.is_conic(r.bg_kind, r.bg) {
            if let Some(path) = shape_path(x0, y0, x1, y1, r.radius * s, r.smooth) {
                self.conic_through(
                    surf,
                    r.bg,
                    (x0, y0, x1 - x0, y1 - y0),
                    r.opacity,
                    (x0, y0, x1, y1),
                    &|pix, clip| {
                        let mut white = base_paint();
                        white.set_color(Color::WHITE);
                        pix.fill_path(
                            &path,
                            &white,
                            FillRule::Winding,
                            Transform::identity(),
                            clip,
                        );
                    },
                );
            }
        } else if let Some(paint) = self.paint(r.bg_kind, r.bg, x0, y0, x1 - x0, y1 - y0, r.opacity)
            && let Some(path) = shape_path(x0, y0, x1, y1, r.radius * s, r.smooth)
        {
            self.fill(surf, &path, &paint);
        }
        for i in 0..r.shadow_len.max(0) as usize {
            let sh = &self.s.shadows[r.shadow_off as usize + i];
            if sh.inset != 0 {
                self.inset_shadow(
                    surf,
                    x0,
                    y0,
                    x1,
                    y1,
                    r.radius * s,
                    r.smooth,
                    sh.x * s,
                    sh.y * s,
                    (sh.blur * s).max(1.0),
                    sh.rgba,
                );
            }
        }
        if r.grain_amount > 0.0 {
            self.grain(surf, r, x0, y0, x1, y1);
        }
        if r.stroke_kind == 0 {
            return;
        }
        let hw = r.stroke_w * s / 2.0;
        let off = match r.stroke_align {
            1 => hw,
            2 => -hw,
            _ => 0.0,
        };
        let (sx0, sy0, sx1, sy1) = (x0 + off, y0 + off, x1 - off, y1 - off);
        let dash = if r.has_dash {
            StrokeDash::new(vec![(r.dash_on * s) as f32, (r.dash_off * s) as f32], 0.0)
        } else {
            None
        };
        // gradient geometry spans the stroke's coverage box (its outer bounds)
        let (gx0, gy0, gx1, gy1) = (sx0 - hw, sy0 - hw, sx1 + hw, sy1 + hw);
        if self.is_conic(r.stroke_kind, r.stroke) {
            self.conic_through(
                surf,
                r.stroke,
                (gx0, gy0, gx1 - gx0, gy1 - gy0),
                r.opacity,
                (gx0, gy0, gx1, gy1),
                &|pix, clip| {
                    let mut white = base_paint();
                    white.set_color(Color::WHITE);
                    rect_stroke_geo(
                        pix,
                        clip,
                        &white,
                        r,
                        s,
                        sx0,
                        sy0,
                        sx1,
                        sy1,
                        hw,
                        off,
                        dash.clone(),
                    );
                },
            );
        } else if let Some(paint) = self.paint(
            r.stroke_kind,
            r.stroke,
            gx0,
            gy0,
            gx1 - gx0,
            gy1 - gy0,
            r.opacity,
        ) {
            rect_stroke_geo(
                &mut surf.pix,
                surf.clips.last(),
                &paint,
                r,
                s,
                sx0,
                sy0,
                sx1,
                sy1,
                hw,
                off,
                dash,
            );
        }
    }

    /// True when the paint is a conic gradient — the one paint tiny-skia has
    /// no shader for; those call sites rasterize via `conic_through`.
    fn is_conic(&self, kind: u32, h: u32) -> bool {
        kind == 2 && self.s.grads.get(h as usize).is_some_and(|g| g.kind == 2)
    }

    /// Samples a paint at a device-px point over its geometry box — kernel
    /// `paint_rgba_at` semantics (linear over the CSS-angle projection,
    /// radial to the farthest corner) plus the contract §6.1 conic sweep
    /// `t = (atan2(px−cx, cy−py)° − from).rem_euclid(360) / 360`.
    fn paint_at(
        &self,
        kind: u32,
        h: u32,
        px: f64,
        py: f64,
        bx: f64,
        by: f64,
        bw: f64,
        bh: f64,
    ) -> u32 {
        if kind == 1 {
            return h;
        }
        if kind != 2 {
            return 0;
        }
        let Some(gr) = self.s.grads.get(h as usize) else {
            return 0;
        };
        let Some(&(_, first)) = gr.stops.first() else {
            return 0;
        };
        let (cx, cy) = (bx + bw / 2.0, by + bh / 2.0);
        let t = match gr.kind {
            0 => {
                let th = gr.angle.to_radians();
                let (dx, dy) = (th.sin(), -th.cos());
                let ln = (bw * dx).abs() + (bh * dy).abs();
                if ln <= 0.0 {
                    return first;
                }
                ((px - (cx - dx * ln / 2.0)) * dx + (py - (cy - dy * ln / 2.0)) * dy) / ln
            }
            1 => {
                let far = (bw * bw / 4.0 + bh * bh / 4.0).sqrt();
                if far <= 0.0 {
                    return first;
                }
                (px - cx).hypot(py - cy) / far
            }
            _ => ((px - cx).atan2(cy - py).to_degrees() - gr.angle).rem_euclid(360.0) / 360.0,
        };
        ramp(&gr.stops, t.clamp(0.0, 1.0))
    }

    /// Alpha (0..1) of a paint at a point — mask sampling uses alpha only.
    fn paint_alpha(
        &self,
        kind: u32,
        h: u32,
        px: f64,
        py: f64,
        bx: f64,
        by: f64,
        bw: f64,
        bh: f64,
    ) -> f64 {
        f64::from(self.paint_at(kind, h, px, py, bx, by, bw, bh) >> 24) / 255.0
    }

    /// Rasterizes a conic paint through arbitrary geometry: `draw` renders
    /// the coverage in opaque white onto a scratch canvas (honoring the
    /// active clip), then the §6.1 sweep is source-over blended per pixel,
    /// weighted by that coverage. `gbox` is the gradient box (x, y, w, h)
    /// and `bounds` the geometry bounds (x0, y0, x1, y1), both device px.
    fn conic_through(
        &self,
        surf: &mut Layer,
        handle: u32,
        gbox: (f64, f64, f64, f64),
        opacity: f64,
        bounds: (f64, f64, f64, f64),
        draw: &dyn Fn(&mut Pixmap, Option<&Mask>),
    ) {
        let (pw, ph) = (surf.pix.width(), surf.pix.height());
        let Some(mut cov) = Pixmap::new(pw, ph) else {
            return;
        };
        draw(&mut cov, surf.clips.last());
        let (bx, by, bw, bh) = gbox;
        let ix0 = (bounds.0.floor() as i64).max(0) as usize;
        let iy0 = (bounds.1.floor() as i64).max(0) as usize;
        let ix1 = (bounds.2.ceil() as i64).min(pw as i64).max(0) as usize;
        let iy1 = (bounds.3.ceil() as i64).min(ph as i64).max(0) as usize;
        let cdata = cov.data();
        let data = surf.pix.data_mut();
        for iy in iy0..iy1 {
            for ix in ix0..ix1 {
                let pi = (iy * pw as usize + ix) * 4;
                let c = f64::from(cdata[pi + 3]) / 255.0;
                if c <= 0.0 {
                    continue;
                }
                let rgba =
                    self.paint_at(2, handle, ix as f64 + 0.5, iy as f64 + 0.5, bx, by, bw, bh);
                let [rr, gg, bb, aa] = rgba.to_le_bytes();
                let alpha = f64::from(aa) / 255.0 * opacity * c;
                if alpha <= 0.0 {
                    continue;
                }
                let inv = 1.0 - alpha;
                let src = [
                    f64::from(rr) * alpha,
                    f64::from(gg) * alpha,
                    f64::from(bb) * alpha,
                    255.0 * alpha,
                ];
                for (k, sv) in src.iter().enumerate() {
                    let v = sv + f64::from(data[pi + k]) * inv;
                    data[pi + k] = v.round().min(255.0) as u8;
                }
            }
        }
    }

    /// Deterministic monochrome speckle over the rect's coverage (contract
    /// §6.2): pcg2d on node-local speckle cells (logical units, so motion
    /// never re-rolls the noise), white above / black below the midpoint,
    /// alpha = amount·|h| (scaled by node opacity), painted after fill and
    /// inset shadows, clipped to the rounded/smooth rect.
    fn grain(&self, surf: &mut Layer, r: &OpRect, x0: f64, y0: f64, x1: f64, y1: f64) {
        let s = self.scale;
        let amount = r.grain_amount.clamp(0.0, 1.0) * r.opacity;
        let size = if r.grain_size > 0.0 {
            r.grain_size
        } else {
            1.0
        };
        let Some(path) = shape_path(x0, y0, x1, y1, r.radius * s, r.smooth) else {
            return;
        };
        let (pw, ph) = (surf.pix.width(), surf.pix.height());
        let mut mask = match surf.clips.last() {
            Some(m) => m.clone(),
            None => Mask::new(pw, ph).unwrap(),
        };
        if surf.clips.last().is_some() {
            mask.intersect_path(&path, FillRule::Winding, true, Transform::identity());
        } else {
            mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
        }
        let ix0 = (x0.floor() as i64).max(0) as usize;
        let iy0 = (y0.floor() as i64).max(0) as usize;
        let ix1 = (x1.ceil() as i64).min(pw as i64).max(0) as usize;
        let iy1 = (y1.ceil() as i64).min(ph as i64).max(0) as usize;
        let mdata = mask.data();
        let data = surf.pix.data_mut();
        for iy in iy0..iy1 {
            for ix in ix0..ix1 {
                let c = f64::from(mdata[iy * pw as usize + ix]) / 255.0;
                if c <= 0.0 {
                    continue;
                }
                // node-local logical speckle cell (floor, not truncate)
                let i = (((ix as f64 + 0.5) / s - r.x) / size).floor() as i64;
                let j = (((iy as f64 + 0.5) / s - r.y) / size).floor() as i64;
                let hash = pcg2d(i as u32, j as u32);
                let f = f64::from(hash) / 4_294_967_296.0;
                let sh = 2.0 * f - 1.0;
                let alpha = amount * sh.abs() * c;
                if alpha <= 0.0 {
                    continue;
                }
                let ink = if sh > 0.0 { 255.0 * alpha } else { 0.0 };
                let inv = 1.0 - alpha;
                let di = (iy * pw as usize + ix) * 4;
                for k in 0..3 {
                    let v = ink + f64::from(data[di + k]) * inv;
                    data[di + k] = v.round().min(255.0) as u8;
                }
                let va = 255.0 * alpha + f64::from(data[di + 3]) * inv;
                data[di + 3] = va.round().min(255.0) as u8;
            }
        }
    }

    /// Multiplies a popped group layer by its mask paint's alpha (contract
    /// §6.3): paint alpha sampled over the mask box, zero coverage outside.
    fn apply_group_mask(&self, pix: &mut Pixmap, m: &GroupMask) {
        let pw = pix.width() as usize;
        let ph = pix.height() as usize;
        let data = pix.data_mut();
        for y in 0..ph {
            for x in 0..pw {
                let i = (y * pw + x) * 4;
                if data[i..i + 4] == [0, 0, 0, 0] {
                    continue;
                }
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let inside = px >= m.x && px < m.x + m.w && py >= m.y && py < m.y + m.h;
                let a = if inside {
                    self.paint_alpha(m.kind, m.handle, px, py, m.x, m.y, m.w, m.h)
                } else {
                    0.0
                };
                if a >= 1.0 {
                    continue;
                }
                if a <= 0.0 {
                    data[i..i + 4].fill(0);
                    continue;
                }
                let na = (f64::from(data[i + 3]) * a).round() as u8;
                for k in 0..3 {
                    data[i + k] = ((f64::from(data[i + k]) * a).round() as u8).min(na);
                }
                data[i + 3] = na;
            }
        }
    }

    /// Combined glyph-run outline (device px); positions/advances from the
    /// SLIR FONT table (the solver's own metrics), outlines from the
    /// matching vendored TTF. One path for the whole run so gradient text
    /// paints with cross-glyph continuity.
    fn text_outline(
        &self,
        text: &str,
        font_ix: i32,
        x: f64,
        y_baseline: f64,
        size: f64,
        tracking: f64,
        strike: bool,
        measured_w: f64,
    ) -> Option<Path> {
        let doc = self.s;
        let fe = doc
            .fonts
            .get(font_ix.max(0) as usize)
            .filter(|_| font_ix >= 0)?;
        let (upem, default_adv) = (fe.upem as f64, fe.default_advance as f64);
        let cmap = &fe.cmap;
        let advances = &fe.advances;
        let face = self.face(font_ix)?;
        let s = self.scale;
        let size_px = size * s;
        let scale_units = size_px / upem;
        let mut sink = GlyphSink {
            pb: PathBuilder::new(),
            s: scale_units as f32,
            dx: 0.0,
            dy: (y_baseline * s) as f32,
        };
        let mut pen = x * s;
        for ch in text.chars() {
            let cp = ch as u32;
            let ix = cmap.binary_search_by_key(&cp, |&(c, _)| c).ok();
            let gid = ix.map(|i| cmap[i].1).unwrap_or(0);
            if graphemes::is_glyph_modifier(cp) {
                continue;
            }
            if gid != 0 && ch != ' ' {
                sink.dx = pen as f32;
                face.outline_glyph(ttf_parser::GlyphId(gid), &mut sink);
            }
            let adv_units = ix.map(|i| advances[i] as f64).unwrap_or(default_adv);
            pen += adv_units * size_px / upem + tracking * s;
        }
        if strike && measured_w > 0.0 {
            let center = (y_baseline - size * 0.3) * s;
            let thickness = (size * s / 16.0).max(1.0);
            sink.pb.push_rect(tiny_skia::Rect::from_ltrb(
                (x * s) as f32,
                (center - thickness / 2.0) as f32,
                ((x + measured_w) * s) as f32,
                (center + thickness / 2.0) as f32,
            )?);
        }
        sink.pb.finish()
    }

    /// Draw one text run: solid fill, gradient fill over the node's content
    /// box (`gx..gh`, contract §6.7), or per-pixel conic.
    fn draw_text(&mut self, surf: &mut Layer, t: &OpText, text: &str) {
        let Some(path) = self.text_outline(
            text,
            t.font,
            t.x,
            t.y_baseline,
            t.size,
            t.tracking,
            t.strike,
            t.measured_w,
        ) else {
            return;
        };
        let s = self.scale;
        if t.color_kind == 2 {
            let (gx, gy, gw, gh) = (t.gx * s, t.gy * s, t.gw * s, t.gh * s);
            if self.is_conic(2, t.color) {
                let b = path.bounds();
                self.conic_through(
                    surf,
                    t.color,
                    (gx, gy, gw, gh),
                    t.opacity,
                    (
                        b.x() as f64,
                        b.y() as f64,
                        b.right() as f64,
                        b.bottom() as f64,
                    ),
                    &|pix, clip| {
                        let mut white = base_paint();
                        white.set_color(Color::WHITE);
                        pix.fill_path(
                            &path,
                            &white,
                            FillRule::Winding,
                            Transform::identity(),
                            clip,
                        );
                    },
                );
            } else if let Some(paint) = self.paint(2, t.color, gx, gy, gw, gh, t.opacity) {
                self.fill(surf, &path, &paint);
            }
            return;
        }
        let mut paint = base_paint();
        paint.set_color(rgba8(t.color, t.opacity));
        self.fill(surf, &path, &paint);
    }

    /// Centered label with the vendored Inter regular (placeholder text; not
    /// kernel-measured — matches the research placeholder path).
    fn center_label(
        &mut self,
        surf: &mut Layer,
        text: &str,
        cx: f64,
        cy: f64,
        size_px: f64,
        color: Color,
    ) {
        let a = slab_fonts::asset(slab_fonts::CLASS_SANS, 400);
        let Ok(face) = ttf_parser::Face::parse(a.bytes, 0) else {
            return;
        };
        let upem = face.units_per_em() as f64;
        let advance = |ch: char| -> f64 {
            face.glyph_index(ch)
                .and_then(|g| face.glyph_hor_advance(g))
                .unwrap_or(0) as f64
                * size_px
                / upem
        };
        let wsum: f64 = text.chars().map(advance).sum();
        let mut pen = cx - wsum / 2.0;
        let ybase = cy + size_px * 0.35;
        let mut paint = base_paint();
        paint.set_color(color);
        for ch in text.chars() {
            if let Some(gid) = face.glyph_index(ch) {
                if ch != ' ' {
                    let mut sink = GlyphSink {
                        pb: PathBuilder::new(),
                        s: (size_px / upem) as f32,
                        dx: pen as f32,
                        dy: ybase as f32,
                    };
                    if face.outline_glyph(gid, &mut sink).is_some()
                        && let Some(path) = sink.pb.finish()
                    {
                        surf.pix.fill_path(
                            &path,
                            &paint,
                            FillRule::Winding,
                            Transform::identity(),
                            surf.clips.last(),
                        );
                    }
                }
                pen += face.glyph_hor_advance(gid).unwrap_or(0) as f64 * size_px / upem;
            }
        }
    }

    fn draw_image(&mut self, surf: &mut Layer, im: &slab_kernel::flatten::OpImage) {
        let s = self.scale;
        let (x0, y0) = (im.x * s, im.y * s);
        let (x1, y1) = ((im.x + im.w) * s, (im.y + im.h) * s);
        let compiled = self
            .s
            .images
            .get(im.img.max(0) as usize)
            .filter(|_| im.img >= 0);
        let runtime = self
            .runtime_images
            .iter()
            .rfind(|image| image.image == im.img);
        let decoded = if let Some(image) = runtime {
            decode_runtime_image(image)
        } else {
            self.images
                .get(im.img.max(0) as usize)
                .filter(|_| im.img >= 0)
                .filter(|bytes| !bytes.is_empty())
                .and_then(|bytes| decode_png(bytes).ok())
        };
        let Some(src_pix) = decoded else {
            // placeholder: checker rect + crossed diagonals + filename
            if let Some(path) = shape_path(x0, y0, x1, y1, im.radius * s, im.smooth) {
                let mut paint = base_paint();
                paint.set_color(Color::from_rgba8(201, 206, 214, 255));
                self.fill(surf, &path, &paint);
            }
            let mut paint = base_paint();
            paint.set_color(Color::from_rgba8(154, 161, 171, 255));
            let stroke = Stroke {
                width: s as f32,
                ..Stroke::default()
            };
            for (a, b, c, d) in [(x0, y0, x1, y1), (x1, y0, x0, y1)] {
                let mut pb = PathBuilder::new();
                pb.move_to(a as f32, b as f32);
                pb.line_to(c as f32, d as f32);
                if let Some(path) = pb.finish() {
                    surf.pix.stroke_path(
                        &path,
                        &paint,
                        &stroke,
                        Transform::identity(),
                        surf.clips.last(),
                    );
                }
            }
            let src = compiled.map(|image| self.s.str_at(image.src)).unwrap_or("");
            let label = src
                .rsplit('/')
                .next()
                .filter(|l| !l.is_empty())
                .unwrap_or("image");
            let label = label.to_string();
            self.center_label(
                surf,
                &label,
                (x0 + x1) / 2.0,
                (y0 + y1) / 2.0,
                11.0 * s,
                Color::from_rgba8(91, 100, 112, 255),
            );
            return;
        };
        let (iw, ih) = (src_pix.width() as f64, src_pix.height() as f64);
        let (dw, dh) = (x1 - x0, y1 - y0);
        if iw <= 0.0 || ih <= 0.0 || dw <= 0.0 || dh <= 0.0 {
            return;
        }
        let (sx, sy) = match im.fit {
            1 => {
                let k = (dw / iw).min(dh / ih);
                (k, k)
            }
            2 => (dw / iw, dh / ih),
            _ => {
                let k = (dw / iw).max(dh / ih);
                (k, k)
            }
        };
        let tx = x0 + (dw - iw * sx) / 2.0;
        let ty = y0 + (dh - ih * sy) / 2.0;
        // clip to the (rounded) dest rect
        let Some(path) = shape_path(x0, y0, x1, y1, im.radius * s, im.smooth) else {
            return;
        };
        let mask = match surf.clips.last() {
            Some(m) => {
                let mut m = m.clone();
                m.intersect_path(&path, FillRule::Winding, true, Transform::identity());
                m
            }
            None => {
                let mut m = Mask::new(surf.pix.width(), surf.pix.height()).unwrap();
                m.fill_path(&path, FillRule::Winding, true, Transform::identity());
                m
            }
        };
        let paint = PixmapPaint {
            opacity: im.opacity as f32,
            quality: tiny_skia::FilterQuality::Bilinear,
            ..PixmapPaint::default()
        };
        surf.pix.draw_pixmap(
            0,
            0,
            src_pix.as_ref(),
            &paint,
            Transform::from_row(sx as f32, 0.0, 0.0, sy as f32, tx as f32, ty as f32),
            Some(&mask),
        );
    }

    fn slir_path(&self, frame: &Frame, index: i32) -> Option<Path> {
        let (verbs, coords): (&[u8], &[f64]) = if index >= 0 {
            let path = self.s.paths.get(index as usize)?;
            (&path.verbs, &path.coords)
        } else {
            let path = frame.paths_rt.get((!index) as usize)?;
            (&path.verbs, &path.coords)
        };
        let mut builder = PathBuilder::new();
        let mut coordinate = 0usize;
        for &verb in verbs {
            match verb {
                0 => {
                    builder.move_to(coords[coordinate] as f32, coords[coordinate + 1] as f32);
                    coordinate += 2;
                }
                1 => {
                    builder.line_to(coords[coordinate] as f32, coords[coordinate + 1] as f32);
                    coordinate += 2;
                }
                2 => {
                    builder.cubic_to(
                        coords[coordinate] as f32,
                        coords[coordinate + 1] as f32,
                        coords[coordinate + 2] as f32,
                        coords[coordinate + 3] as f32,
                        coords[coordinate + 4] as f32,
                        coords[coordinate + 5] as f32,
                    );
                    coordinate += 6;
                }
                3 => {
                    builder.quad_to(
                        coords[coordinate] as f32,
                        coords[coordinate + 1] as f32,
                        coords[coordinate + 2] as f32,
                        coords[coordinate + 3] as f32,
                    );
                    coordinate += 4;
                }
                _ => builder.close(),
            }
        }
        builder.finish()
    }

    /// Rasterize a solved Frame at `scale` device px per unit.
    pub fn render(&mut self, frame: &Frame) -> Result<Pixmap, String> {
        let s = self.scale;
        let w = ((frame.width * s).round() as u32).max(1);
        let h = ((frame.height * s).round() as u32).max(1);
        let base = Pixmap::new(w, h).ok_or("pixmap alloc failed")?;
        let mut stack: Vec<Layer> = vec![Layer {
            pix: base,
            clips: Vec::new(),
            kind: LayerKind::Base,
        }];

        for (op_index, op) in frame.ops.iter().enumerate() {
            match op {
                FrameOp::Rect(r) => self.draw_rect(stack.last_mut().unwrap(), r),
                FrameOp::Text(t) => {
                    let text = frame
                        .strings
                        .get(t.str_ref as usize)
                        .cloned()
                        .unwrap_or_default();
                    self.draw_text(stack.last_mut().unwrap(), t, &text);
                }
                FrameOp::Image(im) => self.draw_image(stack.last_mut().unwrap(), im),
                FrameOp::PathDraw(p) => {
                    let Some(path) = self.slir_path(frame, p.path) else {
                        continue;
                    };
                    let t = Transform::from_row(
                        s as f32,
                        0.0,
                        0.0,
                        s as f32,
                        (p.dx * s) as f32,
                        (p.dy * s) as f32,
                    );
                    let bounds = path.bounds();
                    // Device-space bounds for the per-pixel conic route.
                    let dev = (
                        bounds.x() as f64 * s + p.dx * s,
                        bounds.y() as f64 * s + p.dy * s,
                        bounds.width() as f64 * s,
                        bounds.height() as f64 * s,
                    );
                    if self.is_conic(p.bg_kind, p.bg) {
                        let surf = stack.last_mut().unwrap();
                        self.conic_through(
                            surf,
                            p.bg,
                            dev,
                            p.opacity,
                            (dev.0, dev.1, dev.0 + dev.2, dev.1 + dev.3),
                            &|pix, clip| {
                                let mut white = base_paint();
                                white.set_color(Color::WHITE);
                                pix.fill_path(&path, &white, FillRule::Winding, t, clip);
                            },
                        );
                    } else if let Some(paint) = self.paint(
                        p.bg_kind,
                        p.bg,
                        bounds.x() as f64,
                        bounds.y() as f64,
                        bounds.width() as f64,
                        bounds.height() as f64,
                        p.opacity,
                    ) {
                        // gradient geometry in path space: the draw transform
                        // maps path AND shader coords into device space
                        // together
                        let surf = stack.last_mut().unwrap();
                        surf.pix
                            .fill_path(&path, &paint, FillRule::Winding, t, surf.clips.last());
                    }
                    if p.stroke_kind != 0 {
                        let stroke = Stroke {
                            width: p.stroke_w as f32, // doc units; t scales at draw
                            line_cap: if p.has_dash {
                                tiny_skia::LineCap::Butt
                            } else {
                                tiny_skia::LineCap::Round
                            },
                            line_join: tiny_skia::LineJoin::Round,
                            dash: if p.has_dash {
                                StrokeDash::new(vec![p.dash_on as f32, p.dash_off as f32], 0.0)
                            } else {
                                None
                            },
                            ..Stroke::default()
                        };
                        if self.is_conic(p.stroke_kind, p.stroke) {
                            // stroke ink reaches half the device stroke width
                            // beyond the path bounds
                            let hw = p.stroke_w * s / 2.0;
                            let surf = stack.last_mut().unwrap();
                            self.conic_through(
                                surf,
                                p.stroke,
                                dev,
                                p.opacity,
                                (
                                    dev.0 - hw,
                                    dev.1 - hw,
                                    dev.0 + dev.2 + hw,
                                    dev.1 + dev.3 + hw,
                                ),
                                &|pix, clip| {
                                    let mut white = base_paint();
                                    white.set_color(Color::WHITE);
                                    pix.stroke_path(&path, &white, &stroke, t, clip);
                                },
                            );
                        } else if let Some(paint) = self.paint(
                            p.stroke_kind,
                            p.stroke,
                            bounds.x() as f64,
                            bounds.y() as f64,
                            bounds.width() as f64,
                            bounds.height() as f64,
                            p.opacity,
                        ) {
                            let surf = stack.last_mut().unwrap();
                            surf.pix
                                .stroke_path(&path, &paint, &stroke, t, surf.clips.last());
                        }
                    }
                }
                FrameOp::ClipPush(c) => {
                    let surf = stack.last_mut().unwrap();
                    let path = shape_path(
                        c.x * s,
                        c.y * s,
                        (c.x + c.w) * s,
                        (c.y + c.h) * s,
                        c.radius * s,
                        c.smooth,
                    );
                    let mut mask = match surf.clips.last() {
                        Some(m) => m.clone(),
                        None => {
                            Mask::new(surf.pix.width(), surf.pix.height()).ok_or("mask alloc")?
                        }
                    };
                    if let Some(path) = path {
                        if surf.clips.is_empty() {
                            mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
                        } else {
                            mask.intersect_path(
                                &path,
                                FillRule::Winding,
                                true,
                                Transform::identity(),
                            );
                        }
                    }
                    surf.clips.push(mask);
                }
                FrameOp::ClipPop => {
                    stack.last_mut().unwrap().clips.pop();
                }
                FrameOp::GroupPush(gp) => {
                    let (layer_w, layer_h) = {
                        let parent = stack.last().expect("layer stack has a root");
                        (parent.pix.width(), parent.pix.height())
                    };
                    let pix = Pixmap::new(layer_w, layer_h).ok_or("layer alloc failed")?;
                    let mask = (gp.mask_kind != 0).then_some(GroupMask {
                        kind: gp.mask_kind,
                        handle: gp.mask,
                        x: gp.mx * s,
                        y: gp.my * s,
                        w: gp.mw * s,
                        h: gp.mh * s,
                    });
                    stack.push(Layer {
                        pix,
                        clips: Vec::new(),
                        kind: LayerKind::Group {
                            opacity: gp.opacity,
                            blur: gp.blur * s,
                            mask,
                        },
                    });
                }
                FrameOp::RotatePush(rt) => {
                    let (layer_w, layer_h) = {
                        let parent = stack.last().expect("layer stack has a root");
                        (parent.pix.width(), parent.pix.height())
                    };
                    let pix = Pixmap::new(layer_w, layer_h).ok_or("layer alloc failed")?;
                    stack.push(Layer {
                        pix,
                        clips: Vec::new(),
                        kind: LayerKind::Rotate {
                            deg: rt.deg,
                            cx: rt.cx * s,
                            cy: rt.cy * s,
                        },
                    });
                }
                FrameOp::ScalePush(scale) => {
                    let (parent_w, parent_h) = {
                        let parent = stack.last().expect("layer stack has a root");
                        (parent.pix.width(), parent.pix.height())
                    };
                    // Bound the temporary surface by authored subtree ink,
                    // never by `viewport / scale`: an arbitrarily small scale
                    // must not turn into an arbitrarily large allocation.
                    let mut source_w = f64::from(parent_w) / s;
                    let mut source_h = f64::from(parent_h) / s;
                    let mut depth = 1usize;
                    for enclosed in frame.ops.iter().skip(op_index + 1) {
                        match enclosed {
                            FrameOp::ScalePush(_) => depth += 1,
                            FrameOp::ScalePop => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            FrameOp::Rect(rect) => {
                                let pad = rect.stroke_w.max(0.0)
                                    + if rect.shadow_len > 0 { 64.0 } else { 0.0 };
                                source_w = source_w.max(rect.x + rect.w + pad);
                                source_h = source_h.max(rect.y + rect.h + pad);
                            }
                            FrameOp::Text(text) => {
                                source_w = source_w.max(text.x + text.measured_w);
                                source_h = source_h.max(text.y_baseline + text.size);
                            }
                            FrameOp::Image(image) => {
                                source_w = source_w.max(image.x + image.w);
                                source_h = source_h.max(image.y + image.h);
                            }
                            FrameOp::PathDraw(path_op) => {
                                if let Some(path) = self.slir_path(frame, path_op.path) {
                                    let bounds = path.bounds();
                                    let pad = path_op.stroke_w.max(0.0);
                                    source_w = source_w.max(
                                        path_op.dx + f64::from(bounds.x() + bounds.width()) + pad,
                                    );
                                    source_h = source_h.max(
                                        path_op.dy + f64::from(bounds.y() + bounds.height()) + pad,
                                    );
                                }
                            }
                            FrameOp::Backdrop(backdrop) => {
                                source_w = source_w.max(backdrop.x + backdrop.w);
                                source_h = source_h.max(backdrop.y + backdrop.h);
                            }
                            _ => {}
                        }
                    }
                    let layer_w = (source_w * s).ceil().clamp(1.0, f64::from(u32::MAX)) as u32;
                    let layer_h = (source_h * s).ceil().clamp(1.0, f64::from(u32::MAX)) as u32;
                    let pix =
                        Pixmap::new(layer_w.max(1), layer_h.max(1)).ok_or("layer alloc failed")?;
                    stack.push(Layer {
                        pix,
                        clips: Vec::new(),
                        kind: scale_layer_kind(scale, s),
                    });
                }
                FrameOp::TiltPush(tl) => {
                    let (layer_w, layer_h) = {
                        let parent = stack.last().expect("layer stack has a root");
                        (parent.pix.width(), parent.pix.height())
                    };
                    let pix = Pixmap::new(layer_w, layer_h).ok_or("layer alloc failed")?;
                    stack.push(Layer {
                        pix,
                        clips: Vec::new(),
                        kind: LayerKind::Tilt {
                            cx: tl.cx * s,
                            cy: tl.cy * s,
                            rx: tl.rx,
                            ry: tl.ry,
                            depth: tl.depth * s,
                        },
                    });
                }
                FrameOp::GroupPop | FrameOp::RotatePop | FrameOp::ScalePop | FrameOp::TiltPop => {
                    if stack.len() < 2 {
                        continue;
                    }
                    let mut layer = stack.pop().unwrap();
                    let parent = stack.last_mut().unwrap();
                    match layer.kind {
                        LayerKind::Group {
                            opacity,
                            blur,
                            mask,
                        } => {
                            if blur > 0.0 {
                                let (lw, lh) =
                                    (layer.pix.width() as usize, layer.pix.height() as usize);
                                blur_rgba(layer.pix.data_mut(), lw, lh, blur_rad(blur));
                            }
                            if let Some(m) = &mask {
                                self.apply_group_mask(&mut layer.pix, m);
                            }
                            let paint = PixmapPaint {
                                opacity: opacity as f32,
                                ..PixmapPaint::default()
                            };
                            parent.pix.draw_pixmap(
                                0,
                                0,
                                layer.pix.as_ref(),
                                &paint,
                                Transform::identity(),
                                parent.clips.last(),
                            );
                        }
                        LayerKind::Rotate { deg, cx, cy } => {
                            let paint = PixmapPaint {
                                quality: tiny_skia::FilterQuality::Bilinear,
                                ..PixmapPaint::default()
                            };
                            parent.pix.draw_pixmap(
                                0,
                                0,
                                layer.pix.as_ref(),
                                &paint,
                                Transform::from_rotate_at(deg as f32, cx as f32, cy as f32),
                                parent.clips.last(),
                            );
                        }
                        LayerKind::Scale { cx, cy, sx, sy } => {
                            let paint = PixmapPaint {
                                quality: tiny_skia::FilterQuality::Bilinear,
                                ..PixmapPaint::default()
                            };
                            parent.pix.draw_pixmap(
                                0,
                                0,
                                layer.pix.as_ref(),
                                &paint,
                                Transform::from_row(
                                    sx as f32,
                                    0.0,
                                    0.0,
                                    sy as f32,
                                    (cx * (1.0 - sx)) as f32,
                                    (cy * (1.0 - sy)) as f32,
                                ),
                                parent.clips.last(),
                            );
                        }
                        LayerKind::Tilt {
                            cx,
                            cy,
                            rx,
                            ry,
                            depth,
                        } => {
                            tilt_composite(parent, &layer.pix, cx, cy, rx, ry, depth);
                        }
                        LayerKind::Base => {}
                    }
                }
                FrameOp::Backdrop(b) => {
                    self.backdrop(
                        stack.last_mut().unwrap(),
                        b.x * s,
                        b.y * s,
                        (b.x + b.w) * s,
                        (b.y + b.h) * s,
                        b.radius * s,
                        b.smooth,
                        b.blur * s,
                        b.saturate,
                        b.brightness,
                        (b.mask_kind != 0).then_some((b.mask_kind, b.mask)),
                    );
                }
            }
        }
        Ok(stack.into_iter().next().expect("base layer").pix)
    }
}

fn decode_runtime_image(image: &crate::render::RuntimeImage<'_>) -> Option<Pixmap> {
    if image.format == 0 {
        return decode_png(image.bytes).ok();
    }
    if image.format != 1 || image.width == 0 || image.height == 0 {
        return None;
    }
    let expected = usize::try_from(image.width)
        .ok()?
        .checked_mul(usize::try_from(image.height).ok()?)?
        .checked_mul(4)?;
    if image.bytes.len() != expected {
        return None;
    }
    let mut pixels = image.bytes.to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        pixel[0] = (u32::from(pixel[0]) * alpha / 255) as u8;
        pixel[1] = (u32::from(pixel[1]) * alpha / 255) as u8;
        pixel[2] = (u32::from(pixel[2]) * alpha / 255) as u8;
    }
    Pixmap::from_vec(pixels, IntSize::from_wh(image.width, image.height)?)
}

fn decode_png(bytes: &[u8]) -> Result<Pixmap, String> {
    Pixmap::decode_png(bytes).map_err(|e| e.to_string())
}

/// Render a Frame straight to PNG bytes.
///
/// `runtime_images` borrows the active unified-index payloads referenced by
/// `frame`; RGBA8 entries are premultiplied directly before compositing.
pub fn render_png(
    s: &Slir,
    images: &[Vec<u8>],
    runtime_images: &[crate::render::RuntimeImage<'_>],
    registered_fonts: &[RegisteredFont],
    frame: &Frame,
    scale: f64,
) -> Result<Vec<u8>, String> {
    let pix = Raster::new(s, images, runtime_images, registered_fonts, scale).render(frame)?;
    pix.encode_png().map_err(|e| e.to_string())
}

fn demultiply(pix: &Pixmap) -> Vec<u8> {
    pix.pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

/// Encode pre-rendered frames as APNG (acTL/fcTL/fdAT via the `png` crate).
pub fn encode_apng(frames: &[Pixmap], fps: f64, loops: u32) -> Result<Vec<u8>, String> {
    let first = frames.first().ok_or("no frames")?;
    let (w, h) = (first.width(), first.height());
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_animated(frames.len() as u32, loops)
            .map_err(|e| e.to_string())?;
        enc.set_frame_delay(1, fps.round().max(1.0) as u16)
            .map_err(|e| e.to_string())?;
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        for f in frames {
            if f.width() != w || f.height() != h {
                return Err("frame size mismatch".into());
            }
            writer
                .write_image_data(&demultiply(f))
                .map_err(|e| e.to_string())?;
        }
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_layer_center_uses_device_pixels() {
        let scale = slab_kernel::flatten::OpScale {
            cx: 12.0,
            cy: 7.5,
            sx: 0.5,
            sy: 1.25,
        };
        let LayerKind::Scale { cx, cy, sx, sy } = scale_layer_kind(&scale, 2.0) else {
            panic!("scale helper must create a scale layer");
        };
        assert_eq!((cx, cy), (24.0, 15.0));
        assert_eq!((sx, sy), (0.5, 1.25));
    }
}
