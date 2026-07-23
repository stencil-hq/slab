//! SVG exporter: kernel FrameOps mapped onto SVG elements (research svg.py
//! port onto the 1.0 Frame contract).
//!
//! Effects strategy (unchanged from research):
//! - gradients -> userSpaceOnUse <linearGradient>/<radialGradient> defs
//! - shadow runs -> one <filter> per op (offset/blur/flood merges; inset via
//!   inverted-alpha composite, painted above the fill)
//! - self blur -> <g filter=feGaussianBlur>
//! - Backdrop -> re-emit everything painted so far inside a clipped, blurred
//!   group (pure SVG, no backdrop-filter dependency)
//! - anim -> CSS @keyframes for the paint-level subset (opacity, offset),
//!   read from SLIR ANIM tables and correlated to op ranges via the scene
//!   (0.5's ANIM_PUSH/POP ops are gone).
//! - modern FX (contract v1): conic paints fan into 90 solid wedges; masks
//!   are luminance-safe white-alpha <mask> defs; grain is seeded
//!   feTurbulence; smooth swaps rects for squircle paths; backdrop-mask
//!   re-emits the backdrop in 3 alpha bands; tilt is an affine 3-corner fit.

use crate::render::RegisteredFont;
use slab_kernel::cells::rgba_lerp;
use slab_kernel::flatten::{Frame, FrameOp};
use slab_slir::{GradE, Slir};
use std::collections::{HashMap, HashSet};
use std::path::Path;

const FALLBACK: &str = "Helvetica, Arial, sans-serif";

/// Number formatting: two decimals, trailing zeros trimmed.
fn n(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" || s == "-" {
        "0".into()
    } else {
        s.into()
    }
}

/// Six decimals, trailing zeros trimmed — transform matrices and filter
/// scalars where two decimals round too coarsely.
fn n6(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" || s == "-" {
        "0".into()
    } else {
        s.into()
    }
}

/// `%g`-style formatting for CSS keyframes (shortest round-trip).
fn g(v: f64) -> String {
    format!("{v}")
}

