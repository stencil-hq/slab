//! Static render orchestration (moved lib-side from the CLI so the wasm
//! build can produce the same SVG/PNG/APNG/TUI outputs with no filesystem).
//!
//! The CLI and wasm front ends parse args / read source, call
//! [`slab_compile::compile`], then hand the compiled SLIR to [`render`].
//! Filesystem IO (reading the `.slab`, writing the output) stays in the
//! front end; this module is pure: kernel decode → env/state/param setup →
//! frame → export.

use slab_kernel::cells;
use slab_kernel::frame as kframe;
use slab_slir::Slir;

use slab_fonts::RegisteredMetrics;

/// A runtime font face supplied by the rendering host.
#[derive(Debug, Clone)]
pub struct RegisteredFont {
    pub name: String,
    pub bytes: Vec<u8>,
    pub metrics: RegisteredMetrics,
}

impl RegisteredFont {
    pub fn new(name: String, bytes: Vec<u8>, metrics: RegisteredMetrics) -> Self {
        Self {
            name,
            bytes,
            metrics,
        }
    }
}

/// Borrowed runtime image payload keyed by its unified kernel image index.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeImage<'a> {
    /// Unified image-table index emitted by [`slab_kernel::flatten::OpImage`].
    pub image: i32,
    /// Declared pixel width.
    pub width: u32,
    /// Declared pixel height.
    pub height: u32,
    /// Payload format: `0` PNG or `1` straight-alpha sRGB RGBA8.
    pub format: u32,
    /// Registry generation, advanced only when the registration changes.
    pub generation: u32,
    /// Encoded PNG or raw RGBA8 payload.
    pub bytes: &'a [u8],
}

/// Returns a PNG payload suitable for static exporters.
pub(crate) fn runtime_image_png<'a>(
    image: &RuntimeImage<'a>,
) -> Option<std::borrow::Cow<'a, [u8]>> {
    if image.format == 0 {
        return (!image.bytes.is_empty()).then_some(std::borrow::Cow::Borrowed(image.bytes));
    }
    if image.format != 1 {
        return None;
    }
    let expected = usize::try_from(image.width)
        .ok()?
        .checked_mul(usize::try_from(image.height).ok()?)?
        .checked_mul(4)?;
    if image.bytes.len() != expected {
        return None;
    }
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .ok()?
            .write_image_data(image.bytes)
            .ok()?;
    }
    Some(std::borrow::Cow::Owned(png))
}
use crate::capsnote;

/// Output kind; mirrors the CLI `--client` / extension inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderKind {
    Svg,
    Png,
    Apng,
    Tui,
}

/// Render parameters (all CLI `render` flags except the file path).
#[derive(Debug, Clone)]
pub struct RenderOpts {
    pub kind: RenderKind,
    pub client: Option<String>,
    pub theme: Option<String>,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    pub t: f64,
    pub dur: f64,
    pub fps: f64,
    pub states: Vec<String>,
    pub env: Vec<String>,
    pub sets: Vec<(String, String)>,
    pub plain: bool,
    /// Runtime font faces to install before solving and use for export paint.
    pub registered_fonts: Vec<RegisteredFont>,
}

/// Render output. `text` is true for the TUI kind (UTF-8 cells in `bytes`);
/// otherwise `bytes` is binary (SVG/PNG/APNG). `notes` carries the one-time
/// capability + tui-grid notes the front end prints to stderr; `summary` is
/// the `({}x{}u …)` dimension annotation.
#[derive(Debug, Clone)]
pub struct RenderOut {
    pub bytes: Vec<u8>,
    pub text: bool,
    pub notes: Vec<String>,
    pub summary: String,
}

/// Map a `--client` name to its kernel client code.
pub fn client_code(name: &str) -> Option<u32> {
    match name {
        "web" => Some(0),
        "gpu" => Some(1),
        "tui" => Some(2),
        "svg" => Some(3),
        "png" => Some(4),
        _ => None,
    }
}

/// Default client code for a render kind (when `--client` is not given).
pub fn default_client(kind: RenderKind) -> u32 {
    match kind {
        RenderKind::Svg => 3,
        RenderKind::Png | RenderKind::Apng => 4,
        RenderKind::Tui => 2,
    }
}

