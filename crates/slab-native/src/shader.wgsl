// slab-native pipelines: instanced SDF rounded rects (fill/stroke/shadow +
// in-shader gradients + grain), hinted A8 and RGBA atlas glyphs, lyon path
// meshes (solid or gradient), textured quads (images, layer composites,
// backdrops), layer-mask multiplies, banded progressive backdrops, projective
// tilt composites, and a separable gaussian blur.
//
// Coordinates: instance data is DEVICE pixels; the per-instance 2x3 affine
// (rotation/scale about a point) maps device -> device; to_ndc flips to clip
// space. Colors are straight-alpha sRGB bytes /255 (blending happens in sRGB
// space, matching the tiny-skia raster, Slate, and web drivers); fragments
// premultiply.

struct Globals {
    viewport: vec2<f32>,
    _pad: vec2<f32>,
}
@group(0) @binding(0) var<uniform> globals: Globals;

struct Grad {
    kind: u32,   // 0 linear, 1 radial, 2 conic
    count: u32,
    _p0: u32,
    _p1: u32,
    pos: array<vec4<f32>, 2>,   // stop positions, 8 max
    col: array<vec4<f32>, 8>,   // straight-alpha stop colors
}
@group(0) @binding(1) var<storage, read> grads: array<Grad>;

fn to_ndc(p: vec2<f32>) -> vec4<f32> {
    return vec4(p.x / globals.viewport.x * 2.0 - 1.0,
                1.0 - p.y / globals.viewport.y * 2.0, 0.0, 1.0);
}

fn sd_rrect(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let rr = min(r, min(b.x, b.y));
    let q = abs(p) - b + vec2(rr, rr);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2(0.0, 0.0))) - rr;
}

// clip: (x0, y0, x1, y1) device px + corner radius; cov in [0,1].
fn clip_cov(frag: vec2<f32>, clip: vec4<f32>, radius: f32) -> f32 {
    let c = (clip.xy + clip.zw) * 0.5;
    let h = (clip.zw - clip.xy) * 0.5;
    if (h.x <= 0.0 || h.y <= 0.0) {
        return 0.0;
    }
    let d = sd_rrect(frag - c, h, radius);
    return 1.0 - smoothstep(-0.5, 0.5, d);
}

// sRGB piecewise-linear stop ramp (render-time rule: gradients stay sRGB).
fn grad_color(gi: u32, t: f32) -> vec4<f32> {
    let n = grads[gi].count;
    if (n == 0u) {
        return vec4(0.0);
    }
    let tc = clamp(t, 0.0, 1.0);
    var pp = grads[gi].pos[0u][0u];
    var pc = grads[gi].col[0u];
    if (tc <= pp) {
        return pc;
    }
    for (var i = 1u; i < n && i < 8u; i = i + 1u) {
        let p = grads[gi].pos[i / 4u][i % 4u];
        let c = grads[gi].col[i];
        if (tc <= p) {
            return mix(pc, c, (tc - pp) / max(p - pp, 1e-6));
        }
        pp = p;
        pc = c;
    }
    return pc;
}

// Ramp parameter for a paint mapped over a box. `local` is the pixel
// relative to the box CENTER (pre-transform device px); `dir` is the
// CPU-packed geometry: linear = unit direction scaled by 1/extent,
// radial = (1/radius, 0), conic = (from-angle in degrees, 0).
fn grad_t(gi: u32, local: vec2<f32>, dir: vec2<f32>) -> f32 {
    let kind = grads[gi].kind;
    if (kind == 2u) {
        // conic (contract 6.1): clockwise, 0 = up; fract == rem_euclid/360
        let ang = degrees(atan2(local.x, -local.y));
        return fract((ang - dir.x) / 360.0);
    }
    if (kind == 1u) {
        return length(local) * dir.x;
    }
    return dot(local, dir) + 0.5;
}

