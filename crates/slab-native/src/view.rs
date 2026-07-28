//! Generic `slab-native FILE.slab` document viewer.
//!
//! Compiles the source in-process (assets resolve against the file's
//! directory), hands SLIR to the kernel, and drives the same winit/wgpu loop as
//! the demos. Signals print to stdout; holes render empty (hole content is app
//! territory — mount a child instance like `--demo settings` does when you need
//! one).
//!
//! # Window chrome contract (`--undecorated`)
//! Borderless windows flip the document-global state `undecorated` on, so a
//! document renders its own chrome behind `when undecorated { … }` (static
//! renders preview it with `--state undecorated`). The chrome talks back
//! through reserved activation signals the viewer intercepts instead of the
//! host: `act=window-close`, `act=window-minimize`, `act=window-maximize`
//! (toggle), and `act=window-drag` (starts an OS window move on press —
//! bind it on the titlebar container; nested controls still win).

use std::{path::Path, sync::Arc, time::Instant};

use slab_kernel::{
	dispatch as kdispatch,
	dispatch::Event,
	frame::{self as kframe},
	slir::Doc,
};
use winit::{
	application::ApplicationHandler,
	dpi::LogicalSize,
	event::{ElementState, MouseScrollDelta, WindowEvent},
	event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
	keyboard::{Key, NamedKey},
	window::{Window, WindowId},
};

use crate::{
	NativeDocument, a11y,
	demo::{self, write_png},
	input::{self, Clipboard, ImeState},
	renderer::{LayerInput, Renderer},
};

/// User-event envelope used by [`NativeShell`]. Hosts can send application
/// events through the same winit loop AccessKit uses.
pub enum ShellEvent<U> {
	/// Accessibility event delivered by AccessKit through the shared loop.
	Accessibility(a11y::Event),
	/// Application event supplied by the host, such as an SDP wake-up.
	User(U),
	/// Graceful shutdown request (SIGTERM); the shell exits with status 0.
	Shutdown,
}

impl<U> From<a11y::Event> for ShellEvent<U> {
	fn from(event: a11y::Event) -> Self {
		Self::Accessibility(event)
	}
}

/// Window and scheduler policy supplied to [`NativeShell`].
#[derive(Clone)]
pub struct ShellOptions {
	/// Window title.
	pub title:         String,
	/// Initial logical viewport width.
	pub width:         f64,
	/// Initial logical viewport height.
	pub height:        f64,
	/// Initial dark-environment preference.
	pub dark:          bool,
	/// Whether the OS window decorations are disabled.
	pub undecorated:   bool,
	/// Optional rendered-frame limit used by deterministic hosts and demos.
	pub max_frames:    Option<u64>,
	/// Optional wall-clock lifetime after which the shell exits.
	pub exit_after_ms: Option<u64>,
}

impl Default for ShellOptions {
	fn default() -> Self {
		Self {
			title:         "Slab".to_owned(),
			width:         960.0,
			height:        640.0,
			dark:          false,
			undecorated:   false,
			max_frames:    None,
			exit_after_ms: None,
		}
	}
}

/// Application policy plugged into the reusable native window driver.
///
/// Signal callbacks own model synchronization. User events are suitable for
/// draining a `RequestPump`; mutate the document, then return `true` to request
/// a redraw. Input, IME, accessibility, presentation and motion scheduling
/// remain shell-owned.
pub trait ShellHost<U> {
	/// Handles one decoded Slab signal and may synchronize application state.
	///
	/// `text` is empty for non-text signals. The default prints the signal.
	fn signal(&mut self, _document: &mut NativeDocument, name: &str, text: &str) {
		if text.is_empty() {
			println!("signal: {name}");
		} else {
			println!("signal: {name} {text:?}");
		}
	}

	/// Receives the complete kernel effect batch so hosts can retain typed
	/// signal metadata. The default forwards each signal to [`Self::signal`].
	fn effects(&mut self, document: &mut NativeDocument, effects: &kdispatch::Effects) {
		for index in 0..effects.sig_name.len() {
			let name =
				slab_kernel::slir::str_at(document.inst.doc(), effects.sig_name[index]).to_owned();
			let text = effects.sig_text.get(index).cloned().unwrap_or_default();
			self.signal(document, &name, &text);
		}
	}