/// `#rrggbb` / `#rrggbbaa` from an rgba8 word (r in the low byte).
fn hex(rgba: u32) -> String {
    let [r, gg, b, a] = rgba.to_le_bytes();
    if a == 255 {
        format!("#{r:02x}{gg:02x}{b:02x}")
    } else {
        format!("#{r:02x}{gg:02x}{b:02x}{a:02x}")
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn esc_attr(s: &str) -> String {
    esc(s).replace('"', "&quot;")
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let w = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(w >> 18) as usize & 63] as char);
        out.push(T[(w >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(w >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[w as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn path_d_data(verbs: &[u8], coords: &[f64]) -> String {
    let mut data = String::new();
    let mut coordinate = 0usize;
    for &verb in verbs {
        let take = |coordinate: &mut usize, count: usize| {
            let values = coords[*coordinate..*coordinate + count]
                .iter()
                .map(|&value| n(value))
                .collect::<Vec<_>>()
                .join(" ");
            *coordinate += count;
            values
        };
        match verb {
            0 => data.push_str(&format!("M{} ", take(&mut coordinate, 2))),
            1 => data.push_str(&format!("L{} ", take(&mut coordinate, 2))),
            2 => data.push_str(&format!("C{} ", take(&mut coordinate, 6))),
            3 => data.push_str(&format!("Q{} ", take(&mut coordinate, 4))),
            _ => data.push_str("Z "),
        }
    }
    data.trim_end().to_string()
}

/// Squircle outline `d` when smoothing applies (`smooth > 0`, `radius > 0`);
/// `None` keeps the caller's native rounded rect (SPEC §7 `smooth`).
fn sq_d(x: f64, y: f64, w: f64, h: f64, r: f64, smooth: f64) -> Option<String> {
    let r = r.min(w / 2.0).min(h / 2.0);
    if smooth <= 0.0 || r <= 0.0 || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let (verbs, mut coords) = slab_kernel::squircle::squircle_path(w, h, r, smooth);
    for (i, c) in coords.iter_mut().enumerate() {
        *c += if i % 2 == 0 { x } else { y };
    }
    Some(path_d_data(&verbs, &coords))
}

/// Rounded-rect outline `d` (clockwise, arc corners), plain rect at radius
/// zero. Used where one `<path>` must carry the shape (conic ring clips).
fn rrect_d(x: f64, y: f64, w: f64, h: f64, r: f64) -> String {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    if r <= 0.0 {
        return format!("M{} {} H{} V{} H{} Z", n(x), n(y), n(x + w), n(y + h), n(x));
    }
    let rr = n(r);
    format!(
        "M{} {} H{} A{rr} {rr} 0 0 1 {} {} V{} A{rr} {rr} 0 0 1 {} {} H{} A{rr} {rr} 0 0 1 {} {} V{} A{rr} {rr} 0 0 1 {} {} Z",
        n(x + r),
        n(y),
        n(x + w - r),
        n(x + w),
        n(y + r),
        n(y + h - r),
        n(x + w - r),
        n(y + h),
        n(x + r),
        n(x),
        n(y + h - r),
        n(y + r),
        n(x + r),
        n(y)
    )
}

/// Border-box outline: squircle when smoothing applies, else rounded rect.
fn outline_d(x: f64, y: f64, w: f64, h: f64, r: f64, smooth: f64) -> String {
    sq_d(x, y, w, h, r, smooth).unwrap_or_else(|| rrect_d(x, y, w, h, r))
}

/// ClipPath child for a rounded/smooth box: squircle path when smoothing
/// applies, else the native rounded rect.
fn clip_shape(x: f64, y: f64, w: f64, h: f64, r: f64, smooth: f64) -> String {
    if let Some(d) = sq_d(x, y, w, h, r, smooth) {
        return format!("<path d=\"{d}\"/>");
    }
    let rx = if r > 0.0 {
        format!(" rx=\"{}\"", n(r))
    } else {
        String::new()
    };
    format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{rx}/>",
        n(x),
        n(y),
        n(w),
        n(h)
    )
}

/// Samples a SLIR gradient ramp at `t` (clamped): per-channel sRGB lerp,
/// mirroring kernel `grad_sample` over the pool encoding.
fn ramp_sample(gr: &GradE, t: f64) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let Some(&(first_pos, first)) = gr.stops.first() else {
        return 0;
    };
    if t <= first_pos {
        return first;
    }
    for pair in gr.stops.windows(2) {
        let ((p0, c0), (p1, c1)) = (pair[0], pair[1]);
        if t <= p1 {
            let local = if p1 > p0 { (t - p0) / (p1 - p0) } else { 0.0 };
            return rgba_lerp(c0, c1, local);
        }
    }
    gr.stops[gr.stops.len() - 1].1
}

/// `<stop>` markup for a gradient def. `whiten` swaps every stop's RGB for
/// white keeping its alpha: a white-with-alpha ramp reads back as the
/// paint's alpha under both luminance and alpha masking (contract §6.3).
fn stops_markup(gr: &GradE, whiten: bool) -> String {
    gr.stops
        .iter()
        .map(|&(pos, rgba)| {
            let [r, gg, b, a] = rgba.to_le_bytes();
            let (r, gg, b) = if whiten { (255, 255, 255) } else { (r, gg, b) };
            let so = if a < 255 {
                format!(" stop-opacity=\"{}\"", n(f64::from(a) / 255.0))
            } else {
                String::new()
            };
            format!(
                "<stop offset=\"{}%\" stop-color=\"#{r:02x}{gg:02x}{b:02x}\"{so}/>",
                n(pos * 100.0)
            )
        })
        .collect()
}

/// 90-wedge conic fan (contract §6.1; SPEC chart: svg degraded): 4°
/// triangle wedges about the paint-box center, each filled with the ramp
/// sampled at its center angle (`t = (i + 0.5)/90`, clockwise from up,
/// offset by the paint's `from` angle). Wedges overlap their successor by
/// 0.25° to close antialiasing seams; `map` post-processes each sampled
/// color (identity, whiten-for-mask, band membership) and `None` skips the
/// wedge. `rad` must cover the clipped region (triangle chords reach
/// `1.01·rad·cos(2°) > rad`).
fn conic_fan(gr: &GradE, cx: f64, cy: f64, rad: f64, map: impl Fn(u32) -> Option<u32>) -> String {
    let rad = rad * 1.01;
    let mut out = String::new();
    for i in 0..90u32 {
        let Some(rgba) = map(ramp_sample(gr, (f64::from(i) + 0.5) / 90.0)) else {
            continue;
        };
        let a0 = (gr.angle + f64::from(i) * 4.0).to_radians();
        let a1 = (gr.angle + f64::from(i + 1) * 4.0 + 0.25).to_radians();
        out.push_str(&format!(
            "<path d=\"M{} {} L{} {} L{} {} Z\" fill=\"{}\"/>",
            n(cx),
            n(cy),
            n(cx + rad * a0.sin()),
            n(cy - rad * a0.cos()),
            n(cx + rad * a1.sin()),
            n(cy - rad * a1.cos()),
            hex(rgba)
        ));
    }
    out
}

/// Identity wedge map: keep visible samples as-is.
fn fan_opaque(c: u32) -> Option<u32> {
    (c >> 24 != 0).then_some(c)
}

/// Hard-stop white ramp for one progressive-blur band (contract §6.6): the
/// source ramp's ALPHA is sampled at 64 points; spans landing in `[lo, hi)`
/// (`hi` inclusive for the last band) become opaque white, the rest
/// transparent, with paired stops at span boundaries for hard transitions.
/// `None` = the band never turns on.
fn band_stops(gr: &GradE, lo: f64, hi: f64, last: bool) -> Option<String> {
    let on = |k: u32| {
        let a = f64::from(ramp_sample(gr, f64::from(k) / 63.0) >> 24) / 255.0;
        a >= lo && (a < hi || (last && a <= hi))
    };
    let stop = |pos: f64, on: bool| {
        format!(
            "<stop offset=\"{}%\" stop-color=\"#ffffff\"{}/>",
            n(pos * 100.0),
            if on { "" } else { " stop-opacity=\"0\"" }
        )
    };
    let mut prev = on(0);
    let mut any = prev;
    let mut stops = stop(0.0, prev);
    for k in 1..64u32 {
        let cur = on(k);
        if cur != prev {
            let boundary = (f64::from(k) - 0.5) / 63.0;
            stops.push_str(&stop(boundary, prev));
            stops.push_str(&stop(boundary, cur));
            prev = cur;
            any |= cur;
        }
    }
    any.then_some(stops)
}

/// Axis-aligned bounds of a path's coordinate stream (control points
/// included — conservative); `None` for an empty path. Gradient and conic
/// paints on paths map over this box (web painter convention).
fn coords_bbox(coords: &[f64]) -> Option<(f64, f64, f64, f64)> {
    let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
    let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for pair in coords.chunks_exact(2) {
        x0 = x0.min(pair[0]);
        x1 = x1.max(pair[0]);
        y0 = y0.min(pair[1]);
        y1 = y1.max(pair[1]);
    }
    x0.is_finite().then_some((x0, y0, x1 - x0, y1 - y0))
}

struct Emitter<'a> {
    s: &'a Slir,
    images: &'a [Vec<u8>],
    runtime_images: &'a [crate::render::RuntimeImage<'a>],
    frame: &'a Frame,
    base_dir: &'a Path,
    allow_backdrop: bool,
}

impl<'a> Emitter<'a> {
    fn uid(&self, defs: &[String], prefix: &str) -> String {
        format!("{prefix}{}", defs.len())
    }

    /// Emits a `<linearGradient>`/`<radialGradient>` def with the given stop
    /// markup mapped over the paint box; returns the `url(#…)` reference.
    /// Conic ramps (`kind == 2`) never land here — callers fan them out.
    #[allow(clippy::too_many_arguments)]
    fn grad_geom_def(
        &self,
        defs: &mut Vec<String>,
        gr: &GradE,
        stops: &str,
        x: f64,
        y: f64,
        w: f64,
        hh: f64,
    ) -> String {
        if gr.kind == 0 {
            let th = gr.angle.to_radians();
            let (dx, dy) = (th.sin(), -th.cos());
            let ln = (w * dx).abs() + (hh * dy).abs();
            let (cx, cy) = (x + w / 2.0, y + hh / 2.0);
            let gid = self.uid(defs, "lg");
            defs.push(format!(
                "<linearGradient id=\"{gid}\" gradientUnits=\"userSpaceOnUse\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\">{stops}</linearGradient>",
                n(cx - dx * ln / 2.0), n(cy - dy * ln / 2.0), n(cx + dx * ln / 2.0), n(cy + dy * ln / 2.0)
            ));
            format!("url(#{gid})")
        } else {
            let gid = self.uid(defs, "rg");
            let r = (w * w + hh * hh).sqrt() / 2.0;
            defs.push(format!(
                "<radialGradient id=\"{gid}\" gradientUnits=\"userSpaceOnUse\" cx=\"{}\" cy=\"{}\" r=\"{}\">{stops}</radialGradient>",
                n(x + w / 2.0), n(y + hh / 2.0), n(r)
            ));
            format!("url(#{gid})")
        }
    }

    /// fill= value for a Paint (kind, handle): solid hex or a gradient def.
    /// Conic degrades to its first stop here — every call site that can
    /// host a wedge fan (rect/path/text/mask) intercepts conic first.
    #[allow(clippy::too_many_arguments)]
    fn paint(
        &self,
        defs: &mut Vec<String>,
        kind: u32,
        h: u32,
        x: f64,
        y: f64,
        w: f64,
        hh: f64,
    ) -> String {
        match kind {
            1 => hex(h),
            2 => {
                let Some(gr) = self.s.grads.get(h as usize) else {
                    return "none".into();
                };
                if gr.kind == 2 {
                    return gr
                        .stops
                        .first()
                        .map_or_else(|| "none".into(), |&(_, rgba)| hex(rgba));
                }
                let stops = stops_markup(gr, false);
                self.grad_geom_def(defs, gr, &stops, x, y, w, hh)
            }
            _ => "none".into(),
        }
    }

    /// Stroke paint: solid hex or a real gradient url over the op box (the
    /// research-era first-stop degradation is gone — SPEC gradient row).
    /// Conic still returns its first stop: full-side non-dashed box conics
    /// take the ring-fan route in the Rect arm instead, path conics the
    /// stroke-mask route, leaving only dashed/per-side box degradations.
    #[allow(clippy::too_many_arguments)]
    fn stroke_paint(
        &self,
        defs: &mut Vec<String>,
        kind: u32,
        h: u32,
        x: f64,
        y: f64,
        w: f64,
        hh: f64,
    ) -> Option<String> {
        match kind {
            1 => Some(hex(h)),
            2 => {
                let gr = self.s.grads.get(h as usize)?;
                if gr.kind == 2 {
                    return gr.stops.first().map(|&(_, rgba)| hex(rgba));
                }
                let stops = stops_markup(gr, false);
                Some(self.grad_geom_def(defs, gr, &stops, x, y, w, hh))
            }
            _ => None,
        }
    }

    /// `<mask>` def cropped to a userSpaceOnUse box: content outside the
    /// box renders transparent black, so ink outside vanishes (contract
    /// §6.3).
    #[allow(clippy::too_many_arguments)]
    fn mask_def(
        &self,
        defs: &mut Vec<String>,
        prefix: &str,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        content: &str,
    ) -> String {
        let mid = self.uid(defs, prefix);
        defs.push(format!(
            "<mask id=\"{mid}\" maskUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\">{content}</mask>",
            n(x), n(y), n(w), n(h)
        ));
        mid
    }

    /// Mask content painting the paint's ALPHA as white over the box: a
    /// white-with-alpha ramp yields the paint's alpha under the default
    /// luminance interpretation (`luminance(#fff)·a == a`) without leaning
    /// on `mask-type:alpha` support (contract §6.3).
    #[allow(clippy::too_many_arguments)]
    fn white_mask_content(
        &self,
        defs: &mut Vec<String>,
        kind: u32,
        h: u32,
        x: f64,
        y: f64,
        w: f64,
        hh: f64,
    ) -> String {
        let box_rect = |fill: &str| {
            format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{fill}\"/>",
                n(x),
                n(y),
                n(w),
                n(hh)
            )
        };
        match kind {
            1 => box_rect(&hex(h | 0x00FF_FFFF)),
            2 => match self.s.grads.get(h as usize) {
                // conic: white wedges keep the sampled stop alpha; the mask
                // region crops them to the box
                Some(gr) if gr.kind == 2 => conic_fan(
                    gr,
                    x + w / 2.0,
                    y + hh / 2.0,
                    (w * w + hh * hh).sqrt() / 2.0,
                    |c| (c >> 24 != 0).then_some(c | 0x00FF_FFFF),
                ),
                Some(gr) => {
                    let stops = stops_markup(gr, true);
                    let url = self.grad_geom_def(defs, gr, &stops, x, y, w, hh);
                    box_rect(&url)
                }
                None => String::new(),
            },
            _ => String::new(),
        }
    }

    /// Mask content for one progressive-blur band: binary white where the
    /// paint's alpha lands in `[lo, hi)` over the backdrop box (contract
    /// §6.6). Linear/radial ramps quantize through `band_stops` (64 alpha
    /// samples → hard stops), conics per wedge (90 samples), solids
    /// all-or-nothing. `None` = the band never turns on and its whole
    /// re-emission is skipped.
    #[allow(clippy::too_many_arguments)]
    fn band_content(
        &self,
        defs: &mut Vec<String>,
        kind: u32,
        h: u32,
        lo: f64,
        hi: f64,
        last: bool,
        x: f64,
        y: f64,
        w: f64,
        hh: f64,
    ) -> Option<String> {
        let in_band = |a: f64| a >= lo && (a < hi || (last && a <= hi));
        let box_rect = |fill: &str| {
            format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{fill}\"/>",
                n(x),
                n(y),
                n(w),
                n(hh)
            )
        };
        match kind {
            1 => in_band(f64::from(h >> 24) / 255.0).then(|| box_rect("#ffffff")),
            2 => {
                let gr = self.s.grads.get(h as usize)?;
                if gr.kind == 2 {
                    let fan = conic_fan(
                        gr,
                        x + w / 2.0,
                        y + hh / 2.0,
                        (w * w + hh * hh).sqrt() / 2.0,
                        |c| in_band(f64::from(c >> 24) / 255.0).then_some(u32::MAX),
                    );
                    (!fan.is_empty()).then_some(fan)
                } else {
                    let stops = band_stops(gr, lo, hi, last)?;
                    let url = self.grad_geom_def(defs, gr, &stops, x, y, w, hh);
                    Some(box_rect(&url))
                }
            }
            _ => None,
        }
    }

    /// Backdrop filter def: gaussian blur plus optional saturation and
    /// brightness (linear RGB slope — backdrop RGB × brightness), shared by
    /// plain and banded backdrops.
    fn backdrop_filter(
        &self,
        defs: &mut Vec<String>,
        blur: f64,
        saturate: f64,
        brightness: f64,
    ) -> String {
        let sat = if saturate != 1.0 {
            format!(
                "<feColorMatrix type=\"saturate\" values=\"{}\"/>",
                n(saturate)
            )
        } else {
            String::new()
        };
        let bright = if brightness != 1.0 {
            let s = n6(brightness);
            format!(
                "<feComponentTransfer><feFuncR type=\"linear\" slope=\"{s}\"/>\
                 <feFuncG type=\"linear\" slope=\"{s}\"/><feFuncB type=\"linear\" slope=\"{s}\"/></feComponentTransfer>"
            )
        } else {
            String::new()
        };
        let fid = self.uid(defs, "bf");
        defs.push(format!(
            "<filter id=\"{fid}\" color-interpolation-filters=\"sRGB\" x=\"-20%\" y=\"-20%\" width=\"140%\" height=\"140%\">\
             <feGaussianBlur stdDeviation=\"{}\"/>{sat}{bright}</filter>",
            n(blur / 2.0)
        ));
        fid
    }

    /// Grain overlay (SPEC chart: svg degraded — feTurbulence realization
    /// of contract §6.2): fixed-seed fractal noise, grayscale speckle with
    /// alpha scaled by `grain_amount` and speckle size via `baseFrequency =
    /// 0.9/size`, cropped to the node's rounded/smooth silhouette by
    /// compositing into SourceGraphic.
    fn grain_el(
        &self,
        defs: &mut Vec<String>,
        r: &slab_kernel::flatten::OpRect,
        rr: f64,
    ) -> String {
        let fid = self.uid(defs, "gn");
        defs.push(format!(
            "<filter id=\"{fid}\" color-interpolation-filters=\"sRGB\">\
             <feTurbulence type=\"fractalNoise\" baseFrequency=\"{}\" numOctaves=\"2\" seed=\"7\" stitchTiles=\"stitch\"/>\
             <feColorMatrix type=\"matrix\" values=\"1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 0 0 0 {} 0\"/>\
             <feComposite in2=\"SourceGraphic\" operator=\"in\"/></filter>",
            n6(0.9 / r.grain_size.max(0.05)),
            n(r.grain_amount.clamp(0.0, 1.0))
        ));
        let o = if r.opacity < 1.0 {
            format!(" opacity=\"{}\"", n(r.opacity))
        } else {
            String::new()
        };
        if let Some(d) = sq_d(r.x, r.y, r.w, r.h, rr, r.smooth) {
            format!("<path d=\"{d}\" fill=\"#fff\" filter=\"url(#{fid})\"{o}/>")
        } else {
            let rx = if rr > 0.0 {
                format!(" rx=\"{}\"", n(rr))
            } else {
                String::new()
            };
            format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{rx} fill=\"#fff\" filter=\"url(#{fid})\"{o}/>",
                n(r.x),
                n(r.y),
                n(r.w),
                n(r.h)
            )
        }
    }

    /// Build the per-node shadow filter. `src_alpha` is the element's fill
    /// alpha: SourceAlpha is boosted back to full coverage so outer shadows
    /// keep CSS strength under translucent glass fills, and outer shadows are
    /// knocked out under the border-box (CSS box-shadow semantics — matches
    /// the raster and the web driver).
    fn shadow_filter(&self, defs: &mut Vec<String>, off: i32, len: i32, src_alpha: f64) -> String {
        let mut parts = String::new();
        let mut below = String::new();
        let mut above = String::new();
        let cover = if src_alpha < 1.0 {
            parts.push_str(&format!(
                "<feComponentTransfer in=\"SourceAlpha\" result=\"cov\">\
                 <feFuncA type=\"linear\" slope=\"{}\"/></feComponentTransfer>",
                n(1.0 / src_alpha.max(1.0 / 255.0))
            ));
            "cov"
        } else {
            "SourceAlpha"
        };
        for i in 0..len.max(0) as usize {
            let s = &self.s.shadows[off as usize + i];
            let std = n((s.blur / 2.0).max(0.01));
            let color = hex(s.rgba);
            if s.inset == 0 {
                parts.push_str(&format!(
                    "<feOffset in=\"{cover}\" dx=\"{}\" dy=\"{}\" result=\"o{i}\"/>\
                     <feGaussianBlur in=\"o{i}\" stdDeviation=\"{std}\" result=\"ob{i}\"/>\
                     <feFlood flood-color=\"{color}\" result=\"of{i}\"/>\
                     <feComposite in=\"of{i}\" in2=\"ob{i}\" operator=\"in\" result=\"os{i}\"/>\
                     <feComposite in=\"os{i}\" in2=\"{cover}\" operator=\"out\" result=\"ok{i}\"/>",
                    n(s.x),
                    n(s.y)
                ));
                below.push_str(&format!("<feMergeNode in=\"ok{i}\"/>"));
            } else {
                parts.push_str(&format!(
                    "<feComponentTransfer in=\"{cover}\" result=\"ia{i}\">\
                     <feFuncA type=\"table\" tableValues=\"1 0\"/></feComponentTransfer>\
                     <feOffset in=\"ia{i}\" dx=\"{}\" dy=\"{}\" result=\"io{i}\"/>\
                     <feGaussianBlur in=\"io{i}\" stdDeviation=\"{std}\" result=\"ib{i}\"/>\
                     <feFlood flood-color=\"{color}\" result=\"if{i}\"/>\
                     <feComposite in=\"if{i}\" in2=\"ib{i}\" operator=\"in\" result=\"ic{i}\"/>\
                     <feComposite in=\"ic{i}\" in2=\"{cover}\" operator=\"in\" result=\"is{i}\"/>",
                    n(s.x),
                    n(s.y)
                ));
                above.push_str(&format!("<feMergeNode in=\"is{i}\"/>"));
            }
        }
        let merge = format!("<feMerge>{below}<feMergeNode in=\"SourceGraphic\"/>{above}</feMerge>");
        let fid = self.uid(defs, "sh");
        defs.push(format!(
            "<filter id=\"{fid}\" color-interpolation-filters=\"sRGB\" x=\"-60%\" y=\"-60%\" width=\"220%\" height=\"220%\">{parts}{merge}</filter>"
        ));
        fid
    }

    /// Stroke as separate element(s) for align/sides control.
    fn stroke_elements(&self, r: &slab_kernel::flatten::OpRect, color: &str) -> Vec<String> {
        let hw = r.stroke_w / 2.0;
        let off = match r.stroke_align {
            1 => hw,  // inside
            2 => -hw, // outside
            _ => 0.0, // center
        };
        let (x0, y0) = (r.x + off, r.y + off);
        let (x1, y1) = (r.x + r.w - off, r.y + r.h - off);
        let dash = if r.has_dash {
            format!(" stroke-dasharray=\"{} {}\"", n(r.dash_on), n(r.dash_off))
        } else {
            String::new()
        };
        let base = format!(
            "stroke=\"{color}\" stroke-width=\"{}\"{dash} fill=\"none\"",
            n(r.stroke_w)
        );
        if r.stroke_sides == 15 {
            let rr = (r.radius - off)
                .min((x1 - x0) / 2.0)
                .min((y1 - y0) / 2.0)
                .max(0.0);
            if let Some(d) = sq_d(x0, y0, x1 - x0, y1 - y0, rr, r.smooth) {
                return vec![format!("<path d=\"{d}\" {base}/>")];
            }
            let rx = if rr > 0.0 {
                format!(" rx=\"{}\"", n(rr))
            } else {
                String::new()
            };
            return vec![format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{rx} {base}/>",
                n(x0),
                n(y0),
                n(x1 - x0),
                n(y1 - y0)
            )];
        }
        // t=1 r=2 b=4 l=8, emitted in research dict order t, b, l, r
        let segs: [(u32, (f64, f64, f64, f64)); 4] = [
            (1, (x0, y0, x1, y0)),
            (4, (x0, y1, x1, y1)),
            (8, (x0, y0, x0, y1)),
            (2, (x1, y0, x1, y1)),
        ];
        segs.iter()
            .filter(|(bit, _)| r.stroke_sides & bit != 0)
            .map(|&(_, (a, b, c, d))| {
                format!(
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" {base}/>",
                    n(a),
                    n(b),
                    n(c),
                    n(d)
                )
            })
            .collect()
    }

    fn placeholder(
        &self,
        out: &mut Vec<String>,
        depth: usize,
        im: &slab_kernel::flatten::OpImage,
        src: &str,
    ) {
        let pad = "  ".repeat(depth);
        let r = im.radius.min(im.w / 2.0).min(im.h / 2.0);
        out.push(format!(
            "{pad}<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#c9ced6\" rx=\"{}\"/>",
            n(im.x),
            n(im.y),
            n(im.w),
            n(im.h),
            n(r)
        ));
        out.push(format!(
            "{pad}<path d=\"M{} {} l{} {} M{} {} l{} {}\" stroke=\"#9aa1ab\" stroke-width=\"1\"/>",
            n(im.x),
            n(im.y),
            n(im.w),
            n(im.h),
            n(im.x + im.w),
            n(im.y),
            n(-im.w),
            n(im.h)
        ));
        let label = src
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("image");
        out.push(format!(
            "{pad}<text x=\"{}\" y=\"{}\" font-family=\"{FALLBACK}\" font-size=\"11\" fill=\"#5b6470\" text-anchor=\"middle\">{}</text>",
            n(im.x + im.w / 2.0), n(im.y + im.h / 2.0), esc(label)
        ));
    }

    #[allow(clippy::too_many_lines)]
    fn run(
        &self,
        defs: &mut Vec<String>,
        ops: &[FrameOp],
        anim_open: &HashMap<usize, Vec<String>>,
        anim_close: &HashMap<usize, usize>,
    ) -> Vec<String> {
        let mut body: Vec<String> = Vec::new();
        let mut depth = 1usize;
        for (idx, op) in ops.iter().enumerate() {
            if let Some(names) = anim_open.get(&idx) {
                for name in names {
                    body.push(format!("{}<g class=\"sa-{name}\">", "  ".repeat(depth)));
                    depth += 1;
                }
            }
            match op {
                FrameOp::Rect(r) => {
                    let pad = "  ".repeat(depth);
                    let rr = r.radius.min(r.w / 2.0).min(r.h / 2.0);
                    let bg_grad = (r.bg_kind == 2)
                        .then(|| self.s.grads.get(r.bg as usize))
                        .flatten();
                    let stroke_grad = (r.stroke_kind == 2)
                        .then(|| self.s.grads.get(r.stroke as usize))
                        .flatten();
                    let conic_ring = stroke_grad.is_some_and(|gr| gr.kind == 2)
                        && r.stroke_sides == 15
                        && !r.has_dash
                        && r.stroke_w > 0.0;
                    // solid fills report their true alpha; gradients/none keep 1.0
                    let src_alpha = if r.bg_kind == 1 {
                        f64::from((r.bg >> 24) & 0xFF) / 255.0 * r.opacity
                    } else {
                        r.opacity
                    };
                    let stroke = if conic_ring {
                        None
                    } else {
                        self.stroke_paint(defs, r.stroke_kind, r.stroke, r.x, r.y, r.w, r.h)
                    };
                    let mut simple = false;
                    if let Some(gr) = bg_grad.filter(|gr| gr.kind == 2) {
                        // Conic bg: wedge fan clipped to the silhouette. The
                        // shadow filter wraps the CLIPPED fan so SourceAlpha
                        // keeps the silhouette without the clip cutting the
                        // outer shadow off.
                        let cid = self.uid(defs, "fc");
                        defs.push(format!(
                            "<clipPath id=\"{cid}\">{}</clipPath>",
                            clip_shape(r.x, r.y, r.w, r.h, rr, r.smooth)
                        ));
                        let fan = conic_fan(
                            gr,
                            r.x + r.w / 2.0,
                            r.y + r.h / 2.0,
                            (r.w * r.w + r.h * r.h).sqrt() / 2.0,
                            fan_opaque,
                        );
                        let mut wrap: Vec<String> = Vec::new();
                        if r.shadow_len > 0 {
                            let fid =
                                self.shadow_filter(defs, r.shadow_off, r.shadow_len, src_alpha);
                            wrap.push(format!("filter=\"url(#{fid})\""));
                        }
                        if r.opacity < 1.0 {
                            wrap.push(format!("opacity=\"{}\"", n(r.opacity)));
                        }
                        if wrap.is_empty() {
                            body.push(format!("{pad}<g clip-path=\"url(#{cid})\">{fan}</g>"));
                        } else {
                            body.push(format!(
                                "{pad}<g {}><g clip-path=\"url(#{cid})\">{fan}</g></g>",
                                wrap.join(" ")
                            ));
                        }
                    } else {
                        let sqd = sq_d(r.x, r.y, r.w, r.h, rr, r.smooth);
                        let mut attrs = if let Some(dd) = &sqd {
                            vec![format!("d=\"{dd}\"")]
                        } else {
                            vec![format!(
                                "x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
                                n(r.x),
                                n(r.y),
                                n(r.w),
                                n(r.h)
                            )]
                        };
                        if sqd.is_none() && rr > 0.0 {
                            attrs.push(format!("rx=\"{}\"", n(rr)));
                        }
                        let fill = self.paint(defs, r.bg_kind, r.bg, r.x, r.y, r.w, r.h);
                        attrs.push(format!("fill=\"{fill}\""));
                        simple = stroke.is_some() && r.stroke_align == 0 && r.stroke_sides == 15;
                        if simple {
                            attrs.push(format!(
                                "stroke=\"{}\" stroke-width=\"{}\"",
                                stroke.clone().unwrap(),
                                n(r.stroke_w)
                            ));
                            if r.has_dash {
                                attrs.push(format!(
                                    "stroke-dasharray=\"{} {}\"",
                                    n(r.dash_on),
                                    n(r.dash_off)
                                ));
                            }
                        }
                        if r.shadow_len > 0 {
                            let fid =
                                self.shadow_filter(defs, r.shadow_off, r.shadow_len, src_alpha);
                            attrs.push(format!("filter=\"url(#{fid})\""));
                        }
                        if r.opacity < 1.0 {
                            attrs.push(format!("opacity=\"{}\"", n(r.opacity)));
                        }
                        let tag = if sqd.is_some() { "path" } else { "rect" };
                        body.push(format!("{pad}<{tag} {}/>", attrs.join(" ")));
                    }
                    if r.grain_amount > 0.0 {
                        body.push(format!("{pad}{}", self.grain_el(defs, r, rr)));
                    }
                    if let Some(gr) = stroke_grad.filter(|_| conic_ring) {
                        // Conic box stroke: fan clipped to the ring between
                        // the stroke's outer and inner rounded/smooth
                        // outlines (clip-rule evenodd).
                        let outset = match r.stroke_align {
                            1 => 0.0,        // inside
                            2 => r.stroke_w, // outside
                            _ => r.stroke_w / 2.0,
                        };
                        let inset = r.stroke_w - outset;
                        let (ow, oh) = (r.w + 2.0 * outset, r.h + 2.0 * outset);
                        let ro = (rr + outset).min(ow / 2.0).min(oh / 2.0).max(0.0);
                        let mut dd = outline_d(r.x - outset, r.y - outset, ow, oh, ro, r.smooth);
                        let (iw, ih) = (r.w - 2.0 * inset, r.h - 2.0 * inset);
                        if iw > 0.0 && ih > 0.0 {
                            let ri = (rr - inset).min(iw / 2.0).min(ih / 2.0).max(0.0);
                            dd.push(' ');
                            dd.push_str(&outline_d(r.x + inset, r.y + inset, iw, ih, ri, r.smooth));
                        }
                        let cid = self.uid(defs, "rc");
                        defs.push(format!(
                            "<clipPath id=\"{cid}\"><path d=\"{dd}\" clip-rule=\"evenodd\"/></clipPath>"
                        ));
                        let fan = conic_fan(
                            gr,
                            r.x + r.w / 2.0,
                            r.y + r.h / 2.0,
                            (ow * ow + oh * oh).sqrt() / 2.0,
                            fan_opaque,
                        );
                        let o = if r.opacity < 1.0 {
                            format!(" opacity=\"{}\"", n(r.opacity))
                        } else {
                            String::new()
                        };
                        body.push(format!("{pad}<g clip-path=\"url(#{cid})\"{o}>{fan}</g>"));
                    } else if let Some(color) = stroke
                        && !simple
                    {
                        for el in self.stroke_elements(r, &color) {
                            body.push(format!("{pad}{el}"));
                        }
                    }
                }
                FrameOp::Text(t) => {
                    let text = self
                        .frame
                        .strings
                        .get(t.str_ref as usize)
                        .map(String::as_str)
                        .unwrap_or("");
                    let fam = match self.s.fonts.get(t.font.max(0) as usize) {
                        Some(f) if t.font >= 0 => {
                            format!("{}, {FALLBACK}", self.s.str_at(f.family))
                        }
                        _ => FALLBACK.into(),
                    };
                    let grad = (t.color_kind == 2)
                        .then(|| self.s.grads.get(t.color as usize))
                        .flatten();
                    let conic = grad.filter(|gr| gr.kind == 2);
                    // Gradient text (contract §6.7): ink maps over the
                    // kernel-provided node content box, sharing one ramp
                    // across lines. Conic routes through a white glyph mask
                    // plus a wedge fan (a fan cannot be a paint server).
                    let fill = if conic.is_some() {
                        "#fff".into()
                    } else if t.color_kind == 2 {
                        self.paint(defs, 2, t.color, t.gx, t.gy, t.gw, t.gh)
                    } else {
                        hex(t.color)
                    };
                    let mut attrs = vec![
                        format!("x=\"{}\" y=\"{}\"", n(t.x), n(t.y_baseline)),
                        format!(
                            "font-family=\"{}\" font-size=\"{}\"",
                            esc_attr(&fam),
                            n(t.size)
                        ),
                        format!("font-weight=\"{}\" fill=\"{fill}\"", t.weight),
                        "xml:space=\"preserve\"".into(),
                    ];
                    if t.tracking != 0.0 {
                        attrs.push(format!("letter-spacing=\"{}\"", n(t.tracking)));
                    }
                    if t.measured_w > 0.0 && text.chars().count() > 1 {
                        attrs.push(format!(
                            "textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\"",
                            n(t.measured_w)
                        ));
                    }
                    if let Some(gr) = conic {
                        let el = format!("<text {}>{}</text>", attrs.join(" "), esc(text));
                        let s = t.size;
                        let (mw, mh) = (t.gw + 2.0 * s, t.gh + 2.0 * s);
                        let mid = self.mask_def(defs, "tm", t.gx - s, t.gy - s, mw, mh, &el);
                        let fan = conic_fan(
                            gr,
                            t.gx + t.gw / 2.0,
                            t.gy + t.gh / 2.0,
                            (mw * mw + mh * mh).sqrt() / 2.0,
                            fan_opaque,
                        );
                        let o = if t.opacity < 1.0 {
                            format!(" opacity=\"{}\"", n(t.opacity))
                        } else {
                            String::new()
                        };
                        body.push(format!(
                            "{}<g mask=\"url(#{mid})\"{o}>{fan}</g>",
                            "  ".repeat(depth)
                        ));
                    } else {
                        if t.opacity < 1.0 {
                            attrs.push(format!("opacity=\"{}\"", n(t.opacity)));
                        }
                        body.push(format!(
                            "{}<text {}>{}</text>",
                            "  ".repeat(depth),
                            attrs.join(" "),
                            esc(text)
                        ));
                    }
                }
                FrameOp::Image(im) => {
                    let compiled = self
                        .s
                        .images
                        .get(im.img.max(0) as usize)
                        .filter(|_| im.img >= 0);
                    let src = compiled.map(|image| self.s.str_at(image.src)).unwrap_or("");
                    let runtime = self
                        .runtime_images
                        .iter()
                        .rfind(|image| image.image == im.img);
                    let href = if let Some(image) = runtime {
                        crate::render::runtime_image_png(image)
                            .map(|png| format!("data:image/png;base64,{}", b64(png.as_ref())))
                    } else if let Some(bytes) = self
                        .images
                        .get(im.img.max(0) as usize)
                        .filter(|_| im.img >= 0)
                        .filter(|bytes| !bytes.is_empty())
                    {
                        Some(format!("data:image/png;base64,{}", b64(bytes)))
                    } else if src.starts_with("http://")
                        || src.starts_with("https://")
                        || src.starts_with("data:")
                    {
                        Some(src.to_string())
                    } else {
                        let resolved = if Path::new(src).is_absolute() {
                            std::path::PathBuf::from(src)
                        } else {
                            self.base_dir.join(src)
                        };
                        if src.is_empty() || !resolved.exists() {
                            None
                        } else {
                            Some(
                                resolved
                                    .canonicalize()
                                    .unwrap_or(resolved)
                                    .display()
                                    .to_string(),
                            )
                        }
                    };
                    let Some(href) = href else {
                        self.placeholder(&mut body, depth, im, src);
                        if let Some(close) = anim_close.get(&idx) {
                            for _ in 0..*close {
                                depth -= 1;
                                body.push(format!("{}</g>", "  ".repeat(depth)));
                            }
                        }
                        continue;
                    };
                    let par = match im.fit {
                        1 => "xMidYMid meet",
                        2 => "none",
                        _ => "xMidYMid slice",
                    };
                    let mut clip = String::new();
                    if im.radius > 0.0 || im.fit == 0 {
                        let cid = self.uid(defs, "imc");
                        let r = im.radius.min(im.w / 2.0).min(im.h / 2.0);
                        defs.push(format!(
                            "<clipPath id=\"{cid}\">{}</clipPath>",
                            clip_shape(im.x, im.y, im.w, im.h, r, im.smooth)
                        ));
                        clip = format!(" clip-path=\"url(#{cid})\"");
                    }
                    let o = if im.opacity < 1.0 {
                        format!(" opacity=\"{}\"", n(im.opacity))
                    } else {
                        String::new()
                    };
                    body.push(format!(
                        "{}<image href=\"{}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"{par}\"{clip}{o}/>",
                        "  ".repeat(depth), esc_attr(&href), n(im.x), n(im.y), n(im.w), n(im.h)
                    ));
                }
                FrameOp::PathDraw(p) => {
                    let geom = if p.path >= 0 {
                        self.s
                            .paths
                            .get(p.path as usize)
                            .map(|pe| (pe.verbs.as_slice(), pe.coords.as_slice()))
                    } else {
                        self.frame
                            .paths_rt
                            .get((!p.path) as usize)
                            .map(|pe| (pe.verbs.as_slice(), pe.coords.as_slice()))
                    };
                    let Some((verbs, coords)) = geom else {
                        continue;
                    };
                    let data = path_d_data(verbs, coords);
                    // Gradient geometry maps over the path-local coordinate
                    // bbox (the element transform carries userSpaceOnUse
                    // paints along; the web painter shares this convention).
                    // Fans and masks are page-space siblings, so they add
                    // the translate themselves.
                    let bbox = coords_bbox(coords);
                    let xf_attr = (p.dx != 0.0 || p.dy != 0.0)
                        .then(|| format!("transform=\"translate({} {})\"", n(p.dx), n(p.dy)));
                    let xform = xf_attr
                        .as_ref()
                        .map(|a| format!(" {a}"))
                        .unwrap_or_default();
                    let bg_grad = (p.bg_kind == 2)
                        .then(|| self.s.grads.get(p.bg as usize))
                        .flatten();
                    let stroke_grad = (p.stroke_kind == 2)
                        .then(|| self.s.grads.get(p.stroke as usize))
                        .flatten();
                    let conic_bg = bg_grad.filter(|gr| gr.kind == 2).zip(bbox);
                    let conic_stroke = stroke_grad
                        .filter(|gr| gr.kind == 2)
                        .zip(bbox)
                        .filter(|_| p.stroke_w > 0.0);
                    let (bx, by, bw, bh) = bbox.unwrap_or((0.0, 0.0, 0.0, 0.0));
                    let mut attrs = vec![format!("d=\"{data}\"")];
                    if let Some(a) = &xf_attr {
                        attrs.push(a.clone());
                    }
                    let fill = if conic_bg.is_some() {
                        "none".into()
                    } else {
                        self.paint(defs, p.bg_kind, p.bg, bx, by, bw, bh)
                    };
                    attrs.push(format!("fill=\"{fill}\""));
                    let cap = if p.has_dash { "butt" } else { "round" };
                    let stroke_base = |color: &str| {
                        let mut s = format!(
                            "stroke=\"{color}\" stroke-width=\"{}\" stroke-linecap=\"{cap}\" stroke-linejoin=\"round\"",
                            n(p.stroke_w)
                        );
                        if p.has_dash {
                            s.push_str(&format!(
                                " stroke-dasharray=\"{} {}\"",
                                n(p.dash_on),
                                n(p.dash_off)
                            ));
                        }
                        s
                    };
                    if conic_stroke.is_none()
                        && let Some(color) =
                            self.stroke_paint(defs, p.stroke_kind, p.stroke, bx, by, bw, bh)
                    {
                        attrs.push(stroke_base(&color));
                    }
                    let o = if p.opacity < 1.0 {
                        format!(" opacity=\"{}\"", n(p.opacity))
                    } else {
                        String::new()
                    };
                    if p.opacity < 1.0 {
                        attrs.push(format!("opacity=\"{}\"", n(p.opacity)));
                    }
                    body.push(format!("{}<path {}/>", "  ".repeat(depth), attrs.join(" ")));
                    if let Some((gr, (x, y, w, h))) = conic_bg {
                        // conic path fill: fan clipped to the path silhouette
                        let cid = self.uid(defs, "fc");
                        defs.push(format!(
                            "<clipPath id=\"{cid}\"><path d=\"{data}\"{xform}/></clipPath>"
                        ));
                        let fan = conic_fan(
                            gr,
                            x + p.dx + w / 2.0,
                            y + p.dy + h / 2.0,
                            (w * w + h * h).sqrt() / 2.0,
                            fan_opaque,
                        );
                        body.push(format!(
                            "{}<g clip-path=\"url(#{cid})\"{o}>{fan}</g>",
                            "  ".repeat(depth)
                        ));
                    }
                    if let Some((gr, (x, y, w, h))) = conic_stroke {
                        // conic path stroke: fan masked by the white-stroked
                        // outline (dashes ride the mask copy)
                        let content = format!(
                            "<path d=\"{data}\"{xform} fill=\"none\" {}/>",
                            stroke_base("#fff")
                        );
                        let (mw, mh) = (w + 2.0 * p.stroke_w, h + 2.0 * p.stroke_w);
                        let mid = self.mask_def(
                            defs,
                            "pm",
                            x + p.dx - p.stroke_w,
                            y + p.dy - p.stroke_w,
                            mw,
                            mh,
                            &content,
                        );
                        let fan = conic_fan(
                            gr,
                            x + p.dx + w / 2.0,
                            y + p.dy + h / 2.0,
                            (mw * mw + mh * mh).sqrt() / 2.0,
                            fan_opaque,
                        );
                        body.push(format!(
                            "{}<g mask=\"url(#{mid})\"{o}>{fan}</g>",
                            "  ".repeat(depth)
                        ));
                    }
                }
                FrameOp::Backdrop(b) => {
                    if !self.allow_backdrop {
                        continue;
                    }
                    let sub = Emitter {
                        allow_backdrop: false,
                        ..*self
                    };
                    let fragment = sub.run(defs, &ops[..idx], anim_open, anim_close);
                    let cid = self.uid(defs, "bc");
                    let r = b.radius.min(b.w / 2.0).min(b.h / 2.0);
                    defs.push(format!(
                        "<clipPath id=\"{cid}\">{}</clipPath>",
                        clip_shape(b.x, b.y, b.w, b.h, r, b.smooth)
                    ));
                    if b.mask_kind == 0 {
                        let fid = self.backdrop_filter(defs, b.blur, b.saturate, b.brightness);
                        // clip OUTSIDE the filter group: blur first, hard edge after
                        body.push(format!(
                            "{}<g clip-path=\"url(#{cid})\"><g filter=\"url(#{fid})\">",
                            "  ".repeat(depth)
                        ));
                        body.extend(fragment.into_iter().map(|ln| format!("  {ln}")));
                        body.push(format!("{}</g></g>", "  ".repeat(depth)));
                    } else {
                        // W9 progressive blur (contract §6.6): 3 banded
                        // re-emissions, band i keeping mask-alpha in
                        // [i/3, (i+1)/3) at strength α = (i+0.5)/3 with
                        // blur·α and saturate/brightness lerped to identity.
                        for band in 0..3u32 {
                            let (lo, hi) = (f64::from(band) / 3.0, f64::from(band + 1) / 3.0);
                            let Some(content) = self.band_content(
                                defs,
                                b.mask_kind,
                                b.mask,
                                lo,
                                hi,
                                band == 2,
                                b.x,
                                b.y,
                                b.w,
                                b.h,
                            ) else {
                                continue;
                            };
                            let alpha = (f64::from(band) + 0.5) / 3.0;
                            let fid = self.backdrop_filter(
                                defs,
                                b.blur * alpha,
                                1.0 + (b.saturate - 1.0) * alpha,
                                1.0 + (b.brightness - 1.0) * alpha,
                            );
                            let mid = self.mask_def(defs, "bm", b.x, b.y, b.w, b.h, &content);
                            body.push(format!(
                                "{}<g mask=\"url(#{mid})\"><g clip-path=\"url(#{cid})\"><g filter=\"url(#{fid})\">",
                                "  ".repeat(depth)
                            ));
                            body.extend(fragment.iter().map(|ln| format!("  {ln}")));
                            body.push(format!("{}</g></g></g>", "  ".repeat(depth)));
                        }
                    }
                }
                FrameOp::ClipPush(c) => {
                    let cid = self.uid(defs, "c");
                    let r = c.radius.min(c.w / 2.0).min(c.h / 2.0);
                    defs.push(format!(
                        "<clipPath id=\"{cid}\">{}</clipPath>",
                        clip_shape(c.x, c.y, c.w, c.h, r, c.smooth)
                    ));
                    body.push(format!(
                        "{}<g clip-path=\"url(#{cid})\">",
                        "  ".repeat(depth)
                    ));
                    depth += 1;
                }
                FrameOp::ClipPop
                | FrameOp::GroupPop
                | FrameOp::RotatePop
                | FrameOp::ScalePop
                | FrameOp::TiltPop => {
                    depth = depth.saturating_sub(1).max(1);
                    body.push(format!("{}</g>", "  ".repeat(depth)));
                }
                FrameOp::GroupPush(gp) => {
                    let mut attrs: Vec<String> = Vec::new();
                    if gp.opacity < 1.0 {
                        attrs.push(format!("opacity=\"{}\"", n(gp.opacity)));
                    }
                    if gp.blur > 0.0 {
                        let fid = self.uid(defs, "gb");
                        defs.push(format!(
                            "<filter id=\"{fid}\" color-interpolation-filters=\"sRGB\" x=\"-40%\" y=\"-40%\" width=\"180%\" height=\"180%\">\
                             <feGaussianBlur stdDeviation=\"{}\"/></filter>",
                            n(gp.blur / 2.0)
                        ));
                        attrs.push(format!("filter=\"url(#{fid})\""));
                    }
                    if gp.mask_kind != 0 {
                        // W7: subtree alpha × the paint's alpha over the mask
                        // box (contract §6.3); the userSpaceOnUse region
                        // hides ink outside the box.
                        let content = self.white_mask_content(
                            defs,
                            gp.mask_kind,
                            gp.mask,
                            gp.mx,
                            gp.my,
                            gp.mw,
                            gp.mh,
                        );
                        let mid = self.mask_def(defs, "mk", gp.mx, gp.my, gp.mw, gp.mh, &content);
                        attrs.push(format!("mask=\"url(#{mid})\""));
                    }
                    let pad = "  ".repeat(depth);
                    if attrs.is_empty() {
                        body.push(format!("{pad}<g>"));
                    } else {
                        body.push(format!("{pad}<g {}>", attrs.join(" ")));
                    }
                    depth += 1;
                }
                FrameOp::RotatePush(rt) => {
                    body.push(format!(
                        "{}<g transform=\"rotate({} {} {})\">",
                        "  ".repeat(depth),
                        n(rt.deg),
                        n(rt.cx),
                        n(rt.cy)
                    ));
                    depth += 1;
                }
                FrameOp::ScalePush(scale) => {
                    body.push(format!(
                        "{}<g transform=\"translate({} {}) scale({} {}) translate({} {})\">",
                        "  ".repeat(depth),
                        n(scale.cx),
                        n(scale.cy),
                        n(scale.sx),
                        n(scale.sy),
                        n(-scale.cx),
                        n(-scale.cy),
                    ));
                    depth += 1;
                }
                FrameOp::TiltPush(tp) => {
                    // contract §6.5 perspective projected at three reference
                    // points → affine fit (SPEC chart: svg degraded — no
                    // foreshortening inside the subtree).
                    let proj = |px: f64, py: f64| {
                        let (x, y) = (px - tp.cx, py - tp.cy);
                        let (sry, cry) = tp.ry.to_radians().sin_cos();
                        let (srx, crx) = tp.rx.to_radians().sin_cos();
                        let z1 = -x * sry;
                        let y2 = y * crx - z1 * srx;
                        let z2 = y * srx + z1 * crx;
                        let zc = z2.min(0.95 * tp.depth);
                        let s = tp.depth / (tp.depth - zc).max(1e-6);
                        (tp.cx + x * cry * s, tp.cy + y2 * s)
                    };
                    let (p0x, p0y) = proj(tp.cx, tp.cy);
                    let (p1x, p1y) = proj(tp.cx + 100.0, tp.cy);
                    let (p2x, p2y) = proj(tp.cx, tp.cy + 100.0);
                    let mut m = [
                        (p1x - p0x) / 100.0,
                        (p1y - p0y) / 100.0,
                        (p2x - p0x) / 100.0,
                        (p2y - p0y) / 100.0,
                        0.0,
                        0.0,
                    ];
                    m[4] = p0x - m[0] * tp.cx - m[2] * tp.cy;
                    m[5] = p0y - m[1] * tp.cx - m[3] * tp.cy;
                    let mat = m.iter().map(|&v| n6(v)).collect::<Vec<_>>().join(" ");
                    body.push(format!(
                        "{}<g transform=\"matrix({mat})\">",
                        "  ".repeat(depth)
                    ));
                    depth += 1;
                }
            }
            if let Some(close) = anim_close.get(&idx) {
                for _ in 0..*close {
                    depth -= 1;
                    body.push(format!("{}</g>", "  ".repeat(depth)));
                }
            }
        }
        while depth > 1 {
            // balance (only reachable via sliced backdrop fragments)
            depth -= 1;
            body.push(format!("{}</g>", "  ".repeat(depth)));
        }
        body
    }
}

