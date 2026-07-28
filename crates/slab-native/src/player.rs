//! `slab-native --demo player`: the 00-player music card in a winit window.
//!
//! `PlayerCore` is the windowless app: main doc + one precompiled queue
//! child instance per playlist rotation (the playing marker baked in),
//! pointer routing into the hole (coordinates translated, hover leave
//! synthesized), child `pick` signals selecting the clicked queue entry,
//! and the playback clock. The winit `App` and the headless tests drive the
//! same core; `--headless-frame` renders one offscreen PNG with self-checking
//! pixel probes. Queue rows mirror the document's exported `Track` def
//! (focusable act=pick, hover/pressed/focus-visible, 140ms ease-out).

use std::{fmt::Write as _, path::PathBuf, sync::Arc, time::Instant};

use slab_kernel::{
	dispatch as kdispatch,
	dispatch::Event,
	flatten::Frame,
	frame::{self as kframe, HoleRect},
	scene as kscene, slir as kslir,
};
use winit::{
	application::ApplicationHandler,
	dpi::LogicalSize,
	event::{ElementState, MouseScrollDelta, WindowEvent},
	event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
	window::{Window, WindowId},
};

use crate::{
	a11y,
	demo::{Opts, write_png},
	gen_player,
	holes::{HoleContent, InstanceHole},
	input::{self, ClickCounter},
	renderer::{LayerInput, Renderer},
};

// ------------------------------------------------------------- playlist ----

struct TrackDef {
	no:     &'static str,
	title:  &'static str,
	artist: &'static str,
	len_s:  f64,
}

/// The Sunset Tree side, matching the document's defaults: track 1 starts at
/// 2:37 of 4:12 = 62% — exactly the .slab param defaults.
const PLAYLIST: [TrackDef; 4] = [
	TrackDef {
		no:     "01",
		title:  "Pale Green Things",
		artist: "The Mountain Goats",
		len_s:  252.0,
	},
	TrackDef { no: "02", title: "This Year", artist: "The Mountain Goats", len_s: 232.0 },
	TrackDef { no: "03", title: "Love Love Love", artist: "The Mountain Goats", len_s: 200.0 },
	TrackDef {
		no:     "04",
		title:  "Dinu Lipatti's Bones",
		artist: "The Mountain Goats",
		len_s:  192.0,
	},
];

/// mm:ss (no hours in this playlist).
fn fmt_time(s: f64) -> String {
	let s = s.max(0.0).round() as u64;
	format!("{}:{:02}", s / 60, s % 60)
}

