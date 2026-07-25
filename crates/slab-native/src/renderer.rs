//! Window-independent wgpu renderer for kernel Frames.
//!
//! Model (research slab-wgpu, extended): painter's-order premultiplied alpha,
//! no depth buffer. Rects are instanced quads with an SDF fragment shader
//! (fill, aligned stroke, in-shader linear/radial gradients, blurred-SDF
//! shadows, rounded clip). Text is textured quads from a shared A8 atlas fed
//! by kernel `text_glyphs`. Paths are lyon meshes tessellated at first use.
//! GroupPush/Pop composite through pooled offscreen layers with opacity and
//! two-pass gaussian blur; Backdrop copies the current target region, blurs
//! it, and paints it back with a rounded mask + saturation.
//!
//! Everything renders into an internal Rgba8Unorm target (blending in sRGB
//! byte space, matching the tiny-skia raster and the web driver), then blits
//! to the window surface or reads back for headless PNG/probes. f64 model
//! values narrow to f32 ONLY here, at instance packing.

use crate::RegisteredFont;
use crate::atlas::{ATLAS, Atlas, Face};
use crate::tess::{
    Mesh, fill_mesh, fill_mesh_data, rect_stroke_mesh, squircle_fill_mesh, stroke_mesh,
    stroke_mesh_data,
};
use bytemuck::Zeroable;
use slab_kernel::flatten::{Frame, FrameOp, RtPath};
use slab_kernel::frame::Instance;
use slab_kernel::slir::Doc;
use std::collections::{HashMap, HashSet};
use wgpu::util::DeviceExt;

const INTERNAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const MAX_GRAD_STOPS: usize = 8;

// ------------------------------------------------------------ instances ----

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RectI {
    mabcd: [f32; 4],
    mtc: [f32; 4],
    hrs: [f32; 4],
    sg: [f32; 4],
    dc: [f32; 4],
    c2: [f32; 4],
    fill: [f32; 4],
    stroke: [f32; 4],
    /// grain amount | grain cell (device px) | stroke grad tag | grain opacity
    g2: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlyphI {
    mabcd: [f32; 4],
    mtp: [f32; 4],
    su: [f32; 4],
    uc: [f32; 4],
    clip: [f32; 4],
    color: [f32; 4],
    /// grad box center xy | grad tag | opacity
    g2: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MeshI {
    mabcd: [f32; 4],
    mto: [f32; 4],
    sc: [f32; 4],
    clip: [f32; 4],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TexI {
    mabcd: [f32; 4],
    mtc: [f32; 4],
    hro: [f32; 4],
    uv: [f32; 4],
    clip: [f32; 4],
    misc: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurI {
    rect: [f32; 4],
    uvr: [f32; 4],
    ds: [f32; 4],
}

/// Banded progressive-backdrop paint-back (`fs_texband`): a [`TexI`] plus
/// the mask paint geometry and the band's alpha window.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TexBandI {
    mabcd: [f32; 4],
    mtc: [f32; 4],
    hro: [f32; 4],
    uv: [f32; 4],
    clip: [f32; 4],
    /// clip radius | saturate | brightness | pad
    misc: [f32; 4],
    /// mask grad tag | dir xy | solid alpha
    mgrad: [f32; 4],
    /// band alpha lo | hi | pad pad
    band: [f32; 4],
}

/// Layer-mask multiply quad (`fs_mask`): dst *= paint alpha over a box.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaskI {
    /// draw region x0 y0 x1 y1 (device px)
    rect: [f32; 4],
    /// box center xy | box half wh
    bx: [f32; 4],
    /// grad tag | dir xy | solid alpha
    grad: [f32; 4],
}

/// Tilt composite quad (`fs_tilt`): four CPU-projected corners with
/// homogeneous weights for projectively-correct uvs.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TiltI {
    /// corner 0 xy | corner 1 xy
    p01: [f32; 4],
    /// corner 2 xy | corner 3 xy
    p23: [f32; 4],
    ws: [f32; 4],
    clip: [f32; 4],
    /// clip radius | pad pad pad
    misc: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GradGpu {
    info: [u32; 4],
    pos: [f32; 8],
    col: [[f32; 4]; 8],
}

const RECT_ATTRS: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
    0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4,
    4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4,
    8 => Float32x4
];
const GLYPH_ATTRS: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
    0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4,
    4 => Float32x4, 5 => Float32x4, 6 => Float32x4
];
const MESH_VTX_ATTRS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x2];
const MESH_INST_ATTRS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4
];
const TEX_ATTRS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4,
    4 => Float32x4, 5 => Float32x4
];
const TEXBAND_ATTRS: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
    0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4,
    4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4
];
const MASK_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];
const TILT_ATTRS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
    0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4
];
const BLUR_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];

// ------------------------------------------------------------- geometry ----

/// Row-major 2x3 affine in device px: columns (a,b), (c,d), translation.
#[derive(Clone, Copy, PartialEq)]
struct Mat {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    tx: f32,
    ty: f32,
}

impl Mat {
    const I: Mat = Mat {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    fn is_identity(&self) -> bool {
        *self == Mat::I
    }

    /// self ∘ o: apply `o` first, then self.
    fn then(self, o: Mat) -> Mat {
        Mat {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            tx: self.a * o.tx + self.c * o.ty + self.tx,
            ty: self.b * o.tx + self.d * o.ty + self.ty,
        }
    }

    fn rotate_about(cx: f32, cy: f32, deg: f32) -> Mat {
        let r = deg.to_radians();
        let (s, c) = r.sin_cos();
        Mat {
            a: c,
            b: s,
            c: -s,
            d: c,
            tx: cx - c * cx + s * cy,
            ty: cy - s * cx - c * cy,
        }
    }

    fn scale_about(cx: f32, cy: f32, sx: f32, sy: f32) -> Mat {
        Mat {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            tx: cx - sx * cx,
            ty: cy - sy * cy,
        }
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        )
    }

    /// Inverse affine, or `None` when degenerate (zero-scale collapse).
    fn invert(&self) -> Option<Mat> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-9 {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Mat {
            a,
            b,
            c,
            d,
            tx: -(a * self.tx + c * self.ty),
            ty: -(b * self.tx + d * self.ty),
        })
    }
}

type Sc = (u32, u32, u32, u32);

fn sc_intersect(a: Sc, b: Sc) -> Sc {
    let x0 = a.0.max(b.0);
    let y0 = a.1.max(b.1);
    let x1 = (a.0 + a.2).min(b.0 + b.2);
    let y1 = (a.1 + a.3).min(b.1 + b.3);
    (x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
}

#[derive(Clone, Copy)]
struct ClipSt {
    sdf: [f32; 4],
    radius: f32,
    scissor: Sc,
}

fn rgba(v: u32, opacity: f64) -> [f32; 4] {
    let [r, g, b, a] = v.to_le_bytes();
    [
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        (a as f64 / 255.0 * opacity).clamp(0.0, 1.0) as f32,
    ]
}

fn mesh_scalar_key(value: f64) -> i64 {
    (value * 100.0).round() as i64
}

fn dash_key(dash: (f64, f64)) -> [i64; 2] {
    [mesh_scalar_key(dash.0), mesh_scalar_key(dash.1)]
}

/// Packed gradient geometry for the shader's `grad_t`: linear = unit
/// direction / extent, radial = (1/radius, 0), conic = (from-angle in
/// degrees, 0). `wd`/`hd` are the paint box dimensions in device px.
fn grad_dir(kind: u32, angle: f64, wd: f32, hd: f32) -> [f32; 2] {
    match kind {
        0 => {
            let th = angle.to_radians();
            let (dx, dy) = (th.sin() as f32, -th.cos() as f32);
            let ln = ((wd * dx).abs() + (hd * dy).abs()).max(1e-6);
            [dx / ln, dy / ln]
        }
        1 => {
            let rr = (wd * wd + hd * hd).sqrt() / 2.0;
            [1.0 / rr.max(1e-6), 0.0]
        }
        _ => [angle as f32, 0.0],
    }
}

/// Control-point bbox `[x, y, w, h]` of a path in path-local units — the
/// gradient paint box for path fills/strokes (matches tiny-skia bounds).
fn path_bounds(doc: &Doc, runtime: Option<&RtPath>, path: i32) -> Option<[f64; 4]> {
    let coords: &[f64] = match runtime {
        Some(rt) => &rt.coords,
        None => {
            let p = usize::try_from(path).ok()?;
            let off = usize::try_from(*doc.path_coord_off.get(p)?).ok()?;
            let len = usize::try_from(*doc.path_coord_len.get(p)?).ok()?;
            doc.path_coords.get(off..off.checked_add(len)?)?
        }
    };
    let mut pairs = coords.chunks_exact(2);
    let first = pairs.next()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first[0], first[1], first[0], first[1]);
    for pair in pairs {
        x0 = x0.min(pair[0]);
        y0 = y0.min(pair[1]);
        x1 = x1.max(pair[0]);
        y1 = y1.max(pair[1]);
    }
    Some([x0, y0, x1 - x0, y1 - y0])
}

// ------------------------------------------------------- per-doc caches ----

struct ImgTex {
    bind: wgpu::BindGroup,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum StrokeKey {
    Path {
        path: i32,
        width: i64,
        dash: Option<[i64; 2]>,
    },
    Rect {
        size: [i64; 2],
        radius: i64,
        width: i64,
        align: u32,
        sides: u32,
        dash: [i64; 2],
        smooth: i64,
    },
}

/// GPU-side resources for one registered SLIR document.
pub struct DocRes {
    grad_base: u32,
    grad_count: u32,
    fonts: Vec<Option<Face>>,
    fills: HashMap<i32, Option<Mesh>>,
    /// Squircle fill meshes keyed by quantized (w, h, radius, smooth).
    sq_fills: HashMap<[i64; 4], Option<Mesh>>,
    strokes: HashMap<StrokeKey, Option<Mesh>>,
    runtime_paths: Vec<RtPath>,
    images: Vec<Option<ImgTex>>,
    image_generations: Vec<Option<u32>>,
}

// ----------------------------------------------------------- FrameBuild ----

#[derive(Clone, Copy)]
enum MeshKey {
    Fill(i32),
    Squircle([i64; 4]),
    Stroke(StrokeKey),
}

enum Step {
    Rects {
        scissor: Sc,
        start: u32,
        end: u32,
    },
    Glyphs {
        scissor: Sc,
        start: u32,
        end: u32,
    },
    Mesh {
        scissor: Sc,
        doc: usize,
        key: MeshKey,
        inst: u32,
    },
    Image {
        scissor: Sc,
        doc: usize,
        img: usize,
        inst: u32,
    },
    PushLayer,
    PopLayer {
        opacity: f32,
        sigma: f32,
        /// Layer-mask multiply applied before compositing (contract 6.3).
        mask: Option<MaskI>,
    },
    /// Tilt composite: pop the layer and draw it as a projected quad
    /// (contract 6.5).
    PopTilt {
        inst: TiltI,
    },
    Backdrop {
        rect: [f32; 4],
        radius: f32,
        sigma: f32,
        saturate: f32,
        brightness: f32,
        /// Progressive-blur mask paint (tag | dir xy | solid alpha); banded
        /// per contract 6.6 when present.
        mask: Option<[f32; 4]>,
        scissor: Sc,
    },
}

/// CPU-built frame: instance lists + the pass/draw step sequence.
pub struct FrameBuild {
    rects: Vec<RectI>,
    glyphs: Vec<GlyphI>,
    meshes: Vec<MeshI>,
    texq: Vec<TexI>,
    steps: Vec<Step>,
    tw: u32,
    th: u32,
}

impl FrameBuild {
    fn push_rect(&mut self, scissor: Sc, inst: RectI) {
        if scissor.2 == 0 || scissor.3 == 0 {
            return;
        }
        let idx = self.rects.len() as u32;
        self.rects.push(inst);
        if let Some(Step::Rects {
            scissor: s, end, ..
        }) = self.steps.last_mut()
            && *s == scissor
            && *end == idx
        {
            *end = idx + 1;
            return;
        }
        self.steps.push(Step::Rects {
            scissor,
            start: idx,
            end: idx + 1,
        });
    }