	/// Handles one application event, returning `true` when redraw is required.
	///
	/// SDP hosts use this hook to drain queued requests against `document`.
	fn user_event(
		&mut self,
		_document: &mut NativeDocument,
		_window: &Window,
		_event_loop: &ActiveEventLoop,
		_event: U,
	) -> bool {
		false
	}
}

/// Default host policy: print signals and ignore application user events.
pub struct DefaultShellHost;

impl<U> ShellHost<U> for DefaultShellHost {}

/// Kernel client code for the GPU driver (`caps::CLIENTS` index).
const CLIENT_GPU: u32 = 1;
/// Window actions a document's own chrome requests through reserved
/// activation signal names (`act=window-close` …). The viewer performs the
/// action; the signal still prints for host visibility.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowCmd {
	Close,
	Minimize,
	/// Toggles maximized.
	Maximize,
	/// Starts an OS window move; only meaningful on pointer press.
	Drag,
}

impl WindowCmd {
	/// Maps a signal name to its window action (None = ordinary app signal).
	pub fn from_signal(name: &str) -> Option<Self> {
		match name {
			"window-close" => Some(Self::Close),
			"window-minimize" => Some(Self::Minimize),
			"window-maximize" => Some(Self::Maximize),
			"window-drag" => Some(Self::Drag),
			_ => None,
		}
	}
}

/// Activation signal name bound to `node` (its `act=` binding), if any.
pub fn act_signal(doc: &Doc, node: u32) -> Option<&str> {
	for s in 0..doc.sign_name.len() {
		if doc.sign_node[s] == node && doc.sign_trigger[s] == 0 {
			return doc.strs.get(doc.sign_name[s] as usize).map(String::as_str);
		}
	}
	None
}

/// Compile `path` and run it windowed (or `--headless-frame` offscreen).
/// Diagnostics print to stderr in CLI format; errors abort before any GPU
/// work.
pub fn run(path: &Path, opts: demo::Opts) -> Result<(), String> {
	let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
	let base = path
		.parent()
		.unwrap_or_else(|| Path::new("."))
		.to_path_buf();
	let name = path
		.file_stem()
		.and_then(|s| s.to_str())
		.unwrap_or("document")
		.to_string();
	run_source(&name, &src, base, opts)
}

/// Compile embedded `.slab` source and run it through the same viewer loop.
/// Backs baked-in demos (`--demo modern`) that carry their document in the
/// binary instead of on disk.
pub fn run_source(
	name: &str,
	src: &str,
	base_dir: std::path::PathBuf,
	opts: demo::Opts,
) -> Result<(), String> {
	let copts = slab_compile::Options { base_dir: base_dir.clone(), ..Default::default() };
	let (slir, diags) = slab_compile::compile(src, &copts);
	for d in &diags.0 {
		eprintln!("{}", d.format(name));
	}
	let Some(slir) = slir else {
		return Err("compile failed".into());
	};
	let bytes = slab_slir::write(&slir);
	let mut doc =
		NativeDocument::decode(&bytes).map_err(|err| format!("SLIR decode failed: {err}"))?;
	if !doc.inst.ok {
		return Err(format!("SLIR decode failed: {:?}", doc.inst.doc().errs));
	}
	if opts.undecorated {
		// documents render their own chrome behind `when undecorated`
		kframe::inst_set_state(&mut doc.inst, "undecorated", true);
	}
	if !doc.inst.doc().hole_name.is_empty() {
		eprintln!("slab-native: note: document declares holes; they render empty here");
	}
	if opts.headless_out.is_some() {
		return headless_frame(&mut doc, &opts);
	}

	let options = ShellOptions {
		title:         format!("slab — {name}"),
		width:         opts.width,
		height:        opts.height,
		dark:          opts.dark,
		undecorated:   opts.undecorated,
		max_frames:    opts.max_frames,
		exit_after_ms: opts.exit_after_ms,
	};
	if let Some(port) = opts.port {
		let doc_path = base_dir.join(format!("{name}.slab"));
		return crate::sdp::run_window(doc, slir, doc_path, options, port);
	}
	let event_loop = EventLoop::<ShellEvent<()>>::with_user_event()
		.build()
		.map_err(|e| e.to_string())?;
	event_loop.set_control_flow(ControlFlow::Wait);
	install_sigterm(event_loop.create_proxy());
	let mut app = NativeShell::new(doc, options, event_loop.create_proxy(), DefaultShellHost);
	event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
	eprintln!("slab-native: presented {} frames", app.frames);
	if app.frames == 0 {
		return Err("no frames presented".into());
	}
	Ok(())
}