// pcg2d hash (Jarzynski–Olano, contract 6.2): seedless, u32 wrapping;
// returns a uniform float in [0, 1).
fn pcg2d(i: i32, j: i32) -> f32 {
    var vx = bitcast<u32>(i) * 1664525u + 1013904223u;
    var vy = bitcast<u32>(j) * 1664525u + 1013904223u;
    vx = vx + vy * 1664525u;
    vy = vy + vx * 1664525u;
    vx = vx ^ (vx >> 16u);
    vy = vy ^ (vy >> 16u);
    vx = vx + vy * 1664525u;
    vy = vy + vx * 1664525u;
    vx = vx ^ (vx >> 16u);
    vy = vy ^ (vy >> 16u);
    return f32(vx) / 4294967296.0;
}

// ---------------------------------------------------------------- rects ----

const INSET_SHADOW: f32 = -2.0;

struct RectIn {
    @location(0) mabcd: vec4<f32>,   // affine a b c d
    @location(1) mtc: vec4<f32>,     // tx ty | center xy
    @location(2) hrs: vec4<f32>,     // half xy | radius | stroke half-width
    @location(3) sg: vec4<f32>,      // stroke off | shadow sigma | grad/inset tag | dir.x/shadow dx
    @location(4) dc: vec4<f32>,      // dir.y/shadow dy | clip radius | clip x0 y0
    @location(5) c2: vec4<f32>,      // clip x1 y1 | pad pad
    @location(6) fill: vec4<f32>,
    @location(7) stroke: vec4<f32>,  // solid rgba | (dir xy, opacity, pad) for gradient strokes
    @location(8) g2: vec4<f32>,      // grain amount | grain cell px | stroke grad tag | grain opacity
}

struct RectVary {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) hrs: vec4<f32>,
    @location(2) sg: vec4<f32>,
    @location(3) dc: vec4<f32>,
    @location(4) c2: vec4<f32>,
    @location(5) fill: vec4<f32>,
    @location(6) stroke: vec4<f32>,
    @location(7) g2: vec4<f32>,
}

@vertex
fn vs_rect(@builtin(vertex_index) vi: u32, r: RectIn) -> RectVary {
    let sx = f32(vi & 1u) * 2.0 - 1.0;
    let sy = f32(vi >> 1u) * 2.0 - 1.0;
    let pad = 2.0 * r.hrs.w + 3.0 * r.sg.y + 1.5;
    let local = vec2(sx, sy) * (r.hrs.xy + vec2(pad, pad));
    let p = r.mtc.zw + local;
    let world = vec2(r.mabcd.x * p.x + r.mabcd.z * p.y + r.mtc.x,
                     r.mabcd.y * p.x + r.mabcd.w * p.y + r.mtc.y);
    var out: RectVary;
    out.clip_pos = to_ndc(world);
    out.local = local;
    out.hrs = r.hrs;
    out.sg = r.sg;
    out.dc = r.dc;
    out.c2 = r.c2;
    out.fill = r.fill;
    out.stroke = r.stroke;
    out.g2 = r.g2;
    return out;
}

