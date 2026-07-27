//! `slab-native --demo settings`: the 10-settings document in a winit window
//! (buttons fire signals to stdout, the field edits through the kernel with
//! IME, the `rows` hole mounts a child kernel instance), plus the
//! `--headless-frame` offscreen smoke hook.

use crate::ClickCounter;
use crate::gen_settings;
use crate::holes::{HoleContent, InstanceHole};
use crate::renderer::{LayerInput, Renderer};
use crate::view::a11y;
use slab_kernel::dispatch as kdispatch;
use slab_kernel::dispatch::Event;
use slab_kernel::flatten::Frame;
use slab_kernel::frame::{self as kframe, HoleRect};
use slab_kernel::slir as kslir;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, Ime, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

#[derive(Clone, Debug)]
pub struct Opts {
    pub width: f64,
    pub height: f64,
    pub t: f64,
    pub scale: Option<f64>,
    pub exit_after_ms: Option<u64>,
    pub max_frames: Option<u64>,
    pub headless_out: Option<PathBuf>,
    pub dark: bool,
    pub theme: Option<String>,
    /// Borderless window (no title bar / OS chrome).
    pub undecorated: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            width: 900.0,
            height: 640.0,
            t: 0.0,
            scale: None,
            exit_after_ms: None,
            max_frames: None,
            headless_out: None,
            dark: false,
            theme: None,
            undecorated: false,
        }
    }
}

/// The demo's hole content: 20 list rows in a scrollable child document,
/// compiled from an inline .slab at startup (host territory — drivers never
/// parse .slab; this is the app half of the demo).
fn rows_slir() -> Result<Vec<u8>, String> {
    let tones = ["#4FC7E0", "#7BE0A3", "#E0B24F", "#E07B7B", "#B48AE0"];
    let labels = [
        "Alpha exposure",
        "Beta channel",
        "Cache lifetime",
        "Delta sync",
        "Edge inset",
        "Focus follows",
        "Grid density",
        "Halo radius",
        "Ink contrast",
        "Jitter guard",
        "Kernel tables",
        "Layout debug",
        "Motion scale",
        "Night shift",
        "Ortho snap",
        "Paint flashing",
        "Quiet hours",
        "Raster hints",
        "Solver trace",
        "Text hinting",
    ];
    let mut src = String::from("col w=fill h=fill scroll clip bg=#171C26 pad=4 gap=2 {\n");
    for (i, label) in labels.iter().enumerate() {
        let tone = tones[i % tones.len()];
        src.push_str(&format!(
            "  row pad=6,12 gap=8 align=center h=28 radius=6 {{\n    \
             rect w=8 h=8 radius=4 bg={tone}\n    \
             text \"{label}\" size=12 color=#E8EEF6 nowrap\n  }}\n"
        ));
    }
    src.push('}');
    let opts = slab_compile::Options {
        embed_assets: true,
        base_dir: PathBuf::from("."),
        ..slab_compile::Options::default()
    };
    let (slir, diags) = slab_compile::compile(&src, &opts);
    let slir = slir.ok_or_else(|| format!("rows doc failed to compile: {:?}", diags.0))?;
    Ok(slab_slir::write(&slir))
}

fn settings_doc(theme: Option<&str>) -> Result<gen_settings::Doc, String> {
    let mut doc = gen_settings::Doc::new();
    if !doc.ok() {
        return Err(format!(
            "embedded SLIR failed to decode: {:?}",
            doc.inst.doc.errs
        ));
    }
    if let Some(name) = theme
        && !doc.set_theme(name)
    {
        return Err(format!("unknown theme '{name}'"));
    }
    Ok(doc)
}

struct HoleBind {
    content: InstanceHole,
    doc_id: usize,
}

// ------------------------------------------------------------- headless ----