/// Routes SIGTERM to a graceful [`ShellEvent::Shutdown`] so supervisors that
/// stop the viewer observe exit status 0 instead of a signal death (N20).
#[cfg(unix)]
pub(crate) fn install_sigterm<U: Send + 'static>(proxy: EventLoopProxy<ShellEvent<U>>) {
	match signal_hook::iterator::Signals::new([signal_hook::consts::SIGTERM]) {
		Ok(mut signals) => {
			std::thread::spawn(move || {
				if signals.forever().next().is_some() {
					let _ = proxy.send_event(ShellEvent::Shutdown);
				}
			});
		},
		Err(e) => eprintln!("slab-native: cannot install SIGTERM handler: {e}"),
	}
}

/// SIGTERM does not exist on this platform; nothing to install.
#[cfg(not(unix))]
pub(crate) fn install_sigterm<U: Send + 'static>(_proxy: EventLoopProxy<ShellEvent<U>>) {}

/// Render one frame offscreen at `--t` and write a PNG (no probe pixels —
/// arbitrary documents carry no known colors to assert).
fn headless_frame(doc: &mut NativeDocument, opts: &demo::Opts) -> Result<(), String> {
	let out = opts.headless_out.clone().ok_or("missing output path")?;
	let instance = wgpu::Instance::default();
	let (adapter, device, queue) =
		crate::request_device(&instance, None).ok_or("no wgpu adapter available (headless)")?;
	eprintln!("slab-native: adapter {} ({:?})", adapter.get_info().name, adapter.get_info().backend);
	let mut renderer = Renderer::new(device, queue);
	kframe::inst_set_env(&mut doc.inst, opts.width, opts.height, CLIENT_GPU, opts.dark, false);
	let doc_id = renderer.register_doc(doc.inst.doc(), &doc.imgs, doc.registered_fonts());
	let fr = kframe::inst_frame(&mut doc.inst, opts.t);
	let pending = kframe::inst_take_signals(&mut doc.inst);
	for name in pending.sig_name {
		println!("signal: {}", slab_kernel::slir::str_at(doc.inst.doc(), name));
	}

	let scale = opts.scale.unwrap_or(1.0);
	let tw = (opts.width * scale).ceil() as u32;
	let th = (opts.height * scale).ceil() as u32;
	let layers = [LayerInput { doc_id, inst: &doc.inst, frame: &fr, ox: 0.0, oy: 0.0, clip: None }];
	let build = renderer.build(&layers, scale, tw, th);
	renderer.render(build, None, wgpu::Color::BLACK);
	let (w, h, px) = renderer.read_pixels().ok_or("readback failed")?;
	write_png(&out, w, h, &px)?;
	eprintln!("slab-native: headless-frame OK ({w}x{h}px) -> {}", out.display());
	Ok(())
}

/// Reusable winit/wgpu host for one Slab document.
///
/// Construct an `EventLoop<ShellEvent<U>>`, pass its proxy to [`Self::new`],
/// and implement [`ShellHost`] for application signal and user-event policy.
/// The shell owns all platform translation and redraw lifecycle.
pub struct NativeShell<U: 'static, H> {
	opts:            ShellOptions,
	window:          Option<Arc<Window>>,
	a11y_proxy:      EventLoopProxy<ShellEvent<U>>,
	accessibility:   Option<a11y::WindowAccessibility>,
	surface:         Option<wgpu::Surface<'static>>,
	surface_format:  wgpu::TextureFormat,
	renderer:        Option<Renderer>,
	doc:             NativeDocument,
	doc_id:          usize,
	host:            H,
	mods:            u32,
	cursor:          (f64, f64),
	cursor_sample:   Option<(f64, f64)>,
	clicks:          input::ClickCounter,
	ime:             ImeState,
	clipboard:       Clipboard,
	context_actions: bool,
	occluded:        bool,
	start:           Instant,
	/// Number of frames successfully presented by this shell.
	pub frames:      u64,
	exit_deadline:   Option<Instant>,
}