@fragment
fn fs_rect(v: RectVary) -> @location(0) vec4<f32> {
    let d = sd_rrect(v.local, v.hrs.xy, v.hrs.z);
    var out = vec4(0.0);
    if (v.sg.y > 0.0) {
        // sigma = blur/2 (CSS box-shadow); inset uses blurred inverse coverage.
        let s = 2.0 * v.sg.y;
        let color = vec4(v.fill.rgb * v.fill.a, v.fill.a);
        if (v.sg.z == INSET_SHADOW) {
            let hole = sd_rrect(v.local - vec2(v.sg.w, v.dc.x), v.hrs.xy, v.hrs.z);
            let inverse_cov = smoothstep(-s, s, hole);
            let inside_cov = 1.0 - smoothstep(-0.5, 0.5, d);
            out = color * inverse_cov * inside_cov;
        } else {
            let cov = 1.0 - smoothstep(-s, s, d);
            out = color * cov;
        }
    } else {
        var fc = v.fill;
        if (v.sg.z >= 0.0) {
            let gi = u32(v.sg.z);
            var gc = grad_color(gi, grad_t(gi, v.local, vec2(v.sg.w, v.dc.x)));
            gc.a = gc.a * v.fill.a; // fill.a carries group opacity for gradients
            fc = gc;
        }
        let fcov = 1.0 - smoothstep(-0.5, 0.5, d);
        out = vec4(fc.rgb * fc.a, fc.a) * fcov;
        if (v.hrs.w > 0.0) {
            var stc = v.stroke;
            if (v.g2.z >= 0.0) {
                // gradient stroke: stroke carries (dir xy, opacity)
                let gi = u32(v.g2.z);
                let gc = grad_color(gi, grad_t(gi, v.local, v.stroke.xy));
                stc = vec4(gc.rgb, gc.a * v.stroke.z);
            }
            let band = abs(d - v.sg.x);
            let scov = 1.0 - smoothstep(v.hrs.w - 0.5, v.hrs.w + 0.5, band);
            let sc = vec4(stc.rgb * stc.a, stc.a) * scov;
            out = sc + out * (1.0 - sc.a);
        }
        if (v.g2.x > 0.0) {
            // grain speckle over the fill area (contract 6.2), node-local
            // cells so the pattern is static under motion.
            let cell = max(v.g2.y, 1e-3);
            let gi2 = i32(floor((v.local.x + v.hrs.x) / cell));
            let gj2 = i32(floor((v.local.y + v.hrs.y) / cell));
            let h = 2.0 * pcg2d(gi2, gj2) - 1.0;
            let ga = v.g2.x * abs(h) * v.g2.w;
            let gl = select(0.0, 1.0, h > 0.0);
            let gc = vec4(vec3(gl) * ga, ga) * fcov;
            out = gc + out * (1.0 - gc.a);
        }
    }
    return out * clip_cov(v.clip_pos.xy, vec4(v.dc.zw, v.c2.xy), v.dc.y);
}

fn srgb_enc1(x: f32) -> f32 {
    return select(1.055 * pow(max(x, 0.0), 1.0 / 2.4) - 0.055, 12.92 * x, x <= 0.0031308);
}

fn srgb_enc3(x: vec3<f32>) -> vec3<f32> {
    return vec3(srgb_enc1(x.r), srgb_enc1(x.g), srgb_enc1(x.b));
}

// --------------------------------------------------------------- glyphs ----

@group(1) @binding(0) var tex0: texture_2d<f32>; // RGBA color glyphs / regular textures
@group(1) @binding(1) var tex1: texture_2d<f32>; // A8 glyph masks
@group(1) @binding(2) var samp0: sampler;

struct GlyphIn {
    @location(0) mabcd: vec4<f32>,
    @location(1) mtp: vec4<f32>,   // tx ty | quad top-left xy (device)
    @location(2) su: vec4<f32>,    // quad size wh | atlas pixel xy
    @location(3) uc: vec4<f32>,    // atlas pixel size wh | clip radius | color flag
    @location(4) clip: vec4<f32>,
    @location(5) color: vec4<f32>, // solid rgba | (dir xy, 0, 0) for gradient ink
    @location(6) g2: vec4<f32>,    // grad box center xy | grad tag | opacity
    @location(7) ink: vec4<f32>,   // nominal device px | reserved
}

struct GlyphVary {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) clip: vec4<f32>,
    @location(3) cr: f32,
    @location(4) pre: vec2<f32>,   // pre-transform device position
    @location(5) g2: vec4<f32>,
    @location(6) @interpolate(flat) kind: u32,
    @location(7) @interpolate(flat) device_px: f32,
    @location(8) @interpolate(flat) uv_rect: vec4<f32>,
}

@vertex
fn vs_glyph(@builtin(vertex_index) vi: u32, g: GlyphIn) -> GlyphVary {
    let corner = vec2(f32(vi & 1u), f32(vi >> 1u));
    let p = g.mtp.zw + corner * g.su.xy;
    let world = vec2(g.mabcd.x * p.x + g.mabcd.z * p.y + g.mtp.x,
                     g.mabcd.y * p.x + g.mabcd.w * p.y + g.mtp.y);
    var out: GlyphVary;
    out.clip_pos = to_ndc(world);
    out.uv = g.su.zw + corner * g.uc.xy;
    out.color = g.color;
    out.clip = g.clip;
    out.cr = g.uc.z;
    out.pre = p;
    out.g2 = g.g2;
    out.kind = u32(g.uc.w);
    out.device_px = g.ink.x;
    out.uv_rect = vec4(g.su.zw + vec2(0.5), g.su.zw + g.uc.xy - vec2(0.5));
    return out;
}