/// ANIM bindings correlated to contiguous op ranges via the scene: the range
/// covers every op whose node lies in the bound node's subtree, extended over
/// enclosing push/pop wrappers of that same range.
fn anim_ranges(s: &Slir, frame: &Frame) -> Vec<(usize, usize, usize)> {
    if s.bindings.is_empty() {
        return Vec::new();
    }
    // matching push/pop pairs
    let mut pair: HashMap<usize, usize> = HashMap::new();
    let mut stack: Vec<usize> = Vec::new();
    for (i, op) in frame.ops.iter().enumerate() {
        match op {
            FrameOp::ClipPush(_)
            | FrameOp::GroupPush(_)
            | FrameOp::RotatePush(_)
            | FrameOp::ScalePush(_)
            | FrameOp::TiltPush(_) => stack.push(i),
            FrameOp::ClipPop
            | FrameOp::GroupPop
            | FrameOp::RotatePop
            | FrameOp::ScalePop
            | FrameOp::TiltPop => {
                if let Some(open) = stack.pop() {
                    pair.insert(open, i);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for (bi, b) in s.bindings.iter().enumerate() {
        // subtree node set from the scene
        let mut in_sub = vec![false; frame.scene.len()];
        for (si, sn) in frame.scene.iter().enumerate() {
            in_sub[si] = sn.node == b.node || (sn.parent_ix >= 0 && in_sub[sn.parent_ix as usize]);
        }
        let nodes: std::collections::HashSet<u32> = frame
            .scene
            .iter()
            .zip(&in_sub)
            .filter(|&(_, &s)| s)
            .map(|(sn, _)| sn.node)
            .collect();
        let op_node = |op: &FrameOp| -> Option<u32> {
            match op {
                FrameOp::Rect(r) => Some(r.node),
                FrameOp::Text(t) => Some(t.node),
                FrameOp::Image(im) => Some(im.node),
                FrameOp::PathDraw(p) => Some(p.node),
                _ => None,
            }
        };
        let mut i0 = None;
        let mut i1 = 0usize;
        for (i, op) in frame.ops.iter().enumerate() {
            if op_node(op).is_some_and(|nd| nodes.contains(&nd)) {
                i0.get_or_insert(i);
                i1 = i;
            }
        }
        let Some(mut i0) = i0 else { continue };
        while i0 > 0 && pair.get(&(i0 - 1)) == Some(&(i1 + 1)) {
            i0 -= 1;
            i1 += 1;
        }
        out.push((i0, i1, bi));
    }
    out
}

/// CSS export for the paint-level subset: opacity and offset (translate).
fn css_keyframes(s: &Slir, used: &[usize]) -> String {
    let mut rules: Vec<String> = Vec::new();
    // research parity: one rule per anim NAME; the last binding wins the
    // animation shorthand (used_anims[name] = op)
    let mut order: Vec<&str> = Vec::new();
    let mut by_name: HashMap<&str, usize> = HashMap::new();
    for &bi in used {
        let b = &s.bindings[bi];
        let Some(anim) = s.anims.get(b.anim as usize) else {
            continue;
        };
        let name = s.str_at(anim.name);
        if !by_name.contains_key(name) {
            order.push(name);
        }
        by_name.insert(name, bi);
    }
    for name in order {
        let bi = by_name[name];
        let b = &s.bindings[bi];
        let Some(anim) = s.anims.get(b.anim as usize) else {
            continue;
        };
        let name = s.str_at(anim.name);
        let mut stops: Vec<String> = Vec::new();
        for &(pos, aoff, alen) in &anim.stops {
            let mut props: Vec<String> = Vec::new();
            for k in 0..alen as usize {
                let (attr, aval) = s.anim_attrs[aoff as usize + k];
                let v = s.avals[aval as usize];
                if attr == slab_slir::attrs::OPACITY && v.tag == slab_slir::aval::NUM {
                    props.push(format!("opacity:{}", g(v.as_f64())));
                }
                if attr == slab_slir::attrs::OFFSET
                    && v.tag == slab_slir::aval::TUPLE
                    && v.hi() == 2
                {
                    let x = s.f64s[v.lo() as usize];
                    let y = s.f64s[v.lo() as usize + 1];
                    props.push(format!("transform:translate({}px,{}px)", g(x), g(y)));
                }
            }
            if !props.is_empty() {
                stops.push(format!("{}%{{{}}}", g(pos * 100.0), props.join(";")));
            }
        }
        if stops.is_empty() {
            continue;
        }
        let count = if b.mode == 1 { "1" } else { "infinite" };
        let direction = if b.mode == 2 { " alternate" } else { "" };
        let fill = if b.mode == 1 { " forwards" } else { "" };
        let easing = match b.easing {
            1 => "ease-in",
            2 => "ease-out",
            3 => "ease-in-out",
            _ => "linear",
        };
        rules.push(format!(
            "@keyframes {name}{{{}}}\n.sa-{name}{{animation:{name} {}ms {easing} {}ms {count}{direction}{fill}}}",
            stops.join(""),
            g(b.dur),
            g(b.delay)
        ));
    }
    rules.join("\n")
}

/// `@font-face` rules for every non-default face referenced by a Text op.
fn font_faces(s: &Slir, frame: &Frame, registered_fonts: &[RegisteredFont]) -> String {
    let mut seen = HashSet::new();
    let mut css = String::new();
    for op in &frame.ops {
        let FrameOp::Text(t) = op else {
            continue;
        };
        let font_ix = t.font;
        let Some(font) = s
            .fonts
            .get(font_ix.max(0) as usize)
            .filter(|_| font_ix >= 0)
        else {
            continue;
        };
        if font.family == 0 || !seen.insert(font_ix) {
            continue;
        }
        let bytes = registered_fonts
            .iter()
            .enumerate()
            .filter(|(_, registered)| registered.name.eq_ignore_ascii_case(s.str_at(font.family)))
            .min_by_key(|(index, registered)| {
                (
                    registered.metrics.weight.abs_diff(font.weight),
                    usize::MAX - *index,
                )
            })
            .map(|(_, registered)| registered.bytes.as_slice())
            .unwrap_or_else(|| slab_fonts::asset(font.class, font.weight).bytes);
        css.push_str(&format!(
            "@font-face{{font-family:\"{}\";font-weight:{};\
             src:url(data:font/ttf;base64,{}) format(\"truetype\");}}",
            s.str_at(font.family),
            font.weight,
            b64(bytes)
        ));
    }
    css
}

/// Render a solved Frame to a standalone SVG document.
///
/// `runtime_images` borrows active unified-index payloads; RGBA8 entries are
/// encoded as embedded PNG data URLs.
pub fn render_svg(
    s: &Slir,
    images: &[Vec<u8>],
    runtime_images: &[crate::render::RuntimeImage<'_>],
    registered_fonts: &[RegisteredFont],
    frame: &Frame,
    base_dir: &Path,
) -> String {
    let mut defs: Vec<String> = Vec::new();
    let ranges = anim_ranges(s, frame);
    let mut anim_open: HashMap<usize, Vec<String>> = HashMap::new();
    let mut anim_close: HashMap<usize, usize> = HashMap::new();
    let mut used: Vec<usize> = Vec::new();
    for &(i0, i1, bi) in &ranges {
        let name = s
            .str_at(s.anims[s.bindings[bi].anim as usize].name)
            .to_string();
        anim_open.entry(i0).or_default().push(name);
        *anim_close.entry(i1).or_default() += 1;
        used.push(bi);
    }
    let em = Emitter {
        s,
        images,
        runtime_images,
        frame,
        base_dir,
        allow_backdrop: true,
    };
    let body = em.run(&mut defs, &frame.ops, &anim_open, &anim_close);
    let faces = font_faces(s, frame, registered_fonts);
    if !faces.is_empty() {
        defs.insert(0, format!("<style>{faces}</style>"));
    }
    let mut out = vec![format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        n(frame.width),
        n(frame.height),
        n(frame.width),
        n(frame.height)
    )];
    if !used.is_empty() {
        let css = css_keyframes(s, &used);
        if !css.is_empty() {
            out.push(format!("  <style>{css}</style>"));
        }
    }
    if !defs.is_empty() {
        out.push(format!("  <defs>{}</defs>", defs.concat()));
    }
    out.extend(body);
    out.push("</svg>".into());
    out.join("\n") + "\n"
}