/// Reports whether a dispatch requires a redraw: the kernel effect repaints,
/// or host signal handlers dirtied the instance while handling `effects`.
const fn needs_redraw(eff: &kdispatch::Effects, inst: &kframe::Instance) -> bool {
	eff.repaint || inst.dirty
}

impl<U, H> NativeShell<U, H>
where
	U: Send + 'static,
	H: ShellHost<U>,
{
	/// Creates a shell around one document, host policy, and shared event proxy.
	pub fn new(
		doc: NativeDocument,
		opts: ShellOptions,
		a11y_proxy: EventLoopProxy<ShellEvent<U>>,
		host: H,
	) -> Self {
		Self {
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
			doc_id: 0,
			host,
			mods: 0,
			cursor: (0.0, 0.0),
			cursor_sample: None,
			clicks: input::ClickCounter::default(),
			ime: ImeState::default(),
			clipboard: Clipboard::default(),
			context_actions: false,
			occluded: false,
			start: Instant::now(),
			frames: 0,
		}
	}

	/// Returns the mounted document and its live kernel instance.
	pub const fn document(&self) -> &NativeDocument {
		&self.doc
	}

	/// Returns mutable access for host-side model and parameter synchronization.
	pub const fn document_mut(&mut self) -> &mut NativeDocument {
		&mut self.doc
	}

	/// Returns the live window after the application has resumed.
	pub fn window(&self) -> Option<&Window> {
		self.window.as_deref()
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
		surface.configure(&renderer.device, &wgpu::SurfaceConfiguration {
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			format: self.surface_format,
			color_space: wgpu::SurfaceColorSpace::Auto,
			width: size.width,
			height: size.height,
			present_mode: wgpu::PresentMode::Fifo,
			alpha_mode: wgpu::CompositeAlphaMode::Opaque,
			view_formats: vec![],
			desired_maximum_frame_latency: 2,
		});
		let s = window.scale_factor();
		kframe::inst_set_env(
			&mut self.doc.inst,
			size.width as f64 / s,
			size.height as f64 / s,
			CLIENT_GPU,
			self.opts.dark,
			false,
		);
	}

	fn refresh_accessibility(
		&mut self,
		frame: &slab_kernel::flatten::Frame,
		size: winit::dpi::PhysicalSize<u32>,
	) {
		let scale = self.scale();
		let layer = a11y::SceneLayer::new(self.doc_id, &self.doc.inst, frame);
		if let Some(accessibility) = &mut self.accessibility {
			accessibility.refresh(
				&self.opts.title,
				f64::from(size.width) / scale,
				f64::from(size.height) / scale,
				scale,
				&[layer],
			);
			accessibility.update(false);
		}
	}

	fn draw(&mut self) {
		if self.occluded {
			return;
		}
		let t = self.t_ms();
		let Some(window) = self.window.clone() else {
			return;
		};
		let size = window.inner_size();
		if size.width == 0 || size.height == 0 {
			return;
		}
		let fr = kframe::inst_frame(&mut self.doc.inst, t);
		let pending = kframe::inst_take_signals(&mut self.doc.inst);
		self.host.effects(&mut self.doc, &pending);
		self.refresh_accessibility(&fr, size);
		let Some(renderer) = self.renderer.as_mut() else {
			return;
		};
		let Some(surface) = self.surface.as_ref() else {
			return;
		};
		let layers = [LayerInput {
			doc_id: self.doc_id,
			inst:   &self.doc.inst,
			frame:  &fr,
			ox:     0.0,
			oy:     0.0,
			clip:   None,
		}];
		let scale = window.scale_factor();
		let build = renderer.build(&layers, scale, size.width, size.height);
		let frame_tex = match surface.get_current_texture() {
			wgpu::CurrentSurfaceTexture::Success(texture)
			| wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
			// The drawable pool is exhausted; retry on the next display cycle
			// instead of dropping the frame (an animating doc reschedules only
			// after a successful draw).
			wgpu::CurrentSurfaceTexture::Timeout => {
				window.request_redraw();
				return;
			},
			wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
				self.configure_surface();
				window.request_redraw();
				return;
			},
			// Occluded windows repaint via `WindowEvent::Occluded(false)`;
			// retrying here would spin while hidden.
			wgpu::CurrentSurfaceTexture::Occluded => return,
			error => {
				eprintln!("slab-native: surface error: {error:?}");
				return;
			},
		};
		let view = frame_tex
			.texture
			.create_view(&wgpu::TextureViewDescriptor::default());
		renderer.render(build, Some((&view, self.surface_format)), wgpu::Color::BLACK);
		window.pre_present_notify();
		renderer.queue.present(frame_tex);
		self.frames += 1;

		if self.doc.inst.dirty || self.doc.inst.ms.active || self.opts.max_frames.is_some() {
			window.request_redraw();
		}
	}

	fn dispatch(&mut self, event_loop: &ActiveEventLoop, ev: Event) {
		let eff = kframe::inst_dispatch(&mut self.doc.inst, &ev);
		let mut cmd = None;
		for k in 0..eff.sig_name.len() {
			let name = self
				.doc
				.inst
				.doc()
				.strs
				.get(eff.sig_name[k] as usize)
				.map_or("?", String::as_str)
				.to_owned();
			cmd = cmd.or_else(|| WindowCmd::from_signal(&name));
		}
		self.host.effects(&mut self.doc, &eff);
		match cmd {
			Some(WindowCmd::Close) => event_loop.exit(),
			Some(WindowCmd::Minimize) => {
				if let Some(w) = &self.window {
					w.set_minimized(true);
				}
			},
			Some(WindowCmd::Maximize) => {
				if let Some(w) = &self.window {
					w.set_maximized(!w.is_maximized());
				}
			},
			// Drag acts on pointer PRESS (see MouseInput), not on Activate.
			Some(WindowCmd::Drag) | None => {},
		}
		let Some(window) = &self.window else { return };
		if needs_redraw(&eff, &self.doc.inst) {
			window.request_redraw();
		}
		window.set_cursor(input::cursor_icon(eff.cursor));
		self.ime.sync_rect(window, &eff);
		self
			.ime
			.set_allowed(window, input::focus_in_field(&self.doc.inst));
	}

	const fn base_event(&self, etype: u32) -> Event {
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
			clauses: Vec::new(),
			mods: self.mods,
		}
	}

	/// Applies one clipboard action to the focused field.
	fn clipboard_action(&mut self, event_loop: &ActiveEventLoop, action: &str) -> bool {
		if !input::focus_in_field(&self.doc.inst) {
			return false;
		}
		match action {
			"c" | "x" => {
				if let Some(selection) = input::selection_text(&self.doc.inst).filter(|s| !s.is_empty())
				{
					self.clipboard.write(&selection);
					if action == "x" {
						let ev = self.base_event(kdispatch::E_CUT);
						self.dispatch(event_loop, ev);
					}
				}
				true
			},
			"v" => {
				if let Some(text) = self.clipboard.read() {
					let mut ev = self.base_event(kdispatch::E_PASTE);
					ev.text = text;
					self.dispatch(event_loop, ev);
				}
				true
			},
			_ => false,
		}
	}

	/// Handles Cmd/Ctrl C/X/V for the focused field.
	fn clipboard_shortcut(&mut self, event_loop: &ActiveEventLoop, key: &Key) -> bool {
		if self.mods & (kdispatch::M_CTRL | kdispatch::M_META) == 0 {
			return false;
		}
		let Key::Character(action) = key else {
			return false;
		};
		// A host `keys=` binding for the chord wins over the shell recipe:
		// the key reaches kernel dispatch and bubbles to the binding.
		if kframe::inst_key_claimed(&self.doc.inst, action.as_str()) {
			return false;
		}
		self.clipboard_action(event_loop, action.as_str())
	}

	/// Opens or closes the reference host's keyboard-operated context
	/// affordance.
	fn set_context_actions(&mut self, open: bool) {
		self.context_actions = open;
		if let Some(window) = &self.window {
			window.set_title(if open {
				"Text actions — C Copy · X Cut · V Paste · Esc Close"
			} else {
				&self.opts.title
			});
		}
	}

	/// Handles a key while the text context affordance is open.
	fn context_action_key(&mut self, event_loop: &ActiveEventLoop, key: &Key) -> bool {
		if !self.context_actions {
			return false;
		}
		let action = match key {
			Key::Named(NamedKey::Escape) => None,
			Key::Character(action) if matches!(action.as_str(), "c" | "x" | "v") => {
				Some(action.as_str())
			},
			_ => return false,
		};
		if let Some(action) = action {
			self.clipboard_action(event_loop, action);
		}
		self.set_context_actions(false);
		true
	}

	fn accessibility_action(
		&mut self,
		event_loop: &ActiveEventLoop,
		request: &accesskit::ActionRequest,
	) {
		let routed = self
			.accessibility
			.as_ref()
			.and_then(|accessibility| accessibility.resolve_action(request));
		let Some(routed) = routed.filter(|action| action.document == self.doc_id) else {
			return;
		};
		match routed.apply(&mut self.doc.inst) {
			a11y::ActionResult::Ignored => return,
			a11y::ActionResult::Changed => {},
			a11y::ActionResult::Dispatch(event) => self.dispatch(event_loop, event),
		}
		if let Some(window) = &self.window {
			window.request_redraw();
		}
	}
}