fn mask_tap(uv: vec2<f32>, rect: vec4<f32>) -> f32 {
    let dims = vec2<f32>(textureDimensions(tex1));
    return textureSampleLevel(tex1, samp0, clamp(uv, rect.xy, rect.zw) / dims, 0.0).r;
}

@fragment
fn fs_glyph(v: GlyphVary) -> @location(0) vec4<f32> {
    var col = v.color;
    if (v.g2.z >= 0.0) {
        // Gradient text (contract 6.7): ink mapped over the node content box.
        let gi = u32(v.g2.z);
        col = grad_color(gi, grad_t(gi, v.pre - v.g2.xy, v.color.xy));
        col.a = col.a * v.g2.w;
    }

    if (v.kind == 1u) {
        let dims = vec2<f32>(textureDimensions(tex0));
        let sample = textureSampleLevel(tex0, samp0, v.uv / dims, 0.0);
        let cov = sample.a * col.a;
        let peak = max(sample.r, max(sample.g, sample.b));
        let alpha = srgb_enc1(peak * cov)
            + 1.0 - srgb_enc1(1.0 - cov * (1.0 - peak));
        let out = vec4(srgb_enc3(sample.rgb * cov), alpha);
        return out * clip_cov(v.clip_pos.xy, v.clip, v.cr);
    }

    // Small grayscale glyphs lose apparent stroke weight at low DPI. Dilate
    // coverage (never blur it) toward four nearby mask samples. The nominal
    // device size gates the effect below 18px; light-on-dark ink gets the
    // stronger lift, matching its greater perceived low-DPI fade.
    const DILATE_STRENGTH: f32 = 0.42;
    const DILATE_RADIUS: f32 = 0.75;
    let sharp = mask_tap(v.uv, v.uv_rect);
    var cov = sharp;
    let small_ink = clamp((18.0 - v.device_px) / 18.0, 0.0, 1.0);
    let lum = dot(col.rgb, vec3(0.299, 0.587, 0.114));
    let polarity = mix(0.72, 1.0, lum);
    let amount = min(small_ink * 1.2, 1.0) * DILATE_STRENGTH * polarity;
    if (amount > 0.0) {
        let d = DILATE_RADIUS;
        let n = max(
            max(
                mask_tap(v.uv + vec2(d, 0.0), v.uv_rect),
                mask_tap(v.uv - vec2(d, 0.0), v.uv_rect),
            ),
            max(
                mask_tap(v.uv + vec2(0.0, d), v.uv_rect),
                mask_tap(v.uv - vec2(0.0, d), v.uv_rect),
            ),
        );
        cov = max(sharp, n * amount);
    }
    let cov_alpha = cov * col.a;
    let peak = max(col.r, max(col.g, col.b));
    let alpha = srgb_enc1(peak * cov_alpha)
        + 1.0 - srgb_enc1(1.0 - cov_alpha * (1.0 - peak));
    let out = vec4(srgb_enc3(col.rgb * cov_alpha), alpha);
    return out * clip_cov(v.clip_pos.xy, v.clip, v.cr);
}

// --------------------------------------------------------------- meshes ----

struct MeshVertex {
    @location(0) pos: vec2<f32>,   // path-local logical units
}

struct MeshIn {
    @location(1) mabcd: vec4<f32>,
    @location(2) mto: vec4<f32>,   // tx ty | offset xy (device)
    @location(3) sc: vec4<f32>,    // scale | clip radius | grad tag | grad opacity
    @location(4) clip: vec4<f32>,
    @location(5) color: vec4<f32>, // solid rgba | (box center xy, dir xy) for gradients
}

struct MeshVary {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) clip: vec4<f32>,
    @location(2) crg: vec4<f32>,   // clip radius | grad tag | grad opacity | pad
    @location(3) pre: vec2<f32>,   // pre-transform device position
}