/// The queue hole's child document with the playing marker on `playing_idx`:
/// all four album tracks in order, each a row mirroring the document's
/// exported `Track` def — focusable act=pick, hover/pressed/focus-visible
/// states, 140ms ease-out transition, and the mint gradient on the playing
/// row.
fn queue_src(playing_idx: usize) -> String {
	let mut src = String::from(
		r#"tokens {
  color {
    faint #5F7263
    muted #9AB0A0
    moss  #1B2E22
    mint  oklch(86% 0.13 155)
  }
  text {
    body { family "Inter"; size 13 }
    mono { family "Berkeley Mono"; size 11 }
  }
}

def Track(no, title, len, playing=false) {
  row focusable act=pick pad=8,12 gap=12 align=center radius=10 transition=140,ease-out {
    text no style=text.mono color=color.faint w=18
    text title style=text.body color=color.muted w=fill ellipsis nowrap
    text len style=text.mono color=color.faint w=36 align-text=end
    when playing {
      bg=linear(90, #7FE0A824 0%, #7FE0A805 100%)
      stroke=#7FE0A82E
    }
    when hover { bg=color.moss }
    when pressed { bg=#0B1A11 }
    when focus-visible { stroke=color.mint stroke-w=2 }
  }
}

col w=fill h=fill scroll clip {
"#,
	);
	for (k, t) in PLAYLIST.iter().enumerate() {
		writeln!(
			&mut src,
			"  Track key=track{} no=\"{}\" title=\"{}\" len=\"{}\"{}",
			k,
			t.no,
			t.title,
			fmt_time(t.len_s),
			if k == playing_idx {
				" playing=true"
			} else {
				""
			}
		)
		.expect("writing to a String cannot fail");
	}
	src.push_str("}\n");
	src
}

fn queue_slir(playing_idx: usize) -> Result<Vec<u8>, String> {
	let opts = slab_compile::Options {
		embed_assets: true,
		base_dir: PathBuf::from("."),
		..slab_compile::Options::default()
	};
	let (slir, diags) = slab_compile::compile(&queue_src(playing_idx), &opts);
	let slir = slir.ok_or_else(|| format!("queue doc failed to compile: {:?}", diags.0))?;
	Ok(slab_slir::write(&slir))
}

/// The queue doc's `act=pick` row nodes in SIGN (= album) order.
fn pick_nodes(d: &kslir::Doc) -> Vec<u32> {
	let mut out = Vec::new();
	for k in 0..d.sign_name.len() {
		if d.sign_trigger[k] == 0 && kslir::str_at(d, d.sign_name[k]) == "pick" {
			out.push(d.sign_node[k]);
		}
	}
	out
}

// ---------------------------------------------------------- player state ----

struct PlayerState {
	idx:     usize,
	pos_s:   f64,
	playing: bool,
}

impl PlayerState {
	const fn new() -> Self {
		// Match the document's param defaults: 2:37 into track 1, playing.
		Self { idx: 0, pos_s: 157.0, playing: true }
	}

	const fn track(&self) -> &'static TrackDef {
		&PLAYLIST[self.idx]
	}

	/// Push title/artist/times/progress/playing into the kernel instance.
	fn apply(&self, doc: &mut gen_player::Doc) {
		let t = self.track();
		doc.set_title(t.title);
		doc.set_artist(t.artist);
		doc.set_elapsed(&fmt_time(self.pos_s));
		doc.set_remain(&format!("-{}", fmt_time(t.len_s - self.pos_s)));
		doc.set_progress((self.pos_s / t.len_s * 100.0).clamp(0.0, 100.0));
		doc.set_playing(self.playing);
	}
}

// ----------------------------------------------------------------- core ----

/// Which instance holds the pointer capture / hover.
#[derive(Clone, Copy, PartialEq)]
enum Route {
	Main,
	Hole(usize),
}

const A11Y_MAIN_DOCUMENT: usize = 0;
const A11Y_QUEUE_DOCUMENT: usize = 1;

/// Kernel-side effects of one routed event.
pub struct CoreOut {
	pub repaint:       bool,
	pub cursor:        u32,
	/// Signals the main document emitted (already applied to the state).
	pub signals:       Vec<gen_player::Signal>,
	/// Non-pick signals emitted by the mounted queue document.
	pub queue_signals: Vec<(String, String)>,
	/// A queue row was picked: the (0-based) playlist index now current.
	pub picked:        Option<usize>,
}

/// The windowless player app: everything except the GPU and the window.
pub struct PlayerCore {
	pub doc:             gen_player::Doc,
	/// One precompiled queue rotation per playlist track; `track_index()`
	/// picks the mounted one (playing marker baked into each doc).
	pub queues:          Vec<InstanceHole>,
	pub queue_bytes:     Vec<Vec<u8>>,
	pub hole_rects:      Vec<HoleRect>,
	state:               PlayerState,
	capture:             Option<Route>,
	focus_route:         Route,
	pending_queue_focus: Option<String>,
	hover_route:         Route,
	dark:                bool,
	coarse:              bool,
	reported_queue:      Option<usize>,
}

impl PlayerCore {
	pub fn new() -> Result<Self, String> {
		let mut doc = gen_player::Doc::new();
		if !doc.ok() {
			return Err(format!("embedded SLIR failed to decode: {:?}", doc.inst.doc().errs));
		}
		let queue_bytes = (0..PLAYLIST.len())
			.map(queue_slir)
			.collect::<Result<Vec<_>, _>>()?;
		let mut queues = queue_bytes
			.iter()
			.map(|b| InstanceHole::new(b).ok_or_else(|| "queue SLIR failed to decode".to_string()))
			.collect::<Result<Vec<_>, _>>()?;
		let state = PlayerState::new();
		let reported_queue = state.idx;
		let natural = queues[state.idx].natural();
		kframe::inst_set_hole_size(&mut doc.inst, 0, natural.0, natural.1);
		state.apply(&mut doc);
		Ok(Self {
			doc,
			queues,
			queue_bytes,
			hole_rects: Vec::new(),
			state,
			capture: None,
			hover_route: Route::Main,
			focus_route: Route::Main,
			pending_queue_focus: None,
			dark: false,
			coarse: false,
			reported_queue: Some(reported_queue),
		})
	}

	pub fn set_env(&mut self, vw: f64, vh: f64, dark: bool, coarse: bool) {
		self.dark = dark;
		self.coarse = coarse;
		self.doc.set_env(vw, vh, dark, coarse);
	}

	pub const fn track_index(&self) -> usize {
		self.state.idx
	}

	pub const fn playing(&self) -> bool {
		self.state.playing
	}

	pub const fn title(&self) -> &'static str {
		self.state.track().title
	}

	/// Advance the playback clock by `dt` seconds; auto-advances at track
	/// end. The dirty instance is re-solved on the next `frame()`.
	pub fn tick(&mut self, dt: f64) {
		if !self.state.playing {
			return;
		}
		self.state.pos_s += dt;
		if self.state.pos_s >= self.state.track().len_s {
			self.select_track((self.state.idx + 1) % PLAYLIST.len());
		} else {
			self.state.apply(&mut self.doc);
		}
	}

	/// Main-document frame; refreshes the hole rects and resizes the
	/// mounted queue instance to the hole viewport.
	pub fn frame(&mut self, t_ms: f64) -> Frame {
		let idx = self.state.idx;
		if self.reported_queue != Some(idx) || self.queues[idx].inst.dirty {
			let natural = self.queues[idx].natural();
			kframe::inst_set_hole_size(&mut self.doc.inst, 0, natural.0, natural.1);
			self.reported_queue = Some(idx);
		}
		let fr = self.doc.frame(t_ms);
		let pending = kframe::inst_take_signals(&mut self.doc.inst);
		let signals = self.doc.decode_signals(&pending);
		for signal in &signals {
			self.on_signal(signal);
			println!("signal: {signal:?}");
		}
		self.hole_rects = self.doc.holes();
		if let Some(hr) = self.hole_rects.first() {
			self.queues[self.state.idx].resize(hr.w, hr.h, self.dark, self.coarse);
		}
		fr
	}

	/// Frame of the mounted queue instance (call after `frame()` so the
	/// hole viewport is applied).
	pub fn queue_frame(&mut self, t_ms: f64) -> Frame {
		let pending_focus = self.pending_queue_focus.take();
		let queue = &mut self.queues[self.state.idx];
		let frame = queue.frame(t_ms);
		let frame = if let Some(key) = pending_focus
			&& kframe::inst_set_focus(&mut queue.inst, &key, true)
		{
			queue.frame(t_ms)
		} else {
			frame
		};
		let pending = kframe::inst_take_signals(&mut queue.inst);
		for name in pending.sig_name {
			println!("signal: {}", kslir::str_at(&queue.inst.doc(), name));
		}
		frame
	}

	/// The mounted queue instance.
	pub fn queue(&self) -> &InstanceHole {
		&self.queues[self.state.idx]
	}

	/// True while another frame is needed (playback clock, main dirty or
	/// animating, or the mounted queue dirty or animating).
	pub fn needs_frame(&self) -> bool {
		self.state.playing
			|| self.doc.inst.dirty
			|| self.doc.inst.ms.active
			|| self.queues[self.state.idx].needs_frame()
	}

	/// Queue row `row` (album order) in the mounted queue: the `act=pick`
	/// node id and its child-local scene rect. Needs a solved queue frame.
	pub fn queue_row(&self, row: usize) -> Option<(u32, (f64, f64, f64, f64))> {
		let q = &self.queues[self.state.idx];
		let node = *pick_nodes(&q.inst.doc()).get(row)?;
		let ix = kscene::index_of(&q.inst.sc, node);
		if ix < 0 {
			return None;
		}
		let ix = ix as usize;
		let entry = &q.inst.sc.entries[ix];
		Some((node, (entry.x, entry.y, entry.w, entry.h)))
	}

	/// Route a pointer-ish event to the queue hole under it or main.
	fn route_of(&self, x: f64, y: f64) -> Route {
		if let Some(r) = self.capture {
			return r;
		}
		for (i, hr) in self.hole_rects.iter().enumerate() {
			if x >= hr.x && x < hr.x + hr.w && y >= hr.y && y < hr.y + hr.h {
				return Route::Hole(i);
			}
		}
		Route::Main
	}

	fn transfer_focus(&mut self, route: Route) {
		match route {
			Route::Main => {
				for queue in &mut self.queues {
					kframe::inst_set_focus(&mut queue.inst, "", false);
				}
			},
			Route::Hole(_) => {
				kframe::inst_set_focus(&mut self.doc.inst, "", false);
				for (index, queue) in self.queues.iter_mut().enumerate() {
					if index != self.state.idx {
						kframe::inst_set_focus(&mut queue.inst, "", false);
					}
				}
			},
		}
		self.pending_queue_focus = None;
		self.focus_route = route;
	}

	/// Route one window-coordinate event: pointer events go to the hole
	/// under the pointer (translated into hole-local coordinates) or main;
	/// close follows pointer capture, while keys and blur follow focus.
	pub fn dispatch(&mut self, ev: &Event) -> CoreOut {
		let mut out = CoreOut {
			repaint:       false,
			cursor:        kdispatch::CUR_DEFAULT,
			signals:       Vec::new(),
			queue_signals: Vec::new(),
			picked:        None,
		};
		let pointer = matches!(
			ev.etype,
			kdispatch::E_POINTER_MOVE
				| kdispatch::E_POINTER_DOWN
				| kdispatch::E_POINTER_UP
				| kdispatch::E_WHEEL
		);
		let route = if ev.etype == kdispatch::E_CLOSE {
			self.capture.unwrap_or(Route::Main)
		} else if pointer {
			self.route_of(ev.x, ev.y)
		} else {
			self.focus_route
		};

		// The pointer crossed the hole boundary: synthesize a leave move so
		// the instance it left drops its hover state (and eases back).
		if ev.etype == kdispatch::E_POINTER_MOVE && route != self.hover_route {
			let mut leave = ev.clone();
			leave.x = -1.0e6;
			leave.y = -1.0e6;
			match self.hover_route {
				Route::Main => {
					let (eff, _) = self.doc.dispatch(&leave);
					out.repaint |= eff.repaint;
				},
				Route::Hole(_) => {
					out.repaint |= self.queues[self.state.idx].dispatch(&leave).repaint;
				},
			}
			self.hover_route = route;
		}
		if ev.etype == kdispatch::E_POINTER_DOWN && ev.button == 0 {
			self.transfer_focus(route);
			self.capture = Some(route);
		} else if ev.etype == kdispatch::E_POINTER_UP && ev.button == 0 {
			self.capture = None;
		}

		match route {
			Route::Main => {
				let (eff, sigs) = self.doc.dispatch(ev);
				out.repaint |= eff.repaint || !sigs.is_empty();
				out.cursor = eff.cursor;
				for s in &sigs {
					self.on_signal(s);
				}
				out.signals = sigs;
			},
			Route::Hole(i) => {
				let Some(hr) = self.hole_rects.get(i).cloned() else {
					return out;
				};
				let mut cev = ev.clone();
				cev.x -= hr.x;
				cev.y -= hr.y;
				let queue = self.dispatch_queue(&cev);
				out.repaint |= queue.repaint;
				out.cursor = queue.cursor;
				out.picked = queue.picked;
				out.queue_signals = queue.queue_signals;
			},
		}
		out
	}

	/// Dispatches an event already translated into the mounted queue's space.
	fn dispatch_queue(&mut self, ev: &Event) -> CoreOut {
		let mut out = CoreOut {
			repaint:       false,
			cursor:        kdispatch::CUR_DEFAULT,
			signals:       Vec::new(),
			queue_signals: Vec::new(),
			picked:        None,
		};
		let idx = self.state.idx;
		let (repaint, cursor, picked_node, signals) = {
			let queue = &mut self.queues[idx];
			let effects = queue.dispatch(ev);
			let mut picked = false;
			let mut signals = Vec::new();
			for (k, &name_ref) in effects.sig_name.iter().enumerate() {
				let name = kslir::str_at(&queue.inst.doc(), name_ref);
				if name == "pick" {
					picked = true;
				} else {
					signals.push((name.to_owned(), effects.sig_text[k].clone()));
				}
			}
			(effects.repaint, effects.cursor, picked.then_some(queue.inst.ds.fs.focus), signals)
		};
		out.repaint = repaint;
		out.cursor = cursor;
		out.queue_signals = signals;
		if let Some(node) = picked_node
			&& let Some(row) = pick_nodes(&self.queues[idx].inst.doc())
				.iter()
				.position(|&candidate| candidate == node)
		{
			self.select_track(row);
			out.picked = Some(row);
			out.repaint = true;
		}
		out
	}

	fn on_signal(&mut self, sig: &gen_player::Signal) {
		use gen_player::Signal;
		match sig {
			Signal::Toggle { .. } => {
				self.state.playing = !self.state.playing;
				self.state.apply(&mut self.doc);
			},
			Signal::Next { .. } => self.change_track(1),
			Signal::Prev { .. } => self.change_track(PLAYLIST.len() - 1),
			Signal::Shuffle { .. } | Signal::Loop { .. } => {
				eprintln!("player: {sig:?} acknowledged (no-op in this demo)");
			},
		}
	}

	fn change_track(&mut self, step: usize) {
		self.select_track((self.state.idx + step) % PLAYLIST.len());
	}

	/// Make playlist entry `k` current: restart at 0:00, keep the play
	/// state, and mount the queue rotation with the playing marker on `k`.
	fn select_track(&mut self, k: usize) {
		let old = self.state.idx;
		let next = k % PLAYLIST.len();
		let focus_key = if old != next && matches!(self.focus_route, Route::Hole(_)) {
			kscene::key_of(
				&self.queues[old].inst.doc(),
				&self.queues[old].inst.st.lists,
				self.queues[old].inst.ds.fs.focus,
			)
		} else {
			String::new()
		};
		self.state.idx = next;
		self.state.pos_s = 0.0;
		if old != next {
			kframe::inst_set_focus(&mut self.queues[old].inst, "", false);
			if !focus_key.is_empty() {
				self.pending_queue_focus = Some(focus_key);
			}
		}
		self.state.apply(&mut self.doc);
	}
}