impl<U, H> ApplicationHandler<ShellEvent<U>> for NativeShell<U, H>
where
	U: Send + 'static,
	H: ShellHost<U>,
{
	fn resumed(&mut self, event_loop: &ActiveEventLoop) {
		if self.window.is_some() {
			return;
		}
		let window = match event_loop.create_window(
			Window::default_attributes()
				.with_title(&self.opts.title)
				.with_inner_size(LogicalSize::new(self.opts.width, self.opts.height))
				.with_decorations(!self.opts.undecorated)
				.with_visible(false),
		) {
			Ok(w) => Arc::new(w),
			Err(e) => {
				eprintln!("slab-native: window creation failed: {e}");
				event_loop.exit();
				return;
			},
		};
		let accessibility =
			a11y::WindowAccessibility::new(event_loop, &window, self.a11y_proxy.clone());
		let instance = wgpu::Instance::default();
		let surface = match instance.create_surface(window.clone()) {
			Ok(s) => s,
			Err(e) => {
				eprintln!("slab-native: surface creation failed: {e}");
				event_loop.exit();
				return;
			},
		};
		crate::surface::enable_transactional_presents(&surface);
		let Some((adapter, device, queue)) = crate::request_device(&instance, Some(&surface)) else {
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
		self.doc_id =
			renderer.register_doc(self.doc.inst.doc(), &self.doc.imgs, self.doc.registered_fonts());
		self.renderer = Some(renderer);
		self.surface = Some(surface);
		self.window = Some(window.clone());
		self.accessibility = Some(accessibility);
		self.configure_surface();
		window.set_visible(true);
		window.request_redraw();
	}

	fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
		// Drop the surface before its window; mobile platforms invalidate both
		// across suspension. The retained kernel instance stays live.
		self.surface = None;
		self.renderer = None;
		self.accessibility = None;
		self.window = None;
	}

	fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
		if let (Some(accessibility), Some(window)) = (&mut self.accessibility, &self.window) {
			accessibility.process_event(window, &event);
		}
		match event {
			WindowEvent::CloseRequested => {
				let ev = self.base_event(kdispatch::E_CLOSE);
				self.dispatch(event_loop, ev);
				event_loop.exit();
			},
			WindowEvent::RedrawRequested => {
				self.draw();
				if let Some(max) = self.opts.max_frames
					&& self.frames >= max
				{
					event_loop.exit();
				}
			},
			WindowEvent::Resized(_) => {
				self.configure_surface();
				// Draw inside the resize transaction so the frame commits with
				// the new window bounds (see `surface::enable_transactional_presents`).
				self.draw();
			},
			WindowEvent::ScaleFactorChanged { .. } => {
				self.cursor_sample = None;
				self.configure_surface();
				self.draw();
			},
			WindowEvent::Occluded(occluded) => {
				self.occluded = occluded;
				// Draws skipped while hidden never queue retries; repaint as
				// soon as the window is visible again.
				if !occluded && let Some(window) = &self.window {
					window.request_redraw();
				}
			},
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
			},
			WindowEvent::CursorMoved { position, .. } => {
				let s = self.scale();
				let cursor = (position.x / s, position.y / s);
				let (dx, dy) = input::cursor_delta(&mut self.cursor_sample, cursor);
				self.cursor = cursor;
				let mut ev = self.base_event(kdispatch::E_POINTER_MOVE);
				ev.dx = dx;
				ev.dy = dy;
				self.dispatch(event_loop, ev);
			},
			WindowEvent::CursorLeft { .. } => {
				self.cursor_sample = None;
			},
			WindowEvent::MouseInput { state, button, .. } => {
				let btn = input::mouse_button_id(button);
				let clicks = if state == ElementState::Pressed {
					self.clicks.pointer_down(btn, self.cursor.0, self.cursor.1)
				} else {
					0
				};
				// window-drag regions start an OS move on press; a nested
				// act-bound control (close button in the titlebar) wins
				if state == ElementState::Pressed && btn == 0 {
					let chain = kframe::inst_hit(&self.doc.inst, self.cursor.0, self.cursor.1);
					for node in chain.iter().rev() {
						// nearest act-bound node decides: drag region → OS
						// move; anything else (app control) → kernel path
						let Some(sig) = act_signal(self.doc.inst.doc(), *node) else {
							continue;
						};
						if WindowCmd::from_signal(sig) == Some(WindowCmd::Drag)
							&& let Some(window) = &self.window
						{
							let _ = window.drag_window();
						}
						break;
					}
				}
				let etype = if state == ElementState::Pressed {
					kdispatch::E_POINTER_DOWN
				} else {
					kdispatch::E_POINTER_UP
				};
				let mut ev = self.base_event(etype);
				ev.button = btn;
				ev.clicks = clicks;
				self.dispatch(event_loop, ev);
				// Context signal dispatch is kernel mechanics. This reference
				// host then exposes real clipboard actions in the title bar.
				if state == ElementState::Pressed && btn == 2 && input::focus_in_field(&self.doc.inst) {
					self.set_context_actions(true);
				}
			},
			WindowEvent::MouseWheel { delta, .. } => {
				let (dx, dy) = match delta {
					MouseScrollDelta::LineDelta(x, y) => (-x as f64 * 40.0, -y as f64 * 40.0),
					MouseScrollDelta::PixelDelta(p) => {
						let s = self.scale();
						(-p.x / s, -p.y / s)
					},
				};
				let mut ev = self.base_event(kdispatch::E_WHEEL);
				ev.dx = dx;
				ev.dy = dy;
				self.dispatch(event_loop, ev);
			},
			WindowEvent::KeyboardInput { event, .. } => {
				if event.state != ElementState::Pressed || self.ime.composing() {
					return;
				}
				if self.context_action_key(event_loop, &event.logical_key) {
					return;
				}
				if self.clipboard_shortcut(event_loop, &event.logical_key) {
					return;
				}
				if let Some(name) = input::key_name(&event.logical_key) {
					let mut ev = self.base_event(kdispatch::E_KEY_DOWN);
					ev.key = name;
					self.dispatch(event_loop, ev);
				}
				let insertable = matches!(event.logical_key, Key::Character(_))
					|| event.logical_key == Key::Named(NamedKey::Space);
				let no_cmd = self.mods & (kdispatch::M_CTRL | kdispatch::M_META) == 0;
				if insertable
					&& no_cmd && self.ime.forwards_key_text()
					&& let Some(text) = &event.text
				{
					let mut ev = self.base_event(kdispatch::E_TEXT);
					ev.text = text.to_string();
					self.dispatch(event_loop, ev);
				}
			},
			WindowEvent::Ime(ime) => {
				for (etype, text) in self.ime.on_ime(ime) {
					let mut ev = self.base_event(etype);
					ev.text = text;
					self.dispatch(event_loop, ev);
				}
			},
			WindowEvent::Focused(false) => {
				let ev = self.base_event(kdispatch::E_BLUR);
				self.dispatch(event_loop, ev);
			},
			_ => {},
		}
	}

	fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ShellEvent<U>) {
		match event {
			ShellEvent::Accessibility(event) => {
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
					},
					a11y::EventKind::ActionRequested(request) => {
						self.accessibility_action(event_loop, &request);
					},
					a11y::EventKind::AccessibilityDeactivated => {},
				}
			},
			ShellEvent::User(event) => {
				let Some(window) = self.window.clone() else {
					return;
				};
				if self
					.host
					.user_event(&mut self.doc, &window, event_loop, event)
				{
					window.request_redraw();
				}
			},
			ShellEvent::Shutdown => {
				// Same path as the window close button: let the document see
				// E_CLOSE, then leave the loop so `main` exits with status 0.
				let ev = self.base_event(kdispatch::E_CLOSE);
				self.dispatch(event_loop, ev);
				event_loop.exit();
			},
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