@vertex
fn vs_mesh(vtx: MeshVertex, m: MeshIn) -> MeshVary {
    let p = vtx.pos * m.sc.x + m.mto.zw;
    let world = vec2(m.mabcd.x * p.x + m.mabcd.z * p.y + m.mto.x,
                     m.mabcd.y * p.x + m.mabcd.w * p.y + m.mto.y);
    var out: MeshVary;
    out.clip_pos = to_ndc(world);
    out.color = m.color;
    out.clip = m.clip;
    out.crg = vec4(m.sc.y, m.sc.z, m.sc.w, 0.0);
    out.pre = p;
    return out;
}

@fragment
fn fs_mesh(v: MeshVary) -> @location(0) vec4<f32> {
    var col = v.color;
    if (v.crg.y >= 0.0) {
        // gradient fill/stroke: color carries the box center + packed dir
        let gi = u32(v.crg.y);
        col = grad_color(gi, grad_t(gi, v.pre - v.color.xy, v.color.zw));
        col.a = col.a * v.crg.z;
    }
    let out = vec4(col.rgb * col.a, col.a);
    return out * clip_cov(v.clip_pos.xy, v.clip, v.crg.x);
}

// ------------------------------------------------- textured quads (tex) ----
// Images, layer composites, backdrop paint-back. Sampled texture is
// PREMULTIPLIED (layers render premul; image uploads premultiply).

struct TexIn {
    @location(0) mabcd: vec4<f32>,
    @location(1) mtc: vec4<f32>,   // tx ty | center xy
    @location(2) hro: vec4<f32>,   // half xy | radius | opacity
    @location(3) uv: vec4<f32>,    // uv0 | uv size
    @location(4) clip: vec4<f32>,
    @location(5) misc: vec4<f32>,  // clip radius | saturate | uv-mask flag | brightness
}

struct TexVary {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) hro: vec4<f32>,
    @location(3) clip: vec4<f32>,
    @location(4) misc: vec4<f32>,
}

@vertex
fn vs_tex(@builtin(vertex_index) vi: u32, t: TexIn) -> TexVary {
    let sx = f32(vi & 1u) * 2.0 - 1.0;
    let sy = f32(vi >> 1u) * 2.0 - 1.0;
    let local = vec2(sx, sy) * (t.hro.xy + vec2(0.75, 0.75));
    let p = t.mtc.zw + local;
    let world = vec2(t.mabcd.x * p.x + t.mabcd.z * p.y + t.mtc.x,
                     t.mabcd.y * p.x + t.mabcd.w * p.y + t.mtc.y);
    var out: TexVary;
    out.clip_pos = to_ndc(world);
    out.local = local;
    out.uv = t.uv.xy + (local / (2.0 * t.hro.xy) + vec2(0.5, 0.5)) * t.uv.zw;
    out.hro = t.hro;
    out.clip = t.clip;
    out.misc = t.misc;
    return out;
}

// saturate + brightness on unpremultiplied rgb; clamped like u8 stores.
fn sat_bright(c: vec4<f32>, saturate: f32, brightness: f32) -> vec4<f32> {
    if ((saturate == 1.0 && brightness == 1.0) || c.a <= 0.0) {
        return c;
    }
    var rgb = c.rgb / c.a;
    let lum = dot(rgb, vec3(0.2126, 0.7152, 0.0722));
    rgb = clamp(mix(vec3(lum), rgb, saturate) * brightness, vec3(0.0), vec3(1.0));
    return vec4(rgb * c.a, c.a);
}

@fragment
fn fs_tex(v: TexVary) -> @location(0) vec4<f32> {
    var c = textureSample(tex0, samp0, v.uv);
    c = sat_bright(c, v.misc.y, v.misc.w);
    // misc.z = 1: mask uv outside [0,1] (image `contain` letterboxing)
    if (v.misc.z > 0.5
        && (v.uv.x < 0.0 || v.uv.x > 1.0 || v.uv.y < 0.0 || v.uv.y > 1.0)) {
        c = vec4(0.0);
    }
    let d = sd_rrect(v.local, v.hro.xy, v.hro.z);
    let cov = 1.0 - smoothstep(-0.5, 0.5, d);
    return c * v.hro.w * cov * clip_cov(v.clip_pos.xy, v.clip, v.misc.x);
}