fn player_core(theme: Option<&str>) -> Result<PlayerCore, String> {
	let mut core = PlayerCore::new()?;
	if let Some(name) = theme
		&& !core.doc.set_theme(name)
	{
		return Err(format!("unknown theme '{name}'"));
	}
	Ok(core)
}

// ------------------------------------------------------------- headless ----

/// Render one frame offscreen and write a PNG; asserts two probe pixels
/// (card night bg, mint play circle) so the artifact is self-checking.
pub fn headless_frame(opts: &Opts) -> Result<(), String> {
	let out = opts.headless_out.clone().ok_or("missing output path")?;
	let instance = wgpu::Instance::default();
	let (adapter, device, queue) =
		crate::request_device(&instance, None).ok_or("no wgpu adapter available (headless)")?;
	eprintln!("slab-native: adapter {} ({:?})", adapter.get_info().name, adapter.get_info().backend);
	let mut renderer = Renderer::new(device, queue);

	let mut core = player_core(opts.theme.as_deref())?;
	core.set_env(opts.width, opts.height, opts.dark, false);
	let main_id = renderer.register_doc(&core.doc.inst.doc(), &core.doc.imgs, &[]);
	let idx = core.track_index();
	let queue_id = renderer.register_doc(&core.queues[idx].inst.doc(), &core.queues[idx].imgs, &[]);

	let fr = core.frame(opts.t);
	let cf = core.queue_frame(opts.t);

	let scale = opts.scale.unwrap_or(1.0);
	let tw = (opts.width * scale).ceil() as u32;
	let th = (opts.height * scale).ceil() as u32;
	let mut layers = vec![LayerInput {
		doc_id: main_id,
		inst:   &core.doc.inst,
		frame:  &fr,
		ox:     0.0,
		oy:     0.0,
		clip:   None,
	}];
	if let Some(hr) = core.hole_rects.first() {
		layers.push(LayerInput {
			doc_id: queue_id,
			inst:   core.queue().instance(),
			frame:  &cf,
			ox:     hr.x,
			oy:     hr.y,
			clip:   Some((hr.x, hr.y, hr.w, hr.h, 0.0)),
		});
	}
	let build = renderer.build(&layers, scale, tw, th);
	renderer.render(build, None, wgpu::Color::BLACK);
	let (w, h, px) = renderer.read_pixels().ok_or("readback failed")?;

	let probe = |x: u32, y: u32| -> [u8; 3] {
		let i = ((y.min(h - 1) * w + x.min(w - 1)) * 4) as usize;
		[px[i], px[i + 1], px[i + 2]]
	};
	// card bg: color.night #0A120D, left edge below the artwork
	let night = probe((5.0 * scale) as u32, (200.0 * scale) as u32);
	let close = |got: [u8; 3], want: [u8; 3]| got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 3);
	if !close(night, [0x0a, 0x12, 0x0d]) {
		return Err(format!("night probe {night:?} != #0A120D"));
	}
	// play circle: mint gradient #B9F5CE..#7FE0A8 — probe above the glyph
	let play = fr
		.ops
		.iter()
		.find_map(|op| match op {
			slab_kernel::flatten::FrameOp::Rect(r) if r.w == 44.0 && r.h == 44.0 => {
				Some((r.x + 22.0, r.y + 8.0))
			},
			_ => None,
		})
		.ok_or("no 44x44 play circle rect in frame ops")?;
	let mint = probe((play.0 * scale) as u32, (play.1 * scale) as u32);
	let band = |v: u8, lo: u8, hi: u8| (lo.saturating_sub(6)..=hi.saturating_add(6)).contains(&v);
	if !(band(mint[0], 0x7f, 0xb9) && band(mint[1], 0xe0, 0xf5) && band(mint[2], 0xa8, 0xce)) {
		return Err(format!("play-circle probe {mint:?} outside mint band"));
	}

	write_png(&out, w, h, &px)?;
	eprintln!(
		"slab-native: headless-frame OK ({w}x{h}px, night {night:?}, mint {mint:?}) -> {}",
		out.display()
	);
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