#[cfg(test)]
mod tests {
	use super::*;

	/// A signal-only key effect leaves the kernel repaint flag clear, but a
	/// host that mutates the document inside `effects` must still redraw.
	#[test]
	fn redraw_follows_host_dirtied_instance() {
		let src = "col w=200 h=100 { row#go focusable keys=F2:save w=50 h=20 { when selected { \
		           opacity=0.5 } } }";
		let copts = slab_compile::Options::default();
		let (slir, _diags) = slab_compile::compile(src, &copts);
		let bytes = slab_slir::write(&slir.expect("fixture compiles"));
		let mut doc = NativeDocument::decode(&bytes).expect("fixture decodes");
		let _ = kframe::inst_frame(&mut doc.inst, 0.0);
		assert!(kframe::inst_set_focus(&mut doc.inst, "go", false), "row focuses");
		doc.inst.dirty = false;

		let ev = kdispatch::Event {
			etype:   kdispatch::E_KEY_DOWN,
			x:       0.0,
			y:       0.0,
			dx:      0.0,
			dy:      0.0,
			button:  0,
			clicks:  0,
			key:     "F2".into(),
			text:    String::new(),
			clauses: Vec::new(),
			mods:    0,
		};
		let eff = kframe::inst_dispatch(&mut doc.inst, &ev);
		assert_eq!(eff.sig_name.len(), 1, "F2 fires the save binding");
		assert!(!eff.repaint, "signal-only effect does not repaint");
		doc.inst.dirty = false;
		assert!(!needs_redraw(&eff, &doc.inst), "clean instance stays idle");

		// The host signal handler mutates the document (param/state write).
		kframe::inst_set_state(&mut doc.inst, "selected", true);
		assert!(needs_redraw(&eff, &doc.inst), "host-dirtied instance redraws");
	}