    fn push_glyph(&mut self, scissor: Sc, inst: GlyphI) {
        if scissor.2 == 0 || scissor.3 == 0 {
            return;
        }
        let idx = self.glyphs.len() as u32;
        self.glyphs.push(inst);
        if let Some(Step::Glyphs {
            scissor: s, end, ..
        }) = self.steps.last_mut()
            && *s == scissor
            && *end == idx
        {
            *end = idx + 1;
            return;
        }
        self.steps.push(Step::Glyphs {
            scissor,
            start: idx,
            end: idx + 1,
        });
    }

    fn push_mesh(&mut self, scissor: Sc, doc: usize, key: MeshKey, inst: MeshI) {
        if scissor.2 == 0 || scissor.3 == 0 {
            return;
        }
        let index = self.meshes.len() as u32;
        self.meshes.push(inst);
        self.steps.push(Step::Mesh {
            scissor,
            doc,
            key,
            inst: index,
        });
    }
}

/// One (instance, frame) pair to draw: the main document at (0,0), hole
/// children translated into their hole rect with a forced clip.
pub struct LayerInput<'a> {
    pub doc_id: usize,
    pub inst: &'a Instance,
    pub frame: &'a Frame,
    pub ox: f64,
    pub oy: f64,
    /// (x, y, w, h, radius) in the PARENT's logical space.
    pub clip: Option<(f64, f64, f64, f64, f64)>,
}

// -------------------------------------------------------------- renderer ---

struct Target {
    tex: wgpu::Texture,
    view: wgpu::TextureView,
    bind: wgpu::BindGroup,
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    rect_pl: wgpu::RenderPipeline,
    glyph_pl: wgpu::RenderPipeline,
    mesh_pl: wgpu::RenderPipeline,
    tex_pl: wgpu::RenderPipeline,
    blur_pl: wgpu::RenderPipeline,
    texband_pl: wgpu::RenderPipeline,
    mask_pl: wgpu::RenderPipeline,
    tilt_pl: wgpu::RenderPipeline,
    blit_pls: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    shader: wgpu::ShaderModule,
    bgl_globals: wgpu::BindGroupLayout,
    bgl_tex: wgpu::BindGroupLayout,
    globals: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    grads_buf: wgpu::Buffer,
    grads_cpu: Vec<GradGpu>,
    sampler: wgpu::Sampler,
    atlas: Atlas,
    atlas_tex: wgpu::Texture,
    atlas_bg: wgpu::BindGroup,
    docs: Vec<DocRes>,
    main: Option<(u32, u32, Target)>,
    pool: Vec<Target>,
    notes: HashSet<&'static str>,
    pub scale: f64,
}