/// Render one frame offscreen and write a PNG; asserts two probe pixels
/// (root bg corner, panel rect) so the artifact is self-checking.
pub fn headless_frame(opts: &Opts) -> Result<(), String> {
    let out = opts.headless_out.clone().ok_or("missing output path")?;
    let instance = wgpu::Instance::default();
    let (adapter, device, queue) =
        crate::request_device(&instance, None).ok_or("no wgpu adapter available (headless)")?;
    eprintln!(
        "slab-native: adapter {} ({:?})",
        adapter.get_info().name,
        adapter.get_info().backend
    );
    let mut renderer = Renderer::new(device, queue);

    let mut doc = settings_doc(opts.theme.as_deref())?;
    doc.set_env(opts.width, opts.height, opts.dark, false);
    let main_id = renderer.register_doc(&doc.inst.doc, &doc.imgs, &[]);

    let rows = rows_slir()?;
    let mut hole = InstanceHole::new(&rows).ok_or("rows SLIR failed to decode")?;
    let rows_id = renderer.register_doc(&hole.inst.doc, &hole.imgs, &[]);
    let natural = hole.natural();
    kframe::inst_set_hole_size(&mut doc.inst, 0, natural.0, natural.1);

    let fr = doc.frame(opts.t);
    let _ = kframe::inst_take_signals(&mut doc.inst);
    let hole_rects = doc.holes();
    let mut child_frames: Vec<Frame> = Vec::new();
    for hr in &hole_rects {
        hole.resize(hr.w, hr.h, opts.dark, false);
        child_frames.push(hole.frame(opts.t));
        let _ = kframe::inst_take_signals(&mut hole.inst);
    }

    let scale = opts.scale.unwrap_or(1.0);
    let tw = (opts.width * scale).ceil() as u32;
    let th = (opts.height * scale).ceil() as u32;
    let mut layers = vec![LayerInput {
        doc_id: main_id,
        inst: &doc.inst,
        frame: &fr,
        ox: 0.0,
        oy: 0.0,
        clip: None,
    }];
    for (hr, cf) in hole_rects.iter().zip(&child_frames) {
        layers.push(LayerInput {
            doc_id: rows_id,
            inst: hole.instance(),
            frame: cf,
            ox: hr.x,
            oy: hr.y,
            clip: Some((hr.x, hr.y, hr.w, hr.h, 0.0)),
        });
    }
    let build = renderer.build(&layers, scale, tw, th);
    renderer.render(&build, None, wgpu::Color::BLACK);
    let (w, h, px) = renderer.read_pixels().ok_or("readback failed")?;

    // self-check probes (straight from the doc's tokens)
    let probe = |x: u32, y: u32| -> [u8; 3] {
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    };
    let expect_bg: [u8; 3] = if opts.dark {
        [0x05, 0x07, 0x0B]
    } else {
        [0x10, 0x14, 0x1B]
    };
    let close = |got: [u8; 3], want: [u8; 3]| got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 3);
    let corner = probe(2, 2);
    if !close(corner, expect_bg) {
        return Err(format!("corner probe {corner:?} != bg {expect_bg:?}"));
    }
    // panel: probe just inside the #panel rect (find its Rect op)
    let panel_word = 0xFF26_1C17u32; // #171C26 (r-low packing)
    let mut panel_ok = false;
    for op in &fr.ops {
        if let slab_kernel::flatten::FrameOp::Rect(r) = op
            && r.bg_kind == 1
            && r.bg == panel_word
        {
            let x = ((r.x + 8.0) * scale) as u32;
            let y = ((r.y + 8.0) * scale) as u32;
            let got = probe(x.min(w - 1), y.min(h - 1));
            if !close(got, [0x17, 0x1C, 0x26]) {
                return Err(format!("panel probe {got:?} != #171C26"));
            }
            panel_ok = true;
            break;
        }
    }
    if !panel_ok {
        return Err("no panel rect (#171C26) found in frame ops".into());
    }

    write_png(&out, w, h, &px)?;
    eprintln!(
        "slab-native: headless-frame OK ({w}x{h}px, corner {corner:?}, panel #171C26) -> {}",
        out.display()
    );
    Ok(())
}

pub fn write_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    Ok(())
}

// ------------------------------------------------------------- windowed ----

pub fn run(opts: Opts) -> Result<(), String> {
    if opts.headless_out.is_some() {
        return headless_frame(&opts);
    }
    let event_loop = EventLoop::<a11y::Event>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(opts, event_loop.create_proxy())?;
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    eprintln!("slab-native: presented {} frames", app.frames);
    if app.frames == 0 {
        return Err("no frames presented".into());
    }
    Ok(())
}

/// Which instance holds the pointer capture.
#[derive(Clone, Copy, PartialEq)]
enum Route {
    Main,
    Hole(usize),
}