	/// A `keys=` clipboard chord is claimed by the host and must bypass the
	/// shell's built-in cut/copy/paste recipe.
	#[test]
	fn host_keys_claim_clipboard_chords() {
		let src = "col w=200 h=100 { row#go focusable keys=v:smart_paste w=50 h=20 }";
		let copts = slab_compile::Options::default();
		let (slir, _diags) = slab_compile::compile(src, &copts);
		let bytes = slab_slir::write(&slir.expect("fixture compiles"));
		let mut doc = NativeDocument::decode(&bytes).expect("fixture decodes");
		let _ = kframe::inst_frame(&mut doc.inst, 0.0);
		assert!(kframe::inst_set_focus(&mut doc.inst, "go", false), "row focuses");

		assert!(kframe::inst_key_claimed(&doc.inst, "v"), "bound chord is claimed");
		assert!(!kframe::inst_key_claimed(&doc.inst, "c"), "unbound chord is free");

		let plain = "col w=200 h=100 { row#go focusable w=50 h=20 }";
		let (slir, _diags) = slab_compile::compile(plain, &copts);
		let bytes = slab_slir::write(&slir.expect("plain fixture compiles"));
		let mut doc = NativeDocument::decode(&bytes).expect("plain fixture decodes");
		let _ = kframe::inst_frame(&mut doc.inst, 0.0);
		assert!(kframe::inst_set_focus(&mut doc.inst, "go", false), "plain row focuses");
		assert!(!kframe::inst_key_claimed(&doc.inst, "v"), "no binding keeps the recipe");
	}
}