/// Solve a compiled SLIR document through the kernel and export statically.
/// `base_dir` is the image-asset resolution directory for the SVG exporter.
pub fn render(
    slir: &Slir,
    opts: &RenderOpts,
    base_dir: &std::path::Path,
) -> Result<RenderOut, String> {
    let bytes = slab_slir::write(slir);
    let (mut inst, images) =
        slab_slir::instance(&bytes).map_err(|err| format!("kernel decode failed: {err}"))?;
    for font in &opts.registered_fonts {
        let metrics = &font.metrics;
        kframe::inst_font_register(
            &mut inst,
            &font.name,
            u32::from(metrics.weight),
            u32::from(metrics.upem),
            i32::from(metrics.ascent),
            i32::from(metrics.descent),
            i32::from(metrics.line_gap),
            u32::from(metrics.default_advance),
            &metrics.cps,
            &metrics.gids,
            &metrics.advances,
        );
    }
    if let Some(theme) = &opts.theme
        && !kframe::inst_set_theme(&mut inst, theme)
    {
        return Err(format!("unknown theme '{theme}'"));
    }

    let client = match &opts.client {
        Some(c) => client_code(c).ok_or_else(|| format!("unknown client '{c}'"))?,
        None => default_client(opts.kind),
    };
    let dark = opts.env.iter().any(|e| e == "dark");
    let coarse = opts.env.iter().any(|e| e == "coarse");
    let mut height = opts.height;
    if height <= 0.0 && opts.env.iter().any(|e| e == "portrait") {
        height = opts.width * 16.0 / 9.0;
    }
    kframe::inst_set_env(&mut inst, opts.width, height, client, dark, coarse);
    for st in &opts.states {
        if !st.is_empty() {
            kframe::inst_set_state(&mut inst, st, true);
        }
    }
    crate::input::apply_sets(&mut inst, &opts.sets)?;

    let fr = if opts.kind == RenderKind::Svg {
        kframe::inst_frame_static(&mut inst)
    } else {
        kframe::inst_frame(&mut inst, opts.t)
    };
    let mut notes = Vec::new();

    match opts.kind {
        RenderKind::Svg => {
            let dims = format!(" ({}x{}u)", fr.width, fr.height);
            notes.extend(capsnote::render_notes(&inst.doc, &fr, client, &[]));
            let svg =
                crate::svg::render_svg(slir, &images, &[], &opts.registered_fonts, &fr, base_dir);
            Ok(RenderOut {
                bytes: svg.into_bytes(),
                text: false,
                notes,
                summary: dims,
            })
        }
        RenderKind::Png => {
            let dims = format!(" ({}x{}u)", fr.width, fr.height);
            notes.extend(capsnote::render_notes(&inst.doc, &fr, client, &[]));
            let png = crate::raster::render_png(
                slir,
                &images,
                &[],
                &opts.registered_fonts,
                &fr,
                opts.scale,
            )?;
            Ok(RenderOut {
                bytes: png,
                text: false,
                notes,
                summary: dims,
            })
        }
        RenderKind::Apng => {
            let n = ((opts.dur * opts.fps).round() as usize).max(1);
            let mut frames = Vec::with_capacity(n);
            let mut raster =
                crate::raster::Raster::new(slir, &images, &[], &opts.registered_fonts, opts.scale);
            for i in 0..n {
                let t = i as f64 * 1000.0 / opts.fps;
                let f = kframe::inst_frame(&mut inst, t);
                frames.push(raster.render(&f)?);
            }
            let apng = crate::raster::encode_apng(&frames, opts.fps, 0)?;
            let dims = format!(" ({}x{}u)", fr.width, fr.height);
            notes.extend(capsnote::render_notes(&inst.doc, &fr, client, &[]));
            Ok(RenderOut {
                bytes: apng,
                text: false,
                notes,
                summary: format!("{dims} {n} frames @ {}fps", opts.fps),
            })
        }
        RenderKind::Tui => {
            let grid = cells::cells_from_frame(&inst.doc, &fr, fr.width, fr.height);
            for k in 0..grid.diag_code.len() {
                notes.push(format!("note {}: {}", grid.diag_code[k], grid.diag_msg[k]));
            }
            notes.extend(capsnote::render_notes(
                &inst.doc,
                &fr,
                client,
                &grid.diag_code,
            ));
            let text = cells::cells_to_text(&grid, opts.plain);
            let summary = format!(
                " ({}x{}u = {}x{} cells)",
                fr.width, fr.height, grid.cols, grid.rows
            );
            Ok(RenderOut {
                bytes: text.into_bytes(),
                text: true,
                notes,
                summary,
            })
        }
    }
}