// -------------------------------------------- banded backdrop paint-back ----
// One band of a progressive backdrop (contract 6.6): the blurred capture is
// painted back only where the mask paint's alpha falls inside [lo, hi).

struct TexBandIn {
    @location(0) mabcd: vec4<f32>,
    @location(1) mtc: vec4<f32>,   // tx ty | center xy
    @location(2) hro: vec4<f32>,   // half xy | radius | opacity
    @location(3) uv: vec4<f32>,    // uv0 | uv size
    @location(4) clip: vec4<f32>,
    @location(5) misc: vec4<f32>,  // clip radius | saturate | brightness | pad
    @location(6) mgrad: vec4<f32>, // mask grad tag | dir xy | solid alpha
    @location(7) band: vec4<f32>,  // alpha lo | alpha hi | pad pad
}

struct TexBandVary {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) hro: vec4<f32>,
    @location(3) clip: vec4<f32>,
    @location(4) misc: vec4<f32>,
    @location(5) mgrad: vec4<f32>,
    @location(6) band: vec4<f32>,
}

@vertex
fn vs_texband(@builtin(vertex_index) vi: u32, t: TexBandIn) -> TexBandVary {
    let sx = f32(vi & 1u) * 2.0 - 1.0;
    let sy = f32(vi >> 1u) * 2.0 - 1.0;
    let local = vec2(sx, sy) * (t.hro.xy + vec2(0.75, 0.75));
    let p = t.mtc.zw + local;
    let world = vec2(t.mabcd.x * p.x + t.mabcd.z * p.y + t.mtc.x,
                     t.mabcd.y * p.x + t.mabcd.w * p.y + t.mtc.y);
    var out: TexBandVary;
    out.clip_pos = to_ndc(world);
    out.local = local;
    out.uv = t.uv.xy + (local / (2.0 * t.hro.xy) + vec2(0.5, 0.5)) * t.uv.zw;
    out.hro = t.hro;
    out.clip = t.clip;
    out.misc = t.misc;
    out.mgrad = t.mgrad;
    out.band = t.band;
    return out;
}

@fragment
fn fs_texband(v: TexBandVary) -> @location(0) vec4<f32> {
    var c = textureSample(tex0, samp0, v.uv);
    // mask paint alpha over the backdrop box (local is box-centered)
    var ma = v.mgrad.w;
    if (v.mgrad.x >= 0.0) {
        let gi = u32(v.mgrad.x);
        ma = grad_color(gi, grad_t(gi, v.local, v.mgrad.yz)).a;
    }
    let inband = select(0.0, 1.0, ma >= v.band.x && ma < v.band.y);
    c = sat_bright(c, v.misc.y, v.misc.z);
    let d = sd_rrect(v.local, v.hro.xy, v.hro.z);
    let cov = 1.0 - smoothstep(-0.5, 0.5, d);
    return c * v.hro.w * cov * inband * clip_cov(v.clip_pos.xy, v.clip, v.misc.x);
}

// ------------------------------------------------- layer mask multiply ----
// Full-target quad multiplying the current layer by a paint's alpha mapped
// over a box (contract 6.3). The pipeline blends dst *= src.a, so only the
// fragment's alpha matters; coverage outside the box is zero.

struct MaskIn {
    @location(0) rect: vec4<f32>,  // draw region x0 y0 x1 y1 (device)
    @location(1) bx: vec4<f32>,    // box center xy | box half wh
    @location(2) grad: vec4<f32>,  // grad tag | dir xy | solid alpha
}

struct MaskVary {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world: vec2<f32>,
    @location(1) bx: vec4<f32>,
    @location(2) grad: vec4<f32>,
}

@vertex
fn vs_mask(@builtin(vertex_index) vi: u32, m: MaskIn) -> MaskVary {
    let corner = vec2(f32(vi & 1u), f32(vi >> 1u));
    let p = mix(m.rect.xy, m.rect.zw, corner);
    var out: MaskVary;
    out.clip_pos = to_ndc(p);
    out.world = p;
    out.bx = m.bx;
    out.grad = m.grad;
    return out;
}