struct App {
	opts:           Opts,
	window:         Option<Arc<Window>>,
	a11y_proxy:     EventLoopProxy<a11y::Event>,
	accessibility:  Option<a11y::WindowAccessibility>,
	surface:        Option<wgpu::Surface<'static>>,
	surface_format: wgpu::TextureFormat,
	renderer:       Option<Renderer>,
	core:           PlayerCore,
	main_id:        usize,
	/// Renderer doc id per queue rotation, parallel with `core.queues`.
	queue_ids:      Vec<usize>,
	last_tick:      Instant,
	mods:           u32,
	cursor:         (f64, f64),
	cursor_sample:  Option<(f64, f64)>,
	clicks:         ClickCounter,
	start:          Instant,
	frames:         u64,
	exit_deadline:  Option<Instant>,
}

impl App {
	fn new(opts: Opts, a11y_proxy: EventLoopProxy<a11y::Event>) -> Result<Self, String> {
		let core = player_core(opts.theme.as_deref())?;
		Ok(Self {
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
			core,
			main_id: 0,
			queue_ids: Vec::new(),
			last_tick: Instant::now(),
			mods: 0,
			cursor: (0.0, 0.0),
			cursor_sample: None,
			clicks: ClickCounter::default(),
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
		self
			.core
			.set_env(size.width as f64 / s, size.height as f64 / s, self.opts.dark, false);
	}

	fn refresh_accessibility(
		&mut self,
		frame: &Frame,
		queue_frame: &Frame,
		size: winit::dpi::PhysicalSize<u32>,
	) {
		let scale = self.scale();
		let idx = self.core.track_index();
		let mut layers = Vec::with_capacity(2);
		layers.push(a11y::SceneLayer::new(A11Y_MAIN_DOCUMENT, &self.core.doc.inst, frame));
		if let Some(rect) = self.core.hole_rects.first() {
			let mut layer =
				a11y::SceneLayer::new(A11Y_QUEUE_DOCUMENT, &self.core.queues[idx].inst, queue_frame)
					.translated(rect.x, rect.y);
			if let Some(node) = usize::try_from(rect.hole)
				.ok()
				.and_then(|hole| self.core.doc.inst.doc().hole_node.get(hole))
				.copied()
			{
				layer = layer.mounted(A11Y_MAIN_DOCUMENT, node);
			}
			layers.push(layer);
		}
		if let Some(accessibility) = &mut self.accessibility {
			accessibility.refresh(
				"slab — player",
				f64::from(size.width) / scale,
				f64::from(size.height) / scale,
				scale,
				&layers,
			);
			accessibility.update(false);
		}
	}

	/// Advance the playback clock; the dirty instance is re-solved by the
	/// kernel on the next `frame()`.
	fn tick_playback(&mut self) {
		let now = Instant::now();
		let dt = now.duration_since(self.last_tick).as_secs_f64();
		self.last_tick = now;
		self.core.tick(dt);
	}

	fn draw(&mut self) {
		self.tick_playback();
		let t = self.t_ms();
		let Some(window) = self.window.clone() else {
			return;
		};
		let size = window.inner_size();
		if size.width == 0 || size.height == 0 {
			return;
		}
		let fr = self.core.frame(t);
		let cf = self.core.queue_frame(t);
		let idx = self.core.track_index();
		self.refresh_accessibility(&fr, &cf, size);

		let Some(renderer) = self.renderer.as_mut() else {
			return;
		};
		let Some(surface) = self.surface.as_ref() else {
			return;
		};
		let mut layers = vec![LayerInput {
			doc_id: self.main_id,
			inst:   &self.core.doc.inst,
			frame:  &fr,
			ox:     0.0,
			oy:     0.0,
			clip:   None,
		}];
		if let (Some(hr), Some(&doc_id)) = (self.core.hole_rects.first(), self.queue_ids.get(idx)) {
			layers.push(LayerInput {
				doc_id,
				inst: self.core.queues[idx].instance(),
				frame: &cf,
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

		if self.core.needs_frame() || self.opts.max_frames.is_some() {
			window.request_redraw();
		}
	}

	/// Route one event through the core and apply its window effects.
	fn forward(&mut self, ev: Event) {
		let out = self.core.dispatch(&ev);
		self.apply_core_out(out);
	}

	fn apply_core_out(&mut self, out: CoreOut) {
		for signal in &out.signals {
			println!("signal: {signal:?}");
			if matches!(signal, gen_player::Signal::Toggle { .. }) {
				// Don't count the paused wall time as playback.
				self.last_tick = Instant::now();
			}
		}
		for (name, text) in &out.queue_signals {
			if text.is_empty() {
				println!("signal: {name}");
			} else {
				println!("signal: {name} {text:?}");
			}
		}
		if let Some(row) = out.picked {
			println!("signal: Pick -> track {} ({})", row + 1, self.core.title());
		}
		let Some(window) = &self.window else { return };
		if out.repaint {
			window.request_redraw();
		}
		window.set_cursor(input::cursor_icon(out.cursor));
	}

	fn accessibility_action(&mut self, request: &accesskit::ActionRequest) {
		let routed = self
			.accessibility
			.as_ref()
			.and_then(|accessibility| accessibility.resolve_action(request));
		let Some(routed) = routed else {
			return;
		};
		let idx = self.core.track_index();
		let (route, result) = match routed.document {
			A11Y_MAIN_DOCUMENT => (Route::Main, routed.apply(&mut self.core.doc.inst)),
			A11Y_QUEUE_DOCUMENT if idx < self.core.queues.len() => {
				(Route::Hole(0), routed.apply(&mut self.core.queues[idx].inst))
			},
			_ => return,
		};
		if matches!(&result, a11y::ActionResult::Ignored) {
			return;
		}
		if routed.moves_focus() {
			self.core.transfer_focus(route);
		}
		match result {
			a11y::ActionResult::Ignored => unreachable!(),
			a11y::ActionResult::Changed => {},
			a11y::ActionResult::Dispatch(event) => self.forward(event),
		}
		if let Some(window) = &self.window {
			window.request_redraw();
		}
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
			mods: self.mods,
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
				.with_title("slab — player")
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
		self.main_id = renderer.register_doc(&self.core.doc.inst.doc(), &self.core.doc.imgs, &[]);
		for q in &self.core.queues {
			self
				.queue_ids
				.push(renderer.register_doc(&q.inst.doc(), &q.imgs, &[]));
		}
		self.renderer = Some(renderer);
		self.surface = Some(surface);
		self.window = Some(window.clone());
		self.accessibility = Some(accessibility);
		self.configure_surface();
		window.set_visible(true);
		self.last_tick = Instant::now();
		window.request_redraw();
	}

	fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
		if let (Some(accessibility), Some(window)) = (&mut self.accessibility, &self.window) {
			accessibility.process_event(window, &event);
		}
		match event {
			WindowEvent::CloseRequested => {
				let ev = self.base_event(kdispatch::E_CLOSE);
				self.forward(ev);
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
			WindowEvent::Occluded(false) => {
				// Draws skipped while hidden never queue retries; repaint as
				// soon as the window is visible again.
				if let Some(window) = &self.window {
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
				self.forward(ev);
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
				let etype = if state == ElementState::Pressed {
					kdispatch::E_POINTER_DOWN
				} else {
					kdispatch::E_POINTER_UP
				};
				let mut ev = self.base_event(etype);
				ev.button = btn;
				ev.clicks = clicks;
				self.forward(ev);
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
				self.forward(ev);
			},
			WindowEvent::KeyboardInput { event, .. } => {
				if event.state != ElementState::Pressed {
					return;
				}
				if let Some(name) = input::key_name(&event.logical_key) {
					let mut ev = self.base_event(kdispatch::E_KEY_DOWN);
					ev.key = name;
					self.forward(ev);
				}
			},
			WindowEvent::Focused(false) => {
				let ev = self.base_event(kdispatch::E_BLUR);
				self.forward(ev);
			},
			_ => {},
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
			},
			a11y::EventKind::ActionRequested(request) => {
				self.accessibility_action(&request);
			},
			a11y::EventKind::AccessibilityDeactivated => {},
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