struct App {
    opts: Opts,
    window: Option<Arc<Window>>,
    a11y_proxy: EventLoopProxy<a11y::Event>,
    accessibility: Option<a11y::WindowAccessibility>,
    surface: Option<wgpu::Surface<'static>>,
    surface_format: wgpu::TextureFormat,
    renderer: Option<Renderer>,
    doc: gen_settings::Doc,
    main_id: usize,
    holes: Vec<HoleBind>,
    rows_bytes: Vec<u8>,
    hole_rects: Vec<HoleRect>,
    mods: u32,
    cursor: (f64, f64),
    cursor_sample: Option<(f64, f64)>,
    clicks: ClickCounter,
    capture: Option<Route>,
    composing: bool,
    start: Instant,
    frames: u64,
    exit_deadline: Option<Instant>,
}

impl App {
    fn new(opts: Opts, a11y_proxy: EventLoopProxy<a11y::Event>) -> Result<App, String> {
        let doc = settings_doc(opts.theme.as_deref())?;
        let rows_bytes = rows_slir()?;
        Ok(App {
            exit_deadline: opts
                .exit_after_ms
                .map(|ms| Instant::now() + std::time::Duration::from_millis(ms)),
            opts,
            window: None,
            a11y_proxy,
            accessibility: None,
            surface: None,
            surface_format: wgpu::TextureFormat::Bgra8Unorm,
            renderer: None,
            doc,
            main_id: 0,
            holes: Vec::new(),
            rows_bytes,
            hole_rects: Vec::new(),
            mods: 0,
            cursor: (0.0, 0.0),
            cursor_sample: None,
            clicks: ClickCounter::default(),
            capture: None,
            composing: false,
            start: Instant::now(),
            frames: 0,
        })
    }

    fn t_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    fn scale(&self) -> f64 {
        self.window.as_ref().map_or(1.0, |w| w.scale_factor())
    }

    fn configure_surface(&mut self) {
        let (Some(window), Some(surface), Some(renderer)) =
            (&self.window, &self.surface, &self.renderer)
        else {
            return;
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        surface.configure(
            &renderer.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format: self.surface_format,
                color_space: wgpu::SurfaceColorSpace::Auto,
                width: size.width,
                height: size.height,
                present_mode: wgpu::PresentMode::Fifo,
                alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                view_formats: vec![],
                desired_maximum_frame_latency: 2,
            },
        );
        let s = window.scale_factor();
        self.doc.set_env(
            size.width as f64 / s,
            size.height as f64 / s,
            self.opts.dark,
            false,
        );
    }

    fn refresh_accessibility(
        &mut self,
        frame: &Frame,
        child_frames: &[Frame],
        size: winit::dpi::PhysicalSize<u32>,
    ) {
        let scale = self.scale();
        let mut layers = Vec::with_capacity(child_frames.len() + 1);
        layers.push(a11y::SceneLayer::new(self.main_id, &self.doc.inst, frame));
        for ((hole, rect), child_frame) in self.holes.iter().zip(&self.hole_rects).zip(child_frames)
        {
            let mut layer = a11y::SceneLayer::new(hole.doc_id, &hole.content.inst, child_frame)
                .translated(rect.x, rect.y);
            if let Some(node) = usize::try_from(rect.hole)
                .ok()
                .and_then(|hole_index| self.doc.inst.doc.hole_node.get(hole_index))
                .copied()
            {
                layer = layer.mounted(self.main_id, node);
            }
            layers.push(layer);
        }
        if let Some(accessibility) = &mut self.accessibility {
            accessibility.refresh(
                "slab — settings",
                f64::from(size.width) / scale,
                f64::from(size.height) / scale,
                scale,
                &layers,
            );
            accessibility.update(false);
        }
    }