impl Renderer {
    /// One-time capability note on stderr (§12 `cap-*` wording).
    fn note(&mut self, code: &'static str, msg: &str) {
        if self.notes.insert(code) {
            eprintln!("slab-native: {code}: {msg}");
        }
    }

    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Renderer {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("slab"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let bgl_globals = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let bgl_tex = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tex"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let grads_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grads"),
            contents: bytemuck::cast_slice(&[GradGpu::zeroed()]),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let globals_bg = Self::make_globals_bg(&device, &bgl_globals, &globals, &grads_buf);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d {
                width: ATLAS,
                height: ATLAS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_view = atlas_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let atlas_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas"),
            layout: &bgl_tex,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let layout1 = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("g"),
            bind_group_layouts: &[&bgl_globals],
            push_constant_ranges: &[],
        });
        let layout2 = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gt"),
            bind_group_layouts: &[&bgl_globals, &bgl_tex],
            push_constant_ranges: &[],
        });

        let rect_pl = make_pipeline(
            &device,
            &shader,
            "rect",
            &layout1,
            "vs_rect",
            "fs_rect",
            &[inst_layout(
                std::mem::size_of::<RectI>() as u64,
                &RECT_ATTRS,
            )],
            INTERNAL_FORMAT,
            premul_blend(),
            wgpu::PrimitiveTopology::TriangleStrip,
        );
        let glyph_pl = make_pipeline(
            &device,
            &shader,
            "glyph",
            &layout2,
            "vs_glyph",
            "fs_glyph",
            &[inst_layout(
                std::mem::size_of::<GlyphI>() as u64,
                &GLYPH_ATTRS,
            )],
            INTERNAL_FORMAT,
            premul_blend(),
            wgpu::PrimitiveTopology::TriangleStrip,
        );
        let mesh_pl = make_pipeline(
            &device,
            &shader,
            "mesh",
            &layout1,
            "vs_mesh",
            "fs_mesh",
            &[
                wgpu::VertexBufferLayout {
                    array_stride: 8,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &MESH_VTX_ATTRS,
                },
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<MeshI>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &MESH_INST_ATTRS,
                },
            ],
            INTERNAL_FORMAT,
            premul_blend(),
            wgpu::PrimitiveTopology::TriangleList,
        );
        let tex_pl = make_pipeline(
            &device,
            &shader,
            "texq",
            &layout2,
            "vs_tex",
            "fs_tex",
            &[inst_layout(std::mem::size_of::<TexI>() as u64, &TEX_ATTRS)],
            INTERNAL_FORMAT,
            premul_blend(),
            wgpu::PrimitiveTopology::TriangleStrip,
        );
        let blur_pl = make_pipeline(
            &device,
            &shader,
            "blur",
            &layout2,
            "vs_blur",
            "fs_blur",
            &[inst_layout(
                std::mem::size_of::<BlurI>() as u64,
                &BLUR_ATTRS,
            )],
            INTERNAL_FORMAT,
            wgpu::BlendState::REPLACE,
            wgpu::PrimitiveTopology::TriangleStrip,
        );
        let texband_pl = make_pipeline(
            &device,
            &shader,
            "texband",
            &layout2,
            "vs_texband",
            "fs_texband",
            &[inst_layout(
                std::mem::size_of::<TexBandI>() as u64,
                &TEXBAND_ATTRS,
            )],
            INTERNAL_FORMAT,
            premul_blend(),
            wgpu::PrimitiveTopology::TriangleStrip,
        );
        // dst *= src.a: the mask fragment only produces alpha
        let mask_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::SrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::SrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let mask_pl = make_pipeline(
            &device,
            &shader,
            "mask",
            &layout1,
            "vs_mask",
            "fs_mask",
            &[inst_layout(
                std::mem::size_of::<MaskI>() as u64,
                &MASK_ATTRS,
            )],
            INTERNAL_FORMAT,
            mask_blend,
            wgpu::PrimitiveTopology::TriangleStrip,
        );
        let tilt_pl = make_pipeline(
            &device,
            &shader,
            "tilt",
            &layout2,
            "vs_tilt",
            "fs_tilt",
            &[inst_layout(
                std::mem::size_of::<TiltI>() as u64,
                &TILT_ATTRS,
            )],
            INTERNAL_FORMAT,
            premul_blend(),
            wgpu::PrimitiveTopology::TriangleStrip,
        );

        Renderer {
            device,
            queue,
            rect_pl,
            glyph_pl,
            mesh_pl,
            tex_pl,
            blur_pl,
            texband_pl,
            mask_pl,
            tilt_pl,
            blit_pls: HashMap::new(),
            shader,
            bgl_globals,
            bgl_tex,
            globals,
            globals_bg,
            grads_buf,
            grads_cpu: Vec::new(),
            sampler,
            atlas: Atlas::default(),
            atlas_tex,
            atlas_bg,
            docs: Vec::new(),
            main: None,
            pool: Vec::new(),
            notes: HashSet::new(),
            scale: 1.0,
        }
    }

    fn make_globals_bg(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        globals: &wgpu::Buffer,
        grads: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: grads.as_entire_binding(),
                },
            ],
        })
    }

    /// Register one decoded SLIR document: select runtime-registered or
    /// bundled fallback faces, append its gradient table, and upload decoded
    /// image payloads.
    pub fn register_doc(
        &mut self,
        doc: &Doc,
        imgs: &[Vec<u8>],
        registered_fonts: &[RegisteredFont],
    ) -> usize {
        let grad_base = self.grads_cpu.len() as u32;
        for g in 0..doc.grad_kind.len() {
            let lo = doc.grad_stop_off[g] as usize;
            let n = (doc.grad_stop_len[g] as usize).min(MAX_GRAD_STOPS);
            if doc.grad_stop_len[g] as usize > MAX_GRAD_STOPS {
                self.note("cap-gradient-stops", "gradients use at most 8 stops on gpu");
            }
            let mut gg = GradGpu::zeroed();
            gg.info[0] = doc.grad_kind[g];
            gg.info[1] = n as u32;
            for i in 0..n {
                gg.pos[i] = doc.grad_stop_pos[lo + i] as f32;
                gg.col[i] = rgba(doc.grad_stop_rgba[lo + i], 1.0);
            }
            self.grads_cpu.push(gg);
        }
        if self.grads_cpu.is_empty() {
            self.grads_cpu.push(GradGpu::zeroed());
        }
        self.grads_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grads"),
                contents: bytemuck::cast_slice(&self.grads_cpu),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
        self.globals_bg = Self::make_globals_bg(
            &self.device,
            &self.bgl_globals,
            &self.globals,
            &self.grads_buf,
        );

        let mut fonts = Vec::with_capacity(doc.font_upem.len());
        for f in 0..doc.font_upem.len() {
            let family = usize::try_from(doc.font_family[f])
                .ok()
                .and_then(|index| doc.strs.get(index))
                .map(String::as_str)
                .unwrap_or("");
            let target_weight = doc.font_weight[f];
            let registered = registered_fonts
                .iter()
                .enumerate()
                .filter(|(_, face)| face.name.eq_ignore_ascii_case(family))
                .min_by_key(|(index, face)| {
                    (
                        face.weight.abs_diff(target_weight),
                        std::cmp::Reverse(*index),
                    )
                })
                .map(|(_, face)| face.bytes.as_slice());
            let bytes = registered.unwrap_or_else(|| {
                slab_fonts::asset(
                    u8::try_from(doc.font_class[f]).expect("SLIR font class fits u8"),
                    u16::try_from(target_weight).expect("SLIR font weight fits u16"),
                )
                .bytes
            });
            let face = Face::from_bytes(bytes);
            if face.is_none() {
                self.note(
                    "cap-font",
                    "a FONT table has no usable native face; its glyphs are skipped",
                );
            }
            fonts.push(face);
        }

        let mut images = Vec::with_capacity(doc.img_src.len());
        for i in 0..doc.img_src.len() {
            let tex = imgs.get(i).and_then(|bytes| self.upload_png(bytes));
            if tex.is_none() {
                self.note(
                    "cap-image",
                    "an image has no decodable SLIR PNG; drawn as a placeholder",
                );
            }
            images.push(tex);
        }

        self.docs.push(DocRes {
            grad_base,
            grad_count: doc.grad_kind.len() as u32,
            fonts,
            fills: HashMap::new(),
            sq_fills: HashMap::new(),
            strokes: HashMap::new(),
            runtime_paths: Vec::new(),
            image_generations: vec![Some(0); images.len()],
            images,
        });
        self.docs.len() - 1
    }

    /// Rebuild faces for FONT tables appended by runtime registration and
    /// discard their cached glyphs.
    pub fn refresh_registered_fonts(
        &mut self,
        doc_id: usize,
        doc: &Doc,
        registered_fonts: &[RegisteredFont],
        first_font: usize,
    ) {
        let Some(resources) = self.docs.get_mut(doc_id) else {
            return;
        };
        resources.fonts.truncate(first_font);
        for f in first_font..doc.font_upem.len() {
            let family = doc
                .strs
                .get(doc.font_family[f] as usize)
                .map(String::as_str)
                .unwrap_or("");
            let target_weight = doc.font_weight[f];
            let bytes = registered_fonts
                .iter()
                .enumerate()
                .filter(|(_, face)| face.name.eq_ignore_ascii_case(family))
                .min_by_key(|(index, face)| {
                    (
                        face.weight.abs_diff(target_weight),
                        std::cmp::Reverse(*index),
                    )
                })
                .map(|(_, face)| face.bytes.as_slice())
                .unwrap_or_else(|| {
                    slab_fonts::asset(doc.font_class[f] as u8, target_weight as u16).bytes
                });
            resources.fonts.push(Face::from_bytes(bytes));
        }
        self.atlas.invalidate_doc_fonts(doc_id, first_font as i32);
    }

    fn upload_png(&self, bytes: &[u8]) -> Option<ImgTex> {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().ok()?;
        let mut buf = vec![0u8; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut buf).ok()?;
        let (w, h) = (info.width, info.height);
        let rgba_len = usize::try_from(w.checked_mul(h)?.checked_mul(4)?).ok()?;
        let mut rgba = vec![0u8; rgba_len];
        match info.color_type {
            png::ColorType::Rgba => rgba.copy_from_slice(&buf[..rgba_len]),
            png::ColorType::Rgb => {
                for (source, target) in buf.chunks_exact(3).zip(rgba.chunks_exact_mut(4)) {
                    target[..3].copy_from_slice(source);
                    target[3] = 255;
                }
            }
            png::ColorType::Grayscale => {
                for (&source, target) in buf.iter().zip(rgba.chunks_exact_mut(4)) {
                    target.copy_from_slice(&[source, source, source, 255]);
                }
            }
            png::ColorType::GrayscaleAlpha => {
                for (source, target) in buf.chunks_exact(2).zip(rgba.chunks_exact_mut(4)) {
                    target.copy_from_slice(&[source[0], source[0], source[0], source[1]]);
                }
            }
            _ => return None,
        }
        self.upload_rgba_owned(w, h, rgba)
    }

    fn upload_rgba(&self, w: u32, h: u32, bytes: &[u8]) -> Option<ImgTex> {
        self.upload_rgba_owned(w, h, bytes.to_vec())
    }

    fn upload_rgba_owned(&self, w: u32, h: u32, mut rgba: Vec<u8>) -> Option<ImgTex> {
        if w == 0 || h == 0 {
            return None;
        }
        let expected = usize::try_from(w.checked_mul(h)?.checked_mul(4)?).ok()?;
        if rgba.len() != expected {
            return None;
        }
        // The shared texture pipeline blends premultiplied colors.
        for pixel in rgba.chunks_exact_mut(4) {
            let alpha = u32::from(pixel[3]);
            pixel[0] = (u32::from(pixel[0]) * alpha / 255) as u8;
            pixel[1] = (u32::from(pixel[1]) * alpha / 255) as u8;
            pixel[2] = (u32::from(pixel[2]) * alpha / 255) as u8;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("image"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("image"),
            layout: &self.bgl_tex,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        Some(ImgTex { bind })
    }

    fn upload_image(&self, w: u32, h: u32, format: u32, bytes: &[u8]) -> Option<ImgTex> {
        match format {
            0 => self.upload_png(bytes),
            1 => self.upload_rgba(w, h, bytes),
            _ => None,
        }
    }

    fn ensure_image(&mut self, doc_id: usize, inst: &Instance, image: i32) -> bool {
        let Ok(index) = usize::try_from(image) else {
            return false;
        };
        let Some((w, h, format, generation)) = slab_kernel::frame::inst_img_info(inst, image)
        else {
            if let Some(resources) = self.docs.get_mut(doc_id)
                && index < resources.images.len()
            {
                resources.images[index] = None;
                resources.image_generations[index] = None;
            }
            return false;
        };
        if self
            .docs
            .get(doc_id)
            .and_then(|resources| resources.image_generations.get(index))
            .copied()
            .flatten()
            == Some(generation)
        {
            return self.docs[doc_id]
                .images
                .get(index)
                .is_some_and(Option::is_some);
        }
        let uploaded = self.upload_image(
            w,
            h,
            format,
            slab_kernel::frame::inst_img_bytes(inst, image),
        );
        let Some(resources) = self.docs.get_mut(doc_id) else {
            return false;
        };
        if resources.images.len() <= index {
            resources.images.resize_with(index + 1, || None);
            resources.image_generations.resize(index + 1, None);
        }
        resources.images[index] = uploaded;
        resources.image_generations[index] = Some(generation);
        resources.images[index].is_some()
    }

    fn sync_runtime_images(&mut self, doc_id: usize, inst: &Instance) {
        let start = inst.doc.img_src.len();
        let end = self
            .docs
            .get(doc_id)
            .map(|resources| resources.images.len())
            .unwrap_or(start);
        for index in start..end {
            let image = i32::try_from(index).expect("image index exceeds i32");
            self.ensure_image(doc_id, inst, image);
        }
    }

    // ------------------------------------------------------------ build ----

    /// Walk kernel frames into instance lists + steps for a `tw`x`th`
    /// device-px target at `scale` device px per logical unit.
    pub fn build(&mut self, layers: &[LayerInput<'_>], scale: f64, tw: u32, th: u32) -> FrameBuild {
        self.scale = scale;
        let mut fb = FrameBuild {
            rects: Vec::new(),
            glyphs: Vec::new(),
            meshes: Vec::new(),
            texq: Vec::new(),
            steps: Vec::new(),
            tw,
            th,
        };
        for layer in layers {
            self.build_layer(&mut fb, layer, scale as f32, tw, th);
        }
        fb
    }

    #[allow(clippy::too_many_lines)]
    fn build_layer(&mut self, fb: &mut FrameBuild, li: &LayerInput<'_>, s: f32, tw: u32, th: u32) {
        self.sync_runtime_images(li.doc_id, li.inst);
        let doc = &li.inst.doc;
        let full: Sc = (0, 0, tw, th);
        let huge = [-1.0e9f32, -1.0e9, 1.0e9, 1.0e9];
        let (ox, oy) = (li.ox as f32, li.oy as f32);
        let mut mats = vec![Mat::I];
        let mut clips = vec![ClipSt {
            sdf: huge,
            radius: 0.0,
            scissor: full,
        }];
        if let Some((cx, cy, cw, ch, cr)) = li.clip {
            let x0 = cx as f32 * s;
            let y0 = cy as f32 * s;
            let x1 = (cx + cw) as f32 * s;
            let y1 = (cy + ch) as f32 * s;
            clips.push(ClipSt {
                sdf: [x0, y0, x1, y1],
                radius: cr as f32 * s,
                scissor: sc_intersect(
                    full,
                    (
                        x0.floor().max(0.0) as u32,
                        y0.floor().max(0.0) as u32,
                        (x1 - x0).ceil().max(0.0) as u32 + 1,
                        (y1 - y0).ceil().max(0.0) as u32 + 1,
                    ),
                ),
            });
        }
        let mut groups: Vec<(f32, f32, Option<MaskI>)> = Vec::new();
        let mut tilts: Vec<TiltI> = Vec::new();
        let mut hidden_groups = 0usize;

        for (op_ix, op) in li.frame.ops.iter().enumerate() {
            if hidden_groups > 0 {
                match op {
                    FrameOp::GroupPush(_) | FrameOp::TiltPush(_) => hidden_groups += 1,
                    FrameOp::GroupPop | FrameOp::TiltPop => hidden_groups -= 1,
                    _ => {}
                }
                continue;
            }
            if matches!(op, FrameOp::GroupPush(group) if group.opacity <= 0.0) {
                hidden_groups = 1;
                continue;
            }
            let mat = *mats.last().unwrap();
            let clip = *clips.last().unwrap();
            match op {
                FrameOp::Rect(r) => {
                    let shadow_lo = (r.shadow_off.max(0) as usize).min(doc.shdw_x.len());
                    let shadow_hi = (r.shadow_off.saturating_add(r.shadow_len).max(0) as usize)
                        .min(doc.shdw_x.len())
                        .max(shadow_lo);
                    let shadows = shadow_lo..shadow_hi;
                    let has_inset = shadows.clone().any(|i| doc.shdw_inset[i] != 0);
                    let shadow_inst = |i: usize| {
                        let inset = doc.shdw_inset[i] != 0;
                        let sigma = (doc.shdw_blur[i] as f32 * s / 2.0).max(0.25);
                        let (cx, cy, hx, hy, radius, kind, dx, dy) = if inset {
                            (
                                ((r.x + r.w / 2.0) as f32 + ox) * s,
                                ((r.y + r.h / 2.0) as f32 + oy) * s,
                                (r.w / 2.0) as f32 * s,
                                (r.h / 2.0) as f32 * s,
                                r.radius as f32 * s,
                                -2.0,
                                doc.shdw_x[i] as f32 * s,
                                doc.shdw_y[i] as f32 * s,
                            )
                        } else {
                            let spread = doc.shdw_spread[i] as f32 * s;
                            (
                                ((r.x + r.w / 2.0) as f32 + doc.shdw_x[i] as f32 + ox) * s,
                                ((r.y + r.h / 2.0) as f32 + doc.shdw_y[i] as f32 + oy) * s,
                                (r.w / 2.0) as f32 * s + spread,
                                (r.h / 2.0) as f32 * s + spread,
                                r.radius as f32 * s + spread,
                                -1.0,
                                0.0,
                                0.0,
                            )
                        };
                        RectI {
                            mabcd: [mat.a, mat.b, mat.c, mat.d],
                            mtc: [mat.tx, mat.ty, cx, cy],
                            hrs: [hx, hy, radius, 0.0],
                            sg: [0.0, sigma, kind, dx],
                            dc: [dy, clip.radius, clip.sdf[0], clip.sdf[1]],
                            c2: [clip.sdf[2], clip.sdf[3], 0.0, 0.0],
                            fill: rgba(doc.shdw_rgba[i], r.opacity),
                            stroke: [0.0; 4],
                            g2: [0.0, 1.0, -1.0, 0.0],
                        }
                    };

                    for i in shadows.clone().filter(|&i| doc.shdw_inset[i] == 0) {
                        fb.push_rect(clip.scissor, shadow_inst(i));
                    }

                    let mut fill = [0.0f32; 4];
                    let mut grad = -1.0f32;
                    let mut dir = [0.0f32, 0.0];
                    match r.bg_kind {
                        1 => fill = rgba(r.bg, r.opacity),
                        2 if r.bg < self.docs[li.doc_id].grad_count => {
                            grad = (self.docs[li.doc_id].grad_base + r.bg) as f32;
                            let g = r.bg as usize;
                            dir = grad_dir(
                                doc.grad_kind[g],
                                doc.grad_angle[g],
                                r.w as f32 * s,
                                r.h as f32 * s,
                            );
                            fill = [0.0, 0.0, 0.0, r.opacity as f32];
                        }
                        _ => {}
                    }
                    let mut stroke = [0.0f32; 4];
                    let mut stroke_grad = -1.0f32;
                    let mut hw = 0.0f32;
                    let mut soff = 0.0f32;
                    if r.stroke_kind != 0 && r.stroke_w > 0.0 {
                        match r.stroke_kind {
                            1 => stroke = rgba(r.stroke, r.opacity),
                            _ => {
                                if r.stroke < self.docs[li.doc_id].grad_count {
                                    stroke_grad =
                                        (self.docs[li.doc_id].grad_base + r.stroke) as f32;
                                    let g = r.stroke as usize;
                                    let sdir = grad_dir(
                                        doc.grad_kind[g],
                                        doc.grad_angle[g],
                                        r.w as f32 * s,
                                        r.h as f32 * s,
                                    );
                                    stroke = [sdir[0], sdir[1], r.opacity as f32, 0.0];
                                }
                            }
                        }
                        hw = (r.stroke_w / 2.0) as f32 * s;
                        soff = match r.stroke_align {
                            1 => -hw,
                            2 => hw,
                            _ => 0.0,
                        };
                    }
                    let use_squircle = r.smooth > 0.0 && r.radius > 0.0;
                    if use_squircle && !shadows.is_empty() {
                        self.note(
                            "cap-smooth",
                            "shadows and clips keep circular corners under smooth on gpu",
                        );
                    }
                    let sides_partial = r.stroke_sides != 15 && hw > 0.0;
                    let grad_sides = sides_partial && stroke_grad >= 0.0;
                    let solid_full_stroke =
                        hw > 0.0 && !sides_partial && !r.has_dash && !use_squircle;
                    let combine_stroke = solid_full_stroke && !has_inset;
                    let cx = ((r.x + r.w / 2.0) as f32 + ox) * s;
                    let cy = ((r.y + r.h / 2.0) as f32 + oy) * s;
                    let inst = RectI {
                        mabcd: [mat.a, mat.b, mat.c, mat.d],
                        mtc: [mat.tx, mat.ty, cx, cy],
                        hrs: [
                            (r.w / 2.0) as f32 * s,
                            (r.h / 2.0) as f32 * s,
                            r.radius as f32 * s,
                            if combine_stroke { hw } else { 0.0 },
                        ],
                        sg: [soff, 0.0, grad, dir[0]],
                        dc: [dir[1], clip.radius, clip.sdf[0], clip.sdf[1]],
                        c2: [clip.sdf[2], clip.sdf[3], 0.0, 0.0],
                        fill,
                        stroke,
                        g2: [
                            0.0,
                            1.0,
                            if combine_stroke { stroke_grad } else { -1.0 },
                            0.0,
                        ],
                    };
                    if use_squircle && (fill[3] > 0.0 || grad >= 0.0) {
                        // squircle fill rides the mesh pipeline (contract 6.4)
                        let key = self.ensure_squircle_fill_mesh(li.doc_id, r);
                        fb.push_mesh(
                            clip.scissor,
                            li.doc_id,
                            MeshKey::Squircle(key),
                            MeshI {
                                mabcd: [mat.a, mat.b, mat.c, mat.d],
                                mto: [mat.tx, mat.ty, (r.x as f32 + ox) * s, (r.y as f32 + oy) * s],
                                sc: [s, clip.radius, grad, r.opacity as f32],
                                clip: clip.sdf,
                                color: if grad >= 0.0 {
                                    [cx, cy, dir[0], dir[1]]
                                } else {
                                    fill
                                },
                            },
                        );
                    } else if fill[3] > 0.0 || grad >= 0.0 || combine_stroke {
                        fb.push_rect(clip.scissor, inst);
                    }

                    for i in shadows.clone().filter(|&i| doc.shdw_inset[i] != 0) {
                        fb.push_rect(clip.scissor, shadow_inst(i));
                    }

                    if solid_full_stroke && has_inset {
                        fb.push_rect(
                            clip.scissor,
                            RectI {
                                hrs: [inst.hrs[0], inst.hrs[1], inst.hrs[2], hw],
                                sg: [soff, 0.0, -1.0, 0.0],
                                fill: [0.0; 4],
                                g2: [0.0, 1.0, stroke_grad, 0.0],
                                ..inst
                            },
                        );
                    }

                    let stroke_mesh_route =
                        hw > 0.0 && (r.has_dash || (use_squircle && !sides_partial) || grad_sides);
                    if stroke_mesh_route {
                        let key = self.ensure_rect_stroke_mesh(li.doc_id, r);
                        fb.push_mesh(
                            clip.scissor,
                            li.doc_id,
                            MeshKey::Stroke(key),
                            MeshI {
                                mabcd: [mat.a, mat.b, mat.c, mat.d],
                                mto: [mat.tx, mat.ty, (r.x as f32 + ox) * s, (r.y as f32 + oy) * s],
                                sc: [s, clip.radius, stroke_grad, r.opacity as f32],
                                clip: clip.sdf,
                                color: if stroke_grad >= 0.0 {
                                    [cx, cy, stroke[0], stroke[1]]
                                } else {
                                    stroke
                                },
                            },
                        );
                    } else if sides_partial {
                        // per-side bars (axis-aligned; radius ignored)
                        let sw = r.stroke_w as f32 * s;
                        let (x0, y0) = ((r.x as f32 + ox) * s, (r.y as f32 + oy) * s);
                        let (x1, y1) =
                            (((r.x + r.w) as f32 + ox) * s, ((r.y + r.h) as f32 + oy) * s);
                        let shift = match r.stroke_align {
                            1 => 0.0, // inside: bar sits within the edge
                            2 => -sw, // outside
                            _ => -sw / 2.0,
                        };
                        let bars = [
                            (r.stroke_sides & 1, x0, y0 + shift, x1 - x0, sw),
                            (r.stroke_sides & 2, x1 - sw - shift, y0, sw, y1 - y0),
                            (r.stroke_sides & 4, x0, y1 - sw - shift, x1 - x0, sw),
                            (r.stroke_sides & 8, x0 + shift, y0, sw, y1 - y0),
                        ];
                        for (on, bx, by, bw, bh) in bars {
                            if on == 0 {
                                continue;
                            }
                            let inst = RectI {
                                mabcd: [mat.a, mat.b, mat.c, mat.d],
                                mtc: [mat.tx, mat.ty, bx + bw / 2.0, by + bh / 2.0],
                                hrs: [bw / 2.0, bh / 2.0, 0.0, 0.0],
                                sg: [0.0, 0.0, -1.0, 0.0],
                                dc: [0.0, clip.radius, clip.sdf[0], clip.sdf[1]],
                                c2: [clip.sdf[2], clip.sdf[3], 0.0, 0.0],
                                fill: stroke,
                                stroke: [0.0; 4],
                                g2: [0.0, 1.0, -1.0, 0.0],
                            };
                            fb.push_rect(clip.scissor, inst);
                        }
                    }

                    if r.grain_amount > 0.0 {
                        // grain speckle layer (contract 6.2), painted after
                        // fill, shadows, and strokes; clipped by the rect SDF
                        // (smooth corners stay circular here, chart-noted).
                        fb.push_rect(
                            clip.scissor,
                            RectI {
                                hrs: [inst.hrs[0], inst.hrs[1], inst.hrs[2], 0.0],
                                sg: [0.0, 0.0, -1.0, 0.0],
                                fill: [0.0; 4],
                                stroke: [0.0; 4],
                                g2: [
                                    r.grain_amount as f32,
                                    (r.grain_size.max(1e-3) * f64::from(s)) as f32,
                                    -1.0,
                                    r.opacity as f32,
                                ],
                                ..inst
                            },
                        );
                    }
                }
                FrameOp::Text(t) => {
                    if t.font < 0 {
                        continue;
                    }
                    let mut color = [0.0f32; 4];
                    let mut tg2 = [0.0f32, 0.0, -1.0, 0.0];
                    if t.color_kind == 2 {
                        if t.color < self.docs[li.doc_id].grad_count {
                            // gradient ink over the node content box
                            // (contract 6.7); the kernel populates gx..gh
                            let g = t.color as usize;
                            let dir = grad_dir(
                                doc.grad_kind[g],
                                doc.grad_angle[g],
                                t.gw as f32 * s,
                                t.gh as f32 * s,
                            );
                            color = [dir[0], dir[1], 0.0, 0.0];
                            tg2 = [
                                ((t.gx + t.gw / 2.0) as f32 + ox) * s,
                                ((t.gy + t.gh / 2.0) as f32 + oy) * s,
                                (self.docs[li.doc_id].grad_base + t.color) as f32,
                                t.opacity as f32,
                            ];
                        }
                    } else {
                        color = rgba(t.color, t.opacity);
                    }
                    let px = (t.size * self.scale) as f32;
                    let glyphs = slab_kernel::frame::text_glyphs(li.inst, li.frame, op_ix as i32);
                    for g in &glyphs {
                        let have_face = self.docs[li.doc_id]
                            .fonts
                            .get(g.font as usize)
                            .is_some_and(|f| f.is_some());
                        if !have_face {
                            self.note(
                                "cap-font",
                                "FONT table without usable native face; text skipped",
                            );
                            break;
                        }
                        let Some(e) = self.atlas_entry(li.doc_id, g.font, g.gid, px) else {
                            continue;
                        };
                        let gx = ((g.x as f32 + ox) * s + e.bearing[0]).round();
                        let gy = ((g.y as f32 + oy) * s + e.bearing[1]).round();
                        let inst = GlyphI {
                            mabcd: [mat.a, mat.b, mat.c, mat.d],
                            mtp: [mat.tx, mat.ty, gx, gy],
                            su: [e.size[0], e.size[1], e.uv[0], e.uv[1]],
                            uc: [e.uv[2], e.uv[3], clip.radius, 0.0],
                            clip: clip.sdf,
                            color,
                            g2: tg2,
                        };
                        fb.push_glyph(clip.scissor, inst);
                    }
                }
                FrameOp::Image(im) => {
                    let cx = ((im.x + im.w / 2.0) as f32 + ox) * s;
                    let cy = ((im.y + im.h / 2.0) as f32 + oy) * s;
                    if im.smooth > 0.0 && im.radius > 0.0 {
                        self.note(
                            "cap-smooth",
                            "shadows and clips keep circular corners under smooth on gpu",
                        );
                    }
                    let has = self.ensure_image(li.doc_id, li.inst, im.img);
                    if !has {
                        self.note(
                            "cap-image",
                            "an image has no decodable runtime payload; drawn as a placeholder",
                        );
                        // gray placeholder box (raster draws checker + label)
                        let inst = RectI {
                            mabcd: [mat.a, mat.b, mat.c, mat.d],
                            mtc: [mat.tx, mat.ty, cx, cy],
                            hrs: [
                                (im.w / 2.0) as f32 * s,
                                (im.h / 2.0) as f32 * s,
                                im.radius as f32 * s,
                                0.0,
                            ],
                            sg: [0.0, 0.0, -1.0, 0.0],
                            dc: [0.0, clip.radius, clip.sdf[0], clip.sdf[1]],
                            c2: [clip.sdf[2], clip.sdf[3], 0.0, 0.0],
                            fill: rgba(0xFFD6CEC9, im.opacity), // #c9ced6
                            stroke: [0.0; 4],
                            g2: [0.0, 1.0, -1.0, 0.0],
                        };
                        fb.push_rect(clip.scissor, inst);
                        continue;
                    }
                    let inst_ix = fb.texq.len() as u32;
                    let uv = image_uv(li.inst, im);
                    let inst = TexI {
                        mabcd: [mat.a, mat.b, mat.c, mat.d],
                        mtc: [mat.tx, mat.ty, cx, cy],
                        hro: [
                            (im.w / 2.0) as f32 * s,
                            (im.h / 2.0) as f32 * s,
                            im.radius as f32 * s,
                            im.opacity as f32,
                        ],
                        uv,
                        clip: clip.sdf,
                        misc: [clip.radius, 1.0, 1.0, 1.0],
                    };
                    fb.texq.push(inst);
                    fb.steps.push(Step::Image {
                        scissor: clip.scissor,
                        doc: li.doc_id,
                        img: im.img as usize,
                        inst: inst_ix,
                    });
                }
                FrameOp::PathDraw(p) => {
                    let runtime_path = if p.path < 0 {
                        let Some(path) = usize::try_from(!p.path)
                            .ok()
                            .and_then(|index| li.frame.paths_rt.get(index))
                        else {
                            continue;
                        };
                        Some(path)
                    } else {
                        None
                    };
                    let mesh_path = runtime_path
                        .map(|path| self.runtime_path_key(li.doc_id, path))
                        .unwrap_or(p.path);
                    // paint = (color slot, grad tag): solid rgba, or the
                    // gradient box geometry over the path's coordinate bbox
                    // (matching the raster exporter's shader box)
                    let bounds = path_bounds(doc, runtime_path, p.path);
                    let paint = |kind: u32, h: u32, rr: &Renderer| -> Option<([f32; 4], f32)> {
                        match kind {
                            1 => Some((rgba(h, p.opacity), -1.0)),
                            2 => {
                                let b = bounds?;
                                if h >= rr.docs[li.doc_id].grad_count {
                                    return None;
                                }
                                let tag = (rr.docs[li.doc_id].grad_base + h) as f32;
                                let g = h as usize;
                                let dir = grad_dir(
                                    doc.grad_kind[g],
                                    doc.grad_angle[g],
                                    (b[2] * f64::from(s)) as f32,
                                    (b[3] * f64::from(s)) as f32,
                                );
                                let bcx = ((p.dx + b[0] + b[2] / 2.0) as f32 + ox) * s;
                                let bcy = ((p.dy + b[1] + b[3] / 2.0) as f32 + oy) * s;
                                Some(([bcx, bcy, dir[0], dir[1]], tag))
                            }
                            _ => None,
                        }
                    };
                    let fill_c = paint(p.bg_kind, p.bg, self);
                    let stroke_c = if p.stroke_w > 0.0 {
                        paint(p.stroke_kind, p.stroke, self)
                    } else {
                        None
                    };
                    let mto = [
                        mat.tx,
                        mat.ty,
                        (p.dx as f32 + ox) * s,
                        (p.dy as f32 + oy) * s,
                    ];
                    let draw = |fb: &mut FrameBuild, key: MeshKey, color: [f32; 4], tag: f32| {
                        fb.push_mesh(
                            clip.scissor,
                            li.doc_id,
                            key,
                            MeshI {
                                mabcd: [mat.a, mat.b, mat.c, mat.d],
                                mto,
                                sc: [s, clip.radius, tag, p.opacity as f32],
                                clip: clip.sdf,
                                color,
                            },
                        );
                    };
                    if let Some((color, tag)) = fill_c {
                        if let Some(path) = runtime_path {
                            self.ensure_runtime_fill_mesh(li.doc_id, mesh_path, path);
                        } else {
                            self.ensure_fill_mesh(li.doc_id, doc, mesh_path);
                        }
                        draw(fb, MeshKey::Fill(mesh_path), color, tag);
                    }
                    if let Some((color, tag)) = stroke_c {
                        let dash = p.has_dash.then_some((p.dash_on, p.dash_off));
                        let key = if let Some(path) = runtime_path {
                            self.ensure_runtime_stroke_mesh(
                                li.doc_id, mesh_path, path, p.stroke_w, dash,
                            )
                        } else {
                            self.ensure_path_stroke_mesh(
                                li.doc_id, doc, mesh_path, p.stroke_w, dash,
                            )
                        };
                        draw(fb, MeshKey::Stroke(key), color, tag);
                    }
                }
                FrameOp::ClipPush(c) => {
                    let x0 = (c.x as f32 + ox) * s;
                    let y0 = (c.y as f32 + oy) * s;
                    let x1 = ((c.x + c.w) as f32 + ox) * s;
                    let y1 = ((c.y + c.h) as f32 + oy) * s;
                    if c.smooth > 0.0 && c.radius > 0.0 {
                        self.note(
                            "cap-smooth",
                            "shadows and clips keep circular corners under smooth on gpu",
                        );
                    }
                    let (sdf, radius) = if mat.is_identity() {
                        (
                            [
                                x0.max(clip.sdf[0]),
                                y0.max(clip.sdf[1]),
                                x1.min(clip.sdf[2]),
                                y1.min(clip.sdf[3]),
                            ],
                            c.radius as f32 * s,
                        )
                    } else {
                        // rotated clip: AABB of the transformed rect
                        if c.radius > 0.0 {
                            self.note(
                                "cap-clip-rotated",
                                "rounded clips under rotation clip their AABB on gpu",
                            );
                        }
                        let pts = [
                            mat.apply(x0, y0),
                            mat.apply(x1, y0),
                            mat.apply(x0, y1),
                            mat.apply(x1, y1),
                        ];
                        let bx0 = pts.iter().map(|p| p.0).fold(f32::MAX, f32::min);
                        let by0 = pts.iter().map(|p| p.1).fold(f32::MAX, f32::min);
                        let bx1 = pts.iter().map(|p| p.0).fold(f32::MIN, f32::max);
                        let by1 = pts.iter().map(|p| p.1).fold(f32::MIN, f32::max);
                        (
                            [
                                bx0.max(clip.sdf[0]),
                                by0.max(clip.sdf[1]),
                                bx1.min(clip.sdf[2]),
                                by1.min(clip.sdf[3]),
                            ],
                            0.0,
                        )
                    };
                    let sx0 = sdf[0].floor().max(0.0) as u32;
                    let sy0 = sdf[1].floor().max(0.0) as u32;
                    let sx1 = sdf[2].ceil().max(0.0) as u32 + 1;
                    let sy1 = sdf[3].ceil().max(0.0) as u32 + 1;
                    clips.push(ClipSt {
                        sdf,
                        radius,
                        scissor: sc_intersect(
                            clip.scissor,
                            (sx0, sy0, sx1.saturating_sub(sx0), sy1.saturating_sub(sy0)),
                        ),
                    });
                }
                FrameOp::ClipPop => {
                    if clips.len() > 1 {
                        clips.pop();
                    }
                }
                FrameOp::GroupPush(g) => {
                    let mask = (g.mask_kind != 0).then(|| {
                        // mask box in device px; under an ancestor transform
                        // the box degrades to its AABB (like rotated clips)
                        let x0 = ((g.mx as f32) + ox) * s;
                        let y0 = ((g.my as f32) + oy) * s;
                        let x1 = ((g.mx + g.mw) as f32 + ox) * s;
                        let y1 = ((g.my + g.mh) as f32 + oy) * s;
                        let (bx0, by0, bx1, by1) = if mat.is_identity() {
                            (x0, y0, x1, y1)
                        } else {
                            self.note(
                                "cap-mask-rotated",
                                "mask boxes under rotation use their AABB on gpu",
                            );
                            let pts = [
                                mat.apply(x0, y0),
                                mat.apply(x1, y0),
                                mat.apply(x0, y1),
                                mat.apply(x1, y1),
                            ];
                            (
                                pts.iter().map(|p| p.0).fold(f32::MAX, f32::min),
                                pts.iter().map(|p| p.1).fold(f32::MAX, f32::min),
                                pts.iter().map(|p| p.0).fold(f32::MIN, f32::max),
                                pts.iter().map(|p| p.1).fold(f32::MIN, f32::max),
                            )
                        };
                        MaskI {
                            rect: [0.0, 0.0, tw as f32, th as f32],
                            bx: [
                                (bx0 + bx1) / 2.0,
                                (by0 + by1) / 2.0,
                                (bx1 - bx0) / 2.0,
                                (by1 - by0) / 2.0,
                            ],
                            grad: self.paint_ref(
                                li.doc_id,
                                doc,
                                g.mask_kind,
                                g.mask,
                                bx1 - bx0,
                                by1 - by0,
                            ),
                        }
                    });
                    groups.push((g.opacity as f32, (g.blur * self.scale) as f32, mask));
                    fb.steps.push(Step::PushLayer);
                }
                FrameOp::GroupPop => {
                    let (opacity, sigma, mask) = groups.pop().unwrap_or((1.0, 0.0, None));
                    fb.steps.push(Step::PopLayer {
                        opacity,
                        sigma,
                        mask,
                    });
                }
                FrameOp::RotatePush(r) => {
                    let m = mat.then(Mat::rotate_about(
                        ((r.cx as f32) + ox) * s,
                        ((r.cy as f32) + oy) * s,
                        r.deg as f32,
                    ));
                    mats.push(m);
                }
                FrameOp::RotatePop => {
                    if mats.len() > 1 {
                        mats.pop();
                    }
                }
                FrameOp::ScalePush(scale) => {
                    let matrix = mat.then(Mat::scale_about(
                        ((scale.cx as f32) + ox) * s,
                        ((scale.cy as f32) + oy) * s,
                        scale.sx as f32,
                        scale.sy as f32,
                    ));
                    mats.push(matrix);
                }
                FrameOp::ScalePop => {
                    if mats.len() > 1 {
                        mats.pop();
                    }
                }
                FrameOp::Backdrop(b) => {
                    let x0 = (b.x as f32 + ox) * s;
                    let y0 = (b.y as f32 + oy) * s;
                    let x1 = ((b.x + b.w) as f32 + ox) * s;
                    let y1 = ((b.y + b.h) as f32 + oy) * s;
                    if b.smooth > 0.0 && b.radius > 0.0 {
                        self.note(
                            "cap-smooth",
                            "shadows and clips keep circular corners under smooth on gpu",
                        );
                    }
                    let mask = (b.mask_kind != 0).then(|| {
                        self.paint_ref(li.doc_id, doc, b.mask_kind, b.mask, x1 - x0, y1 - y0)
                    });
                    fb.steps.push(Step::Backdrop {
                        rect: [x0, y0, x1, y1],
                        radius: b.radius as f32 * s,
                        sigma: (b.blur * self.scale) as f32,
                        saturate: b.saturate as f32,
                        brightness: b.brightness as f32,
                        mask,
                        scissor: clip.scissor,
                    });
                }
                FrameOp::TiltPush(t) => {
                    // ink-only perspective (contract 6.5): the subtree
                    // renders into a layer; TiltPop composites the projected
                    // quad. Ancestor transforms sandwich the projection
                    // (content bakes `mat` in, so warp mat . P . mat^-1).
                    let cx = ((t.cx as f32) + ox) * s;
                    let cy = ((t.cy as f32) + oy) * s;
                    let depth = ((t.depth * f64::from(s)) as f32).max(1.0);
                    let (sin_ry, cos_ry) = (t.ry as f32).to_radians().sin_cos();
                    let (sin_rx, cos_rx) = (t.rx as f32).to_radians().sin_cos();
                    let inv = mat.invert().unwrap_or(Mat::I);
                    let project = |px: f32, py: f32| -> (f32, f32, f32) {
                        let (ux, uy) = inv.apply(px, py);
                        let (x, y) = (ux - cx, uy - cy);
                        let x1 = x * cos_ry;
                        let z1 = -x * sin_ry;
                        let y2 = y * cos_rx - z1 * sin_rx;
                        let z2 = y * sin_rx + z1 * cos_rx;
                        let zc = z2.min(0.95 * depth);
                        let sc = depth / (depth - zc);
                        let (wx, wy) = mat.apply(cx + x1 * sc, cy + y2 * sc);
                        // homogeneous weight = 1/view-depth = the projection
                        // scale itself; verified exact for the strip split
                        (wx, wy, sc)
                    };
                    let (tw_f, th_f) = (tw as f32, th as f32);
                    let corners = [
                        project(0.0, 0.0),
                        project(tw_f, 0.0),
                        project(0.0, th_f),
                        project(tw_f, th_f),
                    ];
                    tilts.push(TiltI {
                        p01: [corners[0].0, corners[0].1, corners[1].0, corners[1].1],
                        p23: [corners[2].0, corners[2].1, corners[3].0, corners[3].1],
                        ws: [corners[0].2, corners[1].2, corners[2].2, corners[3].2],
                        clip: clip.sdf,
                        misc: [clip.radius, 0.0, 0.0, 0.0],
                    });
                    fb.steps.push(Step::PushLayer);
                }
                FrameOp::TiltPop => {
                    if let Some(inst) = tilts.pop() {
                        fb.steps.push(Step::PopTilt { inst });
                    }
                }
            }
        }
    }

    fn atlas_entry(
        &mut self,
        doc_id: usize,
        font: i32,
        gid: u32,
        px: f32,
    ) -> Option<crate::atlas::GlyphEntry> {
        // Take the face out to split the borrow with the atlas, then restore.
        let face = self.docs[doc_id].fonts[font as usize].take()?;
        let e = self.atlas.entry(doc_id, font, &face, gid, px);
        self.docs[doc_id].fonts[font as usize] = Some(face);
        e
    }

    fn runtime_path_key(&mut self, doc_id: usize, path: &RtPath) -> i32 {
        if let Some(index) = self.docs[doc_id]
            .runtime_paths
            .iter()
            .position(|candidate| candidate.verbs == path.verbs && candidate.coords == path.coords)
        {
            return !i32::try_from(index).expect("runtime path index exceeds i32");
        }
        let index = self.docs[doc_id].runtime_paths.len();
        self.docs[doc_id].runtime_paths.push(path.clone());
        !i32::try_from(index).expect("runtime path index exceeds i32")
    }

    fn ensure_runtime_fill_mesh(&mut self, doc_id: usize, key: i32, path: &RtPath) {
        if !self.docs[doc_id].fills.contains_key(&key) {
            let mesh = fill_mesh_data(&self.device, &path.verbs, &path.coords);
            self.docs[doc_id].fills.insert(key, mesh);
        }
    }

    fn ensure_runtime_stroke_mesh(
        &mut self,
        doc_id: usize,
        path_key: i32,
        path: &RtPath,
        width: f64,
        dash: Option<(f64, f64)>,
    ) -> StrokeKey {
        let key = StrokeKey::Path {
            path: path_key,
            width: mesh_scalar_key(width),
            dash: dash.map(dash_key),
        };
        if !self.docs[doc_id].strokes.contains_key(&key) {
            let mesh = stroke_mesh_data(&self.device, &path.verbs, &path.coords, width, dash);
            self.docs[doc_id].strokes.insert(key, mesh);
        }
        key
    }

    fn ensure_fill_mesh(&mut self, doc_id: usize, doc: &Doc, path: i32) {
        if !self.docs[doc_id].fills.contains_key(&path) {
            let m = fill_mesh(&self.device, doc, path);
            self.docs[doc_id].fills.insert(path, m);
        }
    }

    fn ensure_path_stroke_mesh(
        &mut self,
        doc_id: usize,
        doc: &Doc,
        path: i32,
        width: f64,
        dash: Option<(f64, f64)>,
    ) -> StrokeKey {
        let key = StrokeKey::Path {
            path,
            width: mesh_scalar_key(width),
            dash: dash.map(dash_key),
        };
        if !self.docs[doc_id].strokes.contains_key(&key) {
            let mesh = stroke_mesh(&self.device, doc, path, width, dash);
            self.docs[doc_id].strokes.insert(key, mesh);
        }
        key
    }

    /// Resolve a `(kind, handle)` SLIR paint into `fs_mask`/`fs_texband`
    /// params — `[grad tag, dir.x, dir.y, solid alpha]` over a `wd`x`hd`
    /// device-px box. Only the paint's ALPHA is consumed (contract 6.3).
    fn paint_ref(
        &self,
        doc_id: usize,
        doc: &Doc,
        kind: u32,
        handle: u32,
        wd: f32,
        hd: f32,
    ) -> [f32; 4] {
        if kind == 2 && handle < self.docs[doc_id].grad_count {
            let tag = (self.docs[doc_id].grad_base + handle) as f32;
            let g = handle as usize;
            let dir = grad_dir(doc.grad_kind[g], doc.grad_angle[g], wd, hd);
            [tag, dir[0], dir[1], 1.0]
        } else if kind == 1 {
            [-1.0, 0.0, 0.0, f32::from(handle.to_le_bytes()[3]) / 255.0]
        } else {
            // absent or unknown paints behave as a full-alpha mask
            [-1.0, 0.0, 0.0, 1.0]
        }
    }

    /// Cache the squircle fill mesh for a rect's quantized geometry
    /// (contract 6.4) and return its cache key.
    fn ensure_squircle_fill_mesh(
        &mut self,
        doc_id: usize,
        rect: &slab_kernel::flatten::OpRect,
    ) -> [i64; 4] {
        let key = [
            mesh_scalar_key(rect.w),
            mesh_scalar_key(rect.h),
            mesh_scalar_key(rect.radius),
            mesh_scalar_key(rect.smooth),
        ];
        if !self.docs[doc_id].sq_fills.contains_key(&key) {
            let mesh = squircle_fill_mesh(&self.device, rect.w, rect.h, rect.radius, rect.smooth);
            self.docs[doc_id].sq_fills.insert(key, mesh);
        }
        key
    }

    fn ensure_rect_stroke_mesh(
        &mut self,
        doc_id: usize,
        rect: &slab_kernel::flatten::OpRect,
    ) -> StrokeKey {
        let dash = rect
            .has_dash
            .then_some((rect.dash_on, rect.dash_off))
            .map(dash_key)
            .unwrap_or([-1, -1]);
        let key = StrokeKey::Rect {
            size: [mesh_scalar_key(rect.w), mesh_scalar_key(rect.h)],
            radius: mesh_scalar_key(rect.radius),
            width: mesh_scalar_key(rect.stroke_w),
            align: rect.stroke_align,
            sides: rect.stroke_sides,
            dash,
            smooth: mesh_scalar_key(rect.smooth),
        };
        if !self.docs[doc_id].strokes.contains_key(&key) {
            let mesh = rect_stroke_mesh(&self.device, rect);
            self.docs[doc_id].strokes.insert(key, mesh);
        }
        key
    }

    // ----------------------------------------------------------- render ----

    fn make_target(&self, tw: u32, th: u32) -> Target {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("layer"),
            size: wgpu::Extent3d {
                width: tw.max(1),
                height: th.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: INTERNAL_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer"),
            layout: &self.bgl_tex,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        Target { tex, view, bind }
    }

    fn blit_pipeline(&mut self, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        if !self.blit_pls.contains_key(&format) {
            let layout = self
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("blit"),
                    bind_group_layouts: &[&self.bgl_globals, &self.bgl_tex],
                    push_constant_ranges: &[],
                });
            let pl = make_pipeline(
                &self.device,
                &self.shader,
                "blit",
                &layout,
                "vs_tex",
                "fs_tex",
                &[inst_layout(std::mem::size_of::<TexI>() as u64, &TEX_ATTRS)],
                format,
                wgpu::BlendState::REPLACE,
                wgpu::PrimitiveTopology::TriangleStrip,
            );
            self.blit_pls.insert(format, pl);
        }
        &self.blit_pls[&format]
    }

    /// Execute a build into the internal target; optionally blit to a
    /// surface view. `clear` is the base color of the internal target.
    pub fn render(
        &mut self,
        fb: &FrameBuild,
        surface: Option<(&wgpu::TextureView, wgpu::TextureFormat)>,
        clear: wgpu::Color,
    ) {
        let (tw, th) = (fb.tw.max(1), fb.th.max(1));
        if self
            .main
            .as_ref()
            .map(|(w, h, _)| (*w, *h) != (tw, th))
            .unwrap_or(true)
        {
            self.main = Some((tw, th, self.make_target(tw, th)));
            self.pool.clear();
        }
        // pre-size the layer pool: max Push depth + 2 aux for blur/backdrop
        let mut depth = 0usize;
        let mut max_depth = 0usize;
        let mut need_aux = false;
        for st in &fb.steps {
            match st {
                Step::PushLayer => {
                    depth += 1;
                    max_depth = max_depth.max(depth);
                }
                Step::PopLayer { sigma, .. } => {
                    depth = depth.saturating_sub(1);
                    if *sigma > 0.0 {
                        need_aux = true;
                    }
                }
                Step::PopTilt { .. } => {
                    depth = depth.saturating_sub(1);
                }
                Step::Backdrop { .. } => need_aux = true,
                _ => {}
            }
        }
        let pool_n = max_depth + if need_aux { 2 } else { 0 };
        while self.pool.len() < pool_n {
            self.pool.push(self.make_target(tw, th));
        }
        if let Some(f) = surface.map(|(_, f)| f) {
            self.blit_pipeline(f);
        }

        self.queue.write_buffer(
            &self.globals,
            0,
            bytemuck::cast_slice(&[tw as f32, th as f32, 0.0, 0.0]),
        );
        if self.atlas.dirty {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.atlas.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ATLAS),
                    rows_per_image: Some(ATLAS),
                },
                wgpu::Extent3d {
                    width: ATLAS,
                    height: ATLAS,
                    depth_or_array_layers: 1,
                },
            );
            self.atlas.dirty = false;
        }

        let mkbuf = |data: &[u8], label: &str| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: data,
                    usage: wgpu::BufferUsages::VERTEX,
                })
        };
        let rect_buf =
            (!fb.rects.is_empty()).then(|| mkbuf(bytemuck::cast_slice(&fb.rects), "rects"));
        let glyph_buf =
            (!fb.glyphs.is_empty()).then(|| mkbuf(bytemuck::cast_slice(&fb.glyphs), "glyphs"));
        let mesh_buf =
            (!fb.meshes.is_empty()).then(|| mkbuf(bytemuck::cast_slice(&fb.meshes), "meshinst"));
        let tex_buf = (!fb.texq.is_empty()).then(|| mkbuf(bytemuck::cast_slice(&fb.texq), "texq"));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        // Metal queues cap in-flight command buffers (default 64) and wgpu
        // records every render pass into its own Metal command buffer, all
        // committed only at submit. A frame with enough layers would block
        // forever acquiring buffer #65, so the frame is split across
        // multiple submissions well below that limit.
        const MAX_PASSES_PER_SUBMIT: usize = 16;
        let mut encoder_passes = 0usize;

        // layer stack: indices into pool; usize::MAX = main target
        let main_t = &self.main.as_ref().unwrap().2;
        let mut stack: Vec<usize> = vec![usize::MAX];
        let mut cleared: Vec<bool> = vec![false];
        let mut next_layer = 0usize;
        let aux_a = if need_aux { pool_n - 2 } else { 0 };
        let aux_b = if need_aux { pool_n - 1 } else { 0 };
        let target_of = |ix: usize| -> &Target {
            if ix == usize::MAX {
                main_t
            } else {
                &self.pool[ix]
            }
        };

        let mut pending: Vec<&Step> = Vec::new();
        macro_rules! flush {
            ($stack:expr, $cleared:expr) => {
                if !pending.is_empty() || !$cleared[$stack.len() - 1] {
                    let top = *$stack.last().unwrap();
                    let load = if $cleared[$stack.len() - 1] {
                        wgpu::LoadOp::Load
                    } else if top == usize::MAX {
                        wgpu::LoadOp::Clear(clear)
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    };
                    $cleared[$stack.len() - 1] = true;
                    encoder_passes += 1;
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("draws"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target_of(top).view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    pass.set_bind_group(0, &self.globals_bg, &[]);
                    for st in pending.drain(..) {
                        match st {
                            Step::Rects {
                                scissor,
                                start,
                                end,
                            } => {
                                let Some(buf) = &rect_buf else { continue };
                                let (x, y, w, h) = clamp_sc(*scissor, tw, th);
                                if w == 0 || h == 0 {
                                    continue;
                                }
                                pass.set_scissor_rect(x, y, w, h);
                                pass.set_pipeline(&self.rect_pl);
                                pass.set_vertex_buffer(0, buf.slice(..));
                                pass.draw(0..4, *start..*end);
                            }
                            Step::Glyphs {
                                scissor,
                                start,
                                end,
                            } => {
                                let Some(buf) = &glyph_buf else { continue };
                                let (x, y, w, h) = clamp_sc(*scissor, tw, th);
                                if w == 0 || h == 0 {
                                    continue;
                                }
                                pass.set_scissor_rect(x, y, w, h);
                                pass.set_pipeline(&self.glyph_pl);
                                pass.set_bind_group(1, &self.atlas_bg, &[]);
                                pass.set_vertex_buffer(0, buf.slice(..));
                                pass.draw(0..4, *start..*end);
                            }
                            Step::Mesh {
                                scissor,
                                doc,
                                key,
                                inst,
                            } => {
                                let Some(ibuf) = &mesh_buf else { continue };
                                let (x, y, w, h) = clamp_sc(*scissor, tw, th);
                                if w == 0 || h == 0 {
                                    continue;
                                }
                                let mesh = match key {
                                    MeshKey::Fill(p) => {
                                        self.docs[*doc].fills.get(p).and_then(|m| m.as_ref())
                                    }
                                    MeshKey::Squircle(k) => {
                                        self.docs[*doc].sq_fills.get(k).and_then(|m| m.as_ref())
                                    }
                                    MeshKey::Stroke(key) => {
                                        self.docs[*doc].strokes.get(key).and_then(|m| m.as_ref())
                                    }
                                };
                                let Some(m) = mesh else { continue };
                                pass.set_scissor_rect(x, y, w, h);
                                pass.set_pipeline(&self.mesh_pl);
                                pass.set_vertex_buffer(0, m.vbuf.slice(..));
                                pass.set_vertex_buffer(1, ibuf.slice(..));
                                pass.set_index_buffer(m.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                                pass.draw_indexed(0..m.index_count, 0, *inst..*inst + 1);
                            }
                            Step::Image {
                                scissor,
                                doc,
                                img,
                                inst,
                            } => {
                                let Some(buf) = &tex_buf else { continue };
                                let Some(Some(it)) = self.docs[*doc].images.get(*img) else {
                                    continue;
                                };
                                let (x, y, w, h) = clamp_sc(*scissor, tw, th);
                                if w == 0 || h == 0 {
                                    continue;
                                }
                                pass.set_scissor_rect(x, y, w, h);
                                pass.set_pipeline(&self.tex_pl);
                                pass.set_bind_group(1, &it.bind, &[]);
                                pass.set_vertex_buffer(0, buf.slice(..));
                                pass.draw(0..4, *inst..*inst + 1);
                            }
                            _ => {}
                        }
                    }
                }
            };
        }

        for st in &fb.steps {
            if encoder_passes >= MAX_PASSES_PER_SUBMIT {
                encoder_passes = 0;
                let finished = std::mem::replace(
                    &mut encoder,
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("frame"),
                        }),
                )
                .finish();
                self.queue.submit(Some(finished));
            }
            match st {
                Step::Rects { .. }
                | Step::Glyphs { .. }
                | Step::Mesh { .. }
                | Step::Image { .. } => {
                    pending.push(st);
                }
                Step::PushLayer => {
                    flush!(stack, cleared);
                    stack.push(next_layer);
                    cleared.push(false);
                    next_layer += 1;
                }
                Step::PopLayer {
                    opacity,
                    sigma,
                    mask,
                } => {
                    flush!(stack, cleared);
                    if stack.len() <= 1 {
                        continue;
                    }
                    let popped = stack.pop().unwrap();
                    cleared.pop();
                    next_layer = popped; // reuse the slot for siblings
                    // blur popped in place via aux ping-pong
                    let src_ix = if *sigma > 0.0 && need_aux {
                        encoder_passes += 2;
                        self.blur_pass(
                            &mut encoder,
                            target_of(popped),
                            &self.pool[aux_a],
                            [0.0, 0.0, tw as f32, th as f32],
                            *sigma,
                            true,
                            tw,
                            th,
                        );
                        self.blur_pass(
                            &mut encoder,
                            &self.pool[aux_a],
                            &self.pool[aux_b],
                            [0.0, 0.0, tw as f32, th as f32],
                            *sigma,
                            false,
                            tw,
                            th,
                        );
                        aux_b
                    } else {
                        popped
                    };
                    if let Some(mask) = mask {
                        // multiply the layer by the mask paint's alpha over
                        // its box (contract 6.3): dst *= src.a via mask blend
                        let mbuf = mkbuf(bytemuck::cast_slice(&[*mask]), "mask");
                        encoder_passes += 1;
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("mask"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &target_of(src_ix).view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Load,
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                        });
                        pass.set_bind_group(0, &self.globals_bg, &[]);
                        pass.set_pipeline(&self.mask_pl);
                        pass.set_vertex_buffer(0, mbuf.slice(..));
                        pass.draw(0..4, 0..1);
                    }
                    // composite onto (new) top with opacity
                    let comp = TexI {
                        mabcd: [1.0, 0.0, 0.0, 1.0],
                        mtc: [0.0, 0.0, tw as f32 / 2.0, th as f32 / 2.0],
                        hro: [tw as f32 / 2.0, th as f32 / 2.0, 0.0, *opacity],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        clip: [-1.0e9, -1.0e9, 1.0e9, 1.0e9],
                        misc: [0.0, 1.0, 0.0, 1.0],
                    };
                    let cbuf = mkbuf(bytemuck::cast_slice(&[comp]), "composite");
                    let top = *stack.last().unwrap();
                    let load = if cleared[stack.len() - 1] {
                        wgpu::LoadOp::Load
                    } else if top == usize::MAX {
                        wgpu::LoadOp::Clear(clear)
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    };
                    cleared[stack.len() - 1] = true;
                    encoder_passes += 1;
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("composite"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target_of(top).view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    pass.set_bind_group(0, &self.globals_bg, &[]);
                    pass.set_pipeline(&self.tex_pl);
                    pass.set_bind_group(1, &target_of(src_ix).bind, &[]);
                    pass.set_vertex_buffer(0, cbuf.slice(..));
                    pass.draw(0..4, 0..1);
                }
                Step::PopTilt { inst } => {
                    flush!(stack, cleared);
                    if stack.len() <= 1 {
                        continue;
                    }
                    let popped = stack.pop().unwrap();
                    cleared.pop();
                    next_layer = popped; // reuse the slot for siblings
                    // composite the layer as a projected quad (contract 6.5)
                    let tbuf = mkbuf(bytemuck::cast_slice(&[*inst]), "tilt");
                    let top = *stack.last().unwrap();
                    let load = if cleared[stack.len() - 1] {
                        wgpu::LoadOp::Load
                    } else if top == usize::MAX {
                        wgpu::LoadOp::Clear(clear)
                    } else {
                        wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                    };
                    cleared[stack.len() - 1] = true;
                    encoder_passes += 1;
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("tilt"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target_of(top).view,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    pass.set_bind_group(0, &self.globals_bg, &[]);
                    pass.set_pipeline(&self.tilt_pl);
                    pass.set_bind_group(1, &target_of(popped).bind, &[]);
                    pass.set_vertex_buffer(0, tbuf.slice(..));
                    pass.draw(0..4, 0..1);
                }
                Step::Backdrop {
                    rect,
                    radius,
                    sigma,
                    saturate,
                    brightness,
                    mask,
                    scissor,
                } => {
                    flush!(stack, cleared);
                    if !need_aux {
                        continue;
                    }
                    let top = *stack.last().unwrap();
                    if !cleared[stack.len() - 1] {
                        // nothing rendered yet — backdrop over the clear color
                        // still needs the pass to exist; force a clear pass
                        flush!(stack, cleared);
                    }
                    let (x, y, w, h) = clamp_sc(*scissor, tw, th);
                    if w == 0 || h == 0 {
                        continue;
                    }
                    // banded progressive blur (contract 6.6) when masked:
                    // band i keeps mask alpha in [i/N, (i+1)/N) and applies
                    // blur*alpha_i with saturate/brightness lerped to identity
                    let bands = if mask.is_some() { 6u32 } else { 1 };
                    for band in 0..bands {
                        let (bsigma, bsat, bbright, blo, bhi) = if mask.is_some() {
                            let alpha = (band as f32 + 0.5) / bands as f32;
                            (
                                sigma * alpha,
                                1.0 + (saturate - 1.0) * alpha,
                                1.0 + (brightness - 1.0) * alpha,
                                band as f32 / bands as f32,
                                if band + 1 == bands {
                                    1.01 // the top band includes alpha == 1
                                } else {
                                    (band as f32 + 1.0) / bands as f32
                                },
                            )
                        } else {
                            (*sigma, *saturate, *brightness, 0.0, 0.0)
                        };
                        let t = target_of(top);
                        // copy the whole target (regions with offsets are
                        // fiddly with alignment; targets are small)
                        encoder.copy_texture_to_texture(
                            t.tex.as_image_copy(),
                            self.pool[aux_a].tex.as_image_copy(),
                            wgpu::Extent3d {
                                width: tw,
                                height: th,
                                depth_or_array_layers: 1,
                            },
                        );
                        let pad = (3.0 * bsigma).ceil();
                        let region = [
                            (rect[0] - pad).max(0.0),
                            (rect[1] - pad).max(0.0),
                            (rect[2] + pad).min(tw as f32),
                            (rect[3] + pad).min(th as f32),
                        ];
                        encoder_passes += 2;
                        self.blur_pass(
                            &mut encoder,
                            &self.pool[aux_a],
                            &self.pool[aux_b],
                            region,
                            bsigma,
                            true,
                            tw,
                            th,
                        );
                        self.blur_pass(
                            &mut encoder,
                            &self.pool[aux_b],
                            &self.pool[aux_a],
                            region,
                            bsigma,
                            false,
                            tw,
                            th,
                        );
                        // paint the blurred region back with rounded mask +
                        // saturate/brightness (band-windowed when masked)
                        let cx = (rect[0] + rect[2]) / 2.0;
                        let cy = (rect[1] + rect[3]) / 2.0;
                        let hx = (rect[2] - rect[0]) / 2.0;
                        let hy = (rect[3] - rect[1]) / 2.0;
                        let uv = [
                            rect[0] / tw as f32,
                            rect[1] / th as f32,
                            (rect[2] - rect[0]) / tw as f32,
                            (rect[3] - rect[1]) / th as f32,
                        ];
                        let huge = [-1.0e9, -1.0e9, 1.0e9, 1.0e9];
                        let (pipeline, cbuf) = if let Some(mgrad) = mask {
                            let comp = TexBandI {
                                mabcd: [1.0, 0.0, 0.0, 1.0],
                                mtc: [0.0, 0.0, cx, cy],
                                hro: [hx, hy, *radius, 1.0],
                                uv,
                                clip: huge,
                                misc: [0.0, bsat, bbright, 0.0],
                                mgrad: *mgrad,
                                band: [blo, bhi, 0.0, 0.0],
                            };
                            (
                                &self.texband_pl,
                                mkbuf(bytemuck::cast_slice(&[comp]), "backdrop-band"),
                            )
                        } else {
                            let comp = TexI {
                                mabcd: [1.0, 0.0, 0.0, 1.0],
                                mtc: [0.0, 0.0, cx, cy],
                                hro: [hx, hy, *radius, 1.0],
                                uv,
                                clip: huge,
                                misc: [0.0, bsat, 0.0, bbright],
                            };
                            (
                                &self.tex_pl,
                                mkbuf(bytemuck::cast_slice(&[comp]), "backdrop"),
                            )
                        };
                        encoder_passes += 1;
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("backdrop"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &t.view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Load,
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                        });
                        pass.set_bind_group(0, &self.globals_bg, &[]);
                        pass.set_scissor_rect(x, y, w, h);
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(1, &self.pool[aux_a].bind, &[]);
                        pass.set_vertex_buffer(0, cbuf.slice(..));
                        pass.draw(0..4, 0..1);
                    }
                }
            }
        }
        flush!(stack, cleared);

        // final blit to the surface
        if let Some((view, format)) = surface {
            let blit = TexI {
                mabcd: [1.0, 0.0, 0.0, 1.0],
                mtc: [0.0, 0.0, tw as f32 / 2.0, th as f32 / 2.0],
                hro: [tw as f32 / 2.0, th as f32 / 2.0, 0.0, 1.0],
                uv: [0.0, 0.0, 1.0, 1.0],
                clip: [-1.0e9, -1.0e9, 1.0e9, 1.0e9],
                misc: [0.0, 1.0, 0.0, 1.0],
            };
            let bbuf = mkbuf(bytemuck::cast_slice(&[blit]), "blit");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &self.globals_bg, &[]);
            pass.set_pipeline(&self.blit_pls[&format]);
            pass.set_bind_group(1, &main_t.bind, &[]);
            pass.set_vertex_buffer(0, bbuf.slice(..));
            pass.draw(0..4, 0..1);
        }

        self.queue.submit(Some(encoder.finish()));
    }

    #[allow(clippy::too_many_arguments)]
    fn blur_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        src: &Target,
        dst: &Target,
        region: [f32; 4],
        sigma: f32,
        horizontal: bool,
        tw: u32,
        th: u32,
    ) {
        // step >1px approximates very large sigmas with bounded taps
        let taps = 16.0f32;
        let step_px = (3.0 * sigma / taps).max(1.0);
        let eff_sigma = sigma / step_px;
        let dir = if horizontal {
            [step_px / tw as f32, 0.0]
        } else {
            [0.0, step_px / th as f32]
        };
        let inst = BlurI {
            rect: region,
            uvr: [
                region[0] / tw as f32,
                region[1] / th as f32,
                region[2] / tw as f32,
                region[3] / th as f32,
            ],
            ds: [
                dir[0],
                dir[1],
                eff_sigma,
                taps.min((3.0 * eff_sigma).ceil().max(1.0)),
            ],
        };
        let buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("blurq"),
                contents: bytemuck::cast_slice(&[inst]),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blur"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &dst.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_bind_group(0, &self.globals_bg, &[]);
        pass.set_pipeline(&self.blur_pl);
        pass.set_bind_group(1, &src.bind, &[]);
        pass.set_vertex_buffer(0, buf.slice(..));
        pass.draw(0..4, 0..1);
    }

    /// Read the internal target back as tightly-packed RGBA8 rows
    /// (premultiplied over the render clear color).
    pub fn read_pixels(&self) -> Option<(u32, u32, Vec<u8>)> {
        let (tw, th, target) = self.main.as_ref()?;
        let (tw, th) = (*tw, *th);
        let bpr = (tw * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (bpr * th) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("read"),
            });
        encoder.copy_texture_to_buffer(
            target.tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bpr),
                    rows_per_image: Some(th),
                },
            },
            wgpu::Extent3d {
                width: tw,
                height: th,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::Wait);
        rx.recv().ok()?.ok()?;
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((tw * th * 4) as usize);
        for row in 0..th {
            let o = (row * bpr) as usize;
            out.extend_from_slice(&data[o..o + (tw * 4) as usize]);
        }
        drop(data);
        buf.unmap();
        Some((tw, th, out))
    }
}