@fragment
fn fs_mask(v: MaskVary) -> @location(0) vec4<f32> {
    let local = v.world - v.bx.xy;
    var a = v.grad.w;
    if (v.grad.x >= 0.0) {
        let gi = u32(v.grad.x);
        a = grad_color(gi, grad_t(gi, local, v.grad.yz)).a;
    }
    let q = abs(local) - v.bx.zw;
    let inside = 1.0 - smoothstep(-0.5, 0.5, max(q.x, q.y));
    return vec4(0.0, 0.0, 0.0, a * inside);
}

// -------------------------------------------------------- tilt composite ----
// Projectively-correct textured quad (contract 6.5): the four corners are
// CPU-projected; uv rides (u*w, v*w, w) through screen-linear interpolation
// and divides per fragment.

struct TiltIn {
    @location(0) p01: vec4<f32>,   // corner 0 xy | corner 1 xy
    @location(1) p23: vec4<f32>,   // corner 2 xy | corner 3 xy
    @location(2) ws: vec4<f32>,    // homogeneous w per corner
    @location(3) clip: vec4<f32>,
    @location(4) misc: vec4<f32>,  // clip radius | pad pad pad
}

struct TiltVary {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uvw: vec3<f32>,
    @location(1) clip: vec4<f32>,
    @location(2) misc: vec4<f32>,
}

@vertex
fn vs_tilt(@builtin(vertex_index) vi: u32, t: TiltIn) -> TiltVary {
    var p = t.p01.xy;
    if (vi == 1u) {
        p = t.p01.zw;
    } else if (vi == 2u) {
        p = t.p23.xy;
    } else if (vi == 3u) {
        p = t.p23.zw;
    }
    let corner = vec2(f32(vi & 1u), f32(vi >> 1u));
    let w = t.ws[vi];
    var out: TiltVary;
    out.clip_pos = to_ndc(p);
    out.uvw = vec3(corner * w, w);
    out.clip = t.clip;
    out.misc = t.misc;
    return out;
}

@fragment
fn fs_tilt(v: TiltVary) -> @location(0) vec4<f32> {
    let uv = clamp(v.uvw.xy / max(v.uvw.z, 1e-6), vec2(0.0), vec2(1.0));
    let c = textureSample(tex0, samp0, uv);
    return c * clip_cov(v.clip_pos.xy, v.clip, v.misc.x);
}

// ----------------------------------------------------------------- blur ----

struct BlurIn {
    @location(0) rect: vec4<f32>,  // dst x0 y0 x1 y1 (device)
    @location(1) uvr: vec4<f32>,   // uv clamp region u0 v0 u1 v1
    @location(2) ds: vec4<f32>,    // dir (texel step) | sigma | taps
}

struct BlurVary {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) uvr: vec4<f32>,
    @location(2) ds: vec4<f32>,
}

@vertex
fn vs_blur(@builtin(vertex_index) vi: u32, b: BlurIn) -> BlurVary {
    let corner = vec2(f32(vi & 1u), f32(vi >> 1u));
    let p = mix(b.rect.xy, b.rect.zw, corner);
    var out: BlurVary;
    out.clip_pos = to_ndc(p);
    out.uv = mix(b.uvr.xy, b.uvr.zw, corner);
    out.uvr = b.uvr;
    out.ds = b.ds;
    return out;
}

@fragment
fn fs_blur(v: BlurVary) -> @location(0) vec4<f32> {
    let sigma = max(v.ds.z, 0.001);
    let taps = i32(v.ds.w);
    var sum = vec4(0.0);
    var wsum = 0.0;
    for (var i = -taps; i <= taps; i = i + 1) {
        let w = exp(-f32(i * i) / (2.0 * sigma * sigma));
        let uv = clamp(v.uv + v.ds.xy * f32(i), v.uvr.xy, v.uvr.zw);
        sum = sum + textureSampleLevel(tex0, samp0, uv, 0.0) * w;
        wsum = wsum + w;
    }
    return sum / wsum;
}