    fn draw(&mut self) {
        let t = self.t_ms();
        let Some(window) = self.window.clone() else {
            return;
        };
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        for (i, h) in self.holes.iter_mut().enumerate() {
            if h.content.inst.dirty {
                let natural = h.content.natural();
                kframe::inst_set_hole_size(&mut self.doc.inst, i as u32, natural.0, natural.1);
            }
        }
        let fr = self.doc.frame(t);
        let pending = kframe::inst_take_signals(&mut self.doc.inst);
        for signal in self.doc.decode_signals(&pending) {
            println!("signal: {signal:?}");
        }
        self.hole_rects = self.doc.holes();
        let dark = self.opts.dark;
        for (i, hr) in self.hole_rects.iter().enumerate() {
            if let Some(h) = self.holes.get_mut(i) {
                h.content.resize(hr.w, hr.h, dark, false);
            }
        }
        let child_frames: Vec<Frame> = self
            .holes
            .iter_mut()
            .map(|hole| {
                let frame = hole.content.frame(t);
                let pending = kframe::inst_take_signals(&mut hole.content.inst);
                for name in pending.sig_name {
                    println!("signal: {}", kslir::str_at(&hole.content.inst.doc, name));
                }
                frame
            })
            .collect();
        self.refresh_accessibility(&fr, &child_frames, size);

        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let mut layers = vec![LayerInput {
            doc_id: self.main_id,
            inst: &self.doc.inst,
            frame: &fr,
            ox: 0.0,
            oy: 0.0,
            clip: None,
        }];
        for ((h, hr), cf) in self.holes.iter().zip(&self.hole_rects).zip(&child_frames) {
            layers.push(LayerInput {
                doc_id: h.doc_id,
                inst: h.content.instance(),
                frame: cf,
                ox: hr.x,
                oy: hr.y,
                clip: Some((hr.x, hr.y, hr.w, hr.h, 0.0)),
            });
        }
        let scale = window.scale_factor();
        let build = renderer.build(&layers, scale, size.width, size.height);
        let frame_tex = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.configure_surface();
                return;
            }
            error => {
                eprintln!("slab-native: surface error: {error:?}");
                return;
            }
        };
        let view = frame_tex
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        renderer.render(
            &build,
            Some((&view, self.surface_format)),
            wgpu::Color::BLACK,
        );
        window.pre_present_notify();
        renderer.queue.present(frame_tex);
        self.frames += 1;

        let animating = self.doc.inst.dirty
            || self.doc.inst.ms.active
            || self.holes.iter().any(|h| h.content.needs_frame());
        if animating || self.opts.max_frames.is_some() {
            window.request_redraw();
        }
    }

    /// Route a pointer-ish event to the hole under it (translated) or main.
    fn route_of(&self, x: f64, y: f64) -> Route {
        if let Some(r) = self.capture {
            return r;
        }
        for (i, hr) in self.hole_rects.iter().enumerate() {
            if i < self.holes.len() && x >= hr.x && x < hr.x + hr.w && y >= hr.y && y < hr.y + hr.h
            {
                return Route::Hole(i);
            }
        }
        Route::Main
    }

    fn dispatch_routed(&mut self, route: Route, mut ev: Event) {
        match route {
            Route::Main => self.dispatch_main(ev),
            Route::Hole(i) => {
                let hr = &self.hole_rects[i];
                ev.x -= hr.x;
                ev.y -= hr.y;
                let eff = self.holes[i].content.dispatch(&ev);
                for (k, &name_ref) in eff.sig_name.iter().enumerate() {
                    let name = kslir::str_at(&self.holes[i].content.instance().doc, name_ref);
                    let text = &eff.sig_text[k];
                    if text.is_empty() {
                        println!("signal: {name}");
                    } else {
                        println!("signal: {name} {text:?}");
                    }
                }
                if eff.repaint
                    && let Some(w) = &self.window
                {
                    w.request_redraw();
                }
            }
        }
    }

    fn dispatch_main(&mut self, ev: Event) {
        let (eff, sigs) = self.doc.dispatch(&ev);
        for s in &sigs {
            println!("signal: {s:?}");
        }
        let Some(window) = &self.window else { return };
        if eff.repaint {
            window.request_redraw();
        }
        window.set_cursor(crate::cursor_icon(eff.cursor));
        if eff.has_ime {
            window.set_ime_cursor_area(
                LogicalPosition::new(eff.ime_x, eff.ime_y),
                LogicalSize::new(eff.ime_w.max(1.0), eff.ime_h),
            );
        }
    }

    fn base_event(&self, etype: u32) -> Event {
        Event {
            etype,
            x: self.cursor.0,
            y: self.cursor.1,
            dx: 0.0,
            dy: 0.0,
            button: 0,
            clicks: 0,
            key: String::new(),
            text: String::new(),
            mods: self.mods,
        }
    }

    fn accessibility_action(&mut self, request: &accesskit::ActionRequest) {
        let routed = self
            .accessibility
            .as_ref()
            .and_then(|accessibility| accessibility.resolve_action(request));
        let Some(routed) = routed else {
            return;
        };
        let result = if routed.document == self.main_id {
            routed.apply(&mut self.doc.inst)
        } else if let Some(hole) = self
            .holes
            .iter_mut()
            .find(|hole| hole.doc_id == routed.document)
        {
            routed.apply(&mut hole.content.inst)
        } else {
            return;
        };
        if matches!(&result, a11y::ActionResult::Ignored) {
            return;
        }
        if routed.moves_focus() {
            if routed.document == self.main_id {
                for hole in &mut self.holes {
                    kframe::inst_set_focus(&mut hole.content.inst, "", false);
                }
            } else {
                kframe::inst_set_focus(&mut self.doc.inst, "", false);
                for hole in &mut self.holes {
                    if hole.doc_id != routed.document {
                        kframe::inst_set_focus(&mut hole.content.inst, "", false);
                    }
                }
            }
        }
        match result {
            a11y::ActionResult::Ignored => unreachable!(),
            a11y::ActionResult::Changed => {}
            a11y::ActionResult::Dispatch(event) => {
                if routed.document == self.main_id {
                    self.dispatch_main(event);
                } else if let Some(hole) = self
                    .holes
                    .iter_mut()
                    .find(|hole| hole.doc_id == routed.document)
                {
                    let _ = hole.content.dispatch(&event);
                }
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler<a11y::Event> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("slab — settings")
                .with_inner_size(LogicalSize::new(self.opts.width, self.opts.height))
                .with_decorations(!self.opts.undecorated)
                .with_visible(false),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("slab-native: window creation failed: {e}");
                event_loop.exit();
                return;
            }
        };
        let accessibility =
            a11y::WindowAccessibility::new(event_loop, &window, self.a11y_proxy.clone());
        window.set_ime_allowed(true);
        let instance = wgpu::Instance::default();
        let surface = match instance.create_surface(window.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("slab-native: surface creation failed: {e}");
                event_loop.exit();
                return;
            }
        };
        let Some((adapter, device, queue)) = crate::request_device(&instance, Some(&surface))
        else {
            eprintln!("slab-native: no wgpu adapter");
            event_loop.exit();
            return;
        };
        let caps = surface.get_capabilities(&adapter);
        self.surface_format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let mut renderer = Renderer::new(device, queue);
        self.main_id = renderer.register_doc(&self.doc.inst.doc, &self.doc.imgs, &[]);
        if let Some(mut hole) = InstanceHole::new(&self.rows_bytes) {
            let doc_id = renderer.register_doc(&hole.inst.doc, &hole.imgs, &[]);
            let natural = hole.natural();
            kframe::inst_set_hole_size(&mut self.doc.inst, 0, natural.0, natural.1);
            self.holes.push(HoleBind {
                content: hole,
                doc_id,
            });
        } else {
            eprintln!("slab-native: rows SLIR failed to decode; hole left empty");
        }
        self.renderer = Some(renderer);
        self.surface = Some(surface);
        self.window = Some(window.clone());
        self.accessibility = Some(accessibility);
        self.configure_surface();
        window.set_visible(true);
        window.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let (Some(accessibility), Some(window)) = (&mut self.accessibility, &self.window) {
            accessibility.process_event(window, &event);
        }
        match event {
            WindowEvent::CloseRequested => {
                let route = self.capture.unwrap_or(Route::Main);
                let ev = self.base_event(kdispatch::E_CLOSE);
                self.dispatch_routed(route, ev);
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                self.draw();
                if let Some(max) = self.opts.max_frames
                    && self.frames >= max
                {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(_) => {
                self.configure_surface();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                self.cursor_sample = None;
                self.configure_surface();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                let st = m.state();
                self.mods = 0;
                if st.shift_key() {
                    self.mods |= kdispatch::M_SHIFT;
                }
                if st.alt_key() {
                    self.mods |= kdispatch::M_ALT;
                }
                if st.control_key() {
                    self.mods |= kdispatch::M_CTRL;
                }
                if st.super_key() {
                    self.mods |= kdispatch::M_META;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let s = self.scale();
                let cursor = (position.x / s, position.y / s);
                let (dx, dy) = crate::cursor_delta(&mut self.cursor_sample, cursor);
                self.cursor = cursor;
                let mut ev = self.base_event(kdispatch::E_POINTER_MOVE);
                ev.dx = dx;
                ev.dy = dy;
                let route = self.route_of(self.cursor.0, self.cursor.1);
                self.dispatch_routed(route, ev);
            }
            WindowEvent::CursorLeft { .. } => {
                self.cursor_sample = None;
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let btn = crate::mouse_button_id(button);
                let clicks = if state == ElementState::Pressed {
                    self.clicks.pointer_down(btn, self.cursor.0, self.cursor.1)
                } else {
                    0
                };
                let etype = if state == ElementState::Pressed {
                    kdispatch::E_POINTER_DOWN
                } else {
                    kdispatch::E_POINTER_UP
                };
                let mut ev = self.base_event(etype);
                ev.button = btn;
                ev.clicks = clicks;
                let route = self.route_of(self.cursor.0, self.cursor.1);
                if state == ElementState::Pressed {
                    self.capture = Some(route);
                } else {
                    self.capture = None;
                }
                self.dispatch_routed(route, ev);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-x as f64 * 40.0, -y as f64 * 40.0),
                    MouseScrollDelta::PixelDelta(p) => {
                        let s = self.scale();
                        (-p.x / s, -p.y / s)
                    }
                };
                let mut ev = self.base_event(kdispatch::E_WHEEL);
                ev.dx = dx;
                ev.dy = dy;
                let route = self.route_of(self.cursor.0, self.cursor.1);
                self.dispatch_routed(route, ev);
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed || self.composing {
                    return;
                }
                if let Some(name) = crate::key_name(&event.logical_key) {
                    let mut ev = self.base_event(kdispatch::E_KEY_DOWN);
                    ev.key = name;
                    self.dispatch_main(ev);
                }
                let insertable = matches!(event.logical_key, Key::Character(_))
                    || event.logical_key == Key::Named(NamedKey::Space);
                let no_cmd = self.mods & (kdispatch::M_CTRL | kdispatch::M_META) == 0;
                if insertable
                    && no_cmd
                    && let Some(text) = &event.text
                {
                    let mut ev = self.base_event(kdispatch::E_TEXT);
                    ev.text = text.to_string();
                    self.dispatch_main(ev);
                }
            }
            WindowEvent::Ime(ime) => match ime {
                Ime::Enabled => {}
                Ime::Preedit(text, _cursor) => {
                    if !text.is_empty() {
                        if !self.composing {
                            self.composing = true;
                            let ev = self.base_event(kdispatch::E_COMPOSITION_START);
                            self.dispatch_main(ev);
                        }
                        let mut ev = self.base_event(kdispatch::E_COMPOSITION_UPDATE);
                        ev.text = text;
                        self.dispatch_main(ev);
                    }
                }
                Ime::Commit(text) => {
                    if self.composing {
                        self.composing = false;
                        let mut ev = self.base_event(kdispatch::E_COMPOSITION_END);
                        ev.text = text;
                        self.dispatch_main(ev);
                    } else {
                        // direct commit without preedit (e.g. dead keys)
                        let mut ev = self.base_event(kdispatch::E_TEXT);
                        ev.text = text;
                        self.dispatch_main(ev);
                    }
                }
                Ime::Disabled => {
                    if self.composing {
                        self.composing = false;
                        let ev = self.base_event(kdispatch::E_COMPOSITION_END);
                        self.dispatch_main(ev);
                    }
                }
            },
            WindowEvent::Focused(false) => {
                let ev = self.base_event(kdispatch::E_BLUR);
                self.dispatch_main(ev);
            }
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: a11y::Event) {
        if self
            .window
            .as_ref()
            .is_none_or(|window| window.id() != event.window_id)
        {
            return;
        }
        match event.window_event {
            a11y::EventKind::InitialTreeRequested => {
                if let Some(accessibility) = &mut self.accessibility {
                    accessibility.update(true);
                }
            }
            a11y::EventKind::ActionRequested(request) => {
                self.accessibility_action(&request);
            }
            a11y::EventKind::AccessibilityDeactivated => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(deadline) = self.exit_deadline {
            if Instant::now() >= deadline {
                event_loop.exit();
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
            }
        }
    }
}