fn clamp_sc(sc: Sc, tw: u32, th: u32) -> Sc {
    let x = sc.0.min(tw);
    let y = sc.1.min(th);
    let w = sc.2.min(tw - x);
    let h = sc.3.min(th - y);
    (x, y, w, h)
}

/// uv rect covering the dest box for OpImage.fit (0 cover | 1 contain |
/// 2 fill), matching the raster exporter's centered scaling. uv outside
/// [0,1] is masked transparent in the shader (contain letterboxing).
fn image_uv(inst: &Instance, im: &slab_kernel::flatten::OpImage) -> [f32; 4] {
    let (iw, ih) = slab_kernel::frame::inst_img_info(inst, im.img)
        .map(|(width, height, _, _)| (f64::from(width), f64::from(height)))
        .unwrap_or((0.0, 0.0));
    if iw <= 0.0 || ih <= 0.0 || im.w <= 0.0 || im.h <= 0.0 {
        return [0.0, 0.0, 1.0, 1.0];
    }
    let (sx, sy) = match im.fit {
        1 => {
            let k = (im.w / iw).min(im.h / ih);
            (k, k)
        }
        2 => (im.w / iw, im.h / ih),
        _ => {
            let k = (im.w / iw).max(im.h / ih);
            (k, k)
        }
    };
    // scaled image is centered: top-left at (im.x + (w - iw*sx)/2, …)
    let tx = (im.w - iw * sx) / 2.0;
    let ty = (im.h - ih * sy) / 2.0;
    let u0 = (0.0 - tx) / (iw * sx);
    let v0 = (0.0 - ty) / (ih * sy);
    let u1 = (im.w - tx) / (iw * sx);
    let v1 = (im.h - ty) / (ih * sy);
    [u0 as f32, v0 as f32, (u1 - u0) as f32, (v1 - v0) as f32]
}

fn premul_blend() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

fn inst_layout(stride: u64, attrs: &[wgpu::VertexAttribute]) -> wgpu::VertexBufferLayout<'_> {
    wgpu::VertexBufferLayout {
        array_stride: stride,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: attrs,
    }
}

#[allow(clippy::too_many_arguments)]
fn make_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    label: &str,
    layout: &wgpu::PipelineLayout,
    vs: &str,
    fs: &str,
    buffers: &[wgpu::VertexBufferLayout<'_>],
    format: wgpu::TextureFormat,
    blend: wgpu::BlendState,
    topology: wgpu::PrimitiveTopology,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vs),
            compilation_options: Default::default(),
            buffers,
        },
        primitive: wgpu::PrimitiveState {
            topology,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    })
}
