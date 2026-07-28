//! Embed a Slab document as a ratatui [`StatefulWidget`].
//!
//! [`SlabState`] owns the live kernel instance; [`SlabWidget`] blits its
//! terminal cell grid into the ratatui buffer, mapping colors and styles the
//! same way `slab-tui`'s ANSI painter does. Input flows back through
//! [`SlabState::handle_event`], which translates crossterm events with
//! `slab-tui`'s translator and offsets pointer coordinates by the widget area.
//!
//! ```ignore
//! use ratatui::layout::Rect;
//! use slab_ratatui::{SlabState, SlabWidget};
//!
//! let mut state = SlabState::from_file(std::path::Path::new("app.slab"))?;
//! let mut terminal = ratatui::init();
//! let mut area = Rect::default();
//! loop {
//!     terminal.draw(|f| {
//!         area = f.area();
//!         f.render_stateful_widget(SlabWidget, area, &mut state);
//!     })?;
//!     let event = slab_tui::crossterm::event::read()?;
//!     state.handle_event(&event, area);
//!     for signal in state.drain_signals() {
//!         if signal.name == "quit" { break; }
//!     }
//! }
//! ratatui::restore();
//! ```

use std::path::Path;

use ratatui::{
	buffer::Buffer,
	layout::Rect,
	style::{Color, Modifier},
	widgets::StatefulWidget,
};
use slab_kernel::{cells, dispatch, frame as kframe};
use slab_tui::{
	HostKey, KeyHandling, Signal, Translated, Translator,
	crossterm::event::{Event, MouseEvent},
};

/// Live Slab document state driven by a host-owned ratatui render loop.
pub struct SlabState {
	/// Render documents for the dark terminal media flag when `true`.
	pub dark:   bool,
	/// Report a coarse (touch-like) pointer to the kernel when `true`.
	pub coarse: bool,
	inst:       kframe::Instance,
	/// Embedded image payloads decoded from the SLIR container.
	images:     Vec<Vec<u8>>,
	signals:    Vec<Signal>,
	translator: Translator,
	grid:       Option<cells::CellGrid>,
	area:       (u16, u16),
	clock_ms:   f64,
}

impl SlabState {
	/// Builds document state from encoded SLIR bytes.
	pub fn from_slir(bytes: &[u8]) -> Result<Self, String> {
		let (inst, images) = slab_slir::instance(bytes)?;
		Ok(Self {
			dark: false,
			coarse: false,
			inst,
			images,
			signals: Vec::new(),
			translator: Translator::new(),
			grid: None,
			area: (0, 0),
			clock_ms: 0.0,
		})
	}

	/// Compiles a `.slab` file and builds its document state.
	pub fn from_file(path: &Path) -> Result<Self, String> {
		Self::from_slir(&compile(path)?)
	}

	/// Embedded image payloads shipped inside the document's SLIR container.
	pub fn images(&self) -> &[Vec<u8>] {
		&self.images
	}

	/// Borrows the live kernel instance for host-driven parameter writes.
	pub const fn instance_mut(&mut self) -> &mut kframe::Instance {
		&mut self.inst
	}

	/// Borrows the live kernel instance for host queries.
	pub const fn instance(&self) -> &kframe::Instance {
		&self.inst
	}

	/// Translates one crossterm event, dispatches it to the kernel, and queues
	/// any emitted signals; returns `true` when the kernel consumed input.
	/// Pointer coordinates are offset by `area` so mouse events hit the cells
	/// the widget actually painted; mouse events outside `area` are dropped.
	pub fn handle_event(&mut self, event: &Event, area: Rect) -> bool {
		self.handle_event_with(event, area, |_, _| KeyHandling::Forward)
	}

	/// Handles an event with a host shortcut layer matching `slab_tui::run`.
	///
	/// `on_key` runs only for translated keys outside edit fields. Returning
	/// [`KeyHandling::Consumed`] suppresses both the key and its paired text
	/// event; [`KeyHandling::Forward`] preserves normal kernel handling.
	pub fn handle_event_with(
		&mut self,
		event: &Event,
		area: Rect,
		mut on_key: impl FnMut(&mut kframe::Instance, &HostKey) -> KeyHandling,
	) -> bool {
		let event = match event {
			Event::Mouse(mouse) => {
				let inside = mouse.column >= area.x
					&& mouse.column < area.x.saturating_add(area.width)
					&& mouse.row >= area.y
					&& mouse.row < area.y.saturating_add(area.height);
				if !inside {
					return false;
				}
				Event::Mouse(MouseEvent {
					column: mouse.column - area.x,
					row: mouse.row - area.y,
					..*mouse
				})
			},
			other => other.clone(),
		};
		match slab_tui::translate(&mut self.translator, event) {
			Translated::Events(pair) => {
				let (first, second) = *pair;
				if let Some(key) = slab_tui::host_key(&self.inst, &first)
					&& on_key(&mut self.inst, &key) == KeyHandling::Consumed
				{
					return true;
				}
				for event in std::iter::once(first).chain(second) {
					let effects = kframe::inst_dispatch(&mut self.inst, &event);
					collect_signals(&self.inst, &effects, &mut self.signals);
				}
				true
			},
			Translated::Ignored | Translated::Resize(..) | Translated::Quit => false,
		}
	}

	/// Advances the document clock by `ms` milliseconds for animation frames.
	pub fn tick(&mut self, ms: f64) {
		self.clock_ms += ms;
	}

	/// Takes every signal emitted since the previous drain, oldest first.
	pub fn drain_signals(&mut self) -> Vec<Signal> {
		std::mem::take(&mut self.signals)
	}

	/// Applies the widget area to the kernel viewport, settles a frame when
	/// anything is dirty or animating, and returns the current cell grid.
	fn settled_grid(&mut self, width: u16, height: u16) -> &cells::CellGrid {
		let (vw, vh) = slab_tui::terminal_env(width, height);
		kframe::inst_set_env(&mut self.inst, vw, vh, 2, self.dark, self.coarse);
		self.area = (width, height);
		let stale =
			self.grid.is_none() || self.inst.dirty || !self.inst.solved || self.inst.ms.active;
		if stale {
			// Post-solve scroll re-clamp and focus restoration mark the
			// instance dirty for the NEXT frame, so a settled grid needs up
			// to a few passes at the same clock (mirrors slab-tui).
			let mut frame = kframe::inst_frame(&mut self.inst, self.clock_ms);
			for _ in 0..3 {
				if !self.inst.dirty {
					break;
				}
				frame = kframe::inst_frame(&mut self.inst, self.clock_ms);
			}
			let effects = kframe::inst_take_signals(&mut self.inst);
			collect_signals(&self.inst, &effects, &mut self.signals);
			self.grid = Some(cells::cells_with_caret(&self.inst, &frame));
		}
		self.grid.as_ref().expect("grid settled above")
	}
}

/// Renders a [`SlabState`] document into the widget area.
pub struct SlabWidget;

impl StatefulWidget for SlabWidget {
	type State = SlabState;

	fn render(self, area: Rect, buf: &mut Buffer, state: &mut SlabState) {
		if area.width == 0 || area.height == 0 {
			return;
		}
		let grid = state.settled_grid(area.width, area.height);
		let rows = grid.rows.min(i32::from(area.height));
		let cols = grid.cols.min(i32::from(area.width));
		for r in 0..rows {
			for c in 0..cols {
				let ix = (r * grid.cols + c) as usize;
				if grid.ch[ix] == cells::CONT {
					continue;
				}
				let Some(cell) = buf.cell_mut((area.x + c as u16, area.y + r as u16)) else {
					continue;
				};
				if grid.cl[ix].is_empty() {
					cell.set_char(char::from_u32(grid.ch[ix]).unwrap_or(' '));
				} else {
					cell.set_symbol(&grid.cl[ix]);
				}
				cell.set_fg(cell_color(grid.fg[ix], grid.flags[ix] & cells::CF_FG != 0));
				cell.set_bg(cell_color(grid.bg[ix], grid.flags[ix] & cells::CF_BG != 0));
				if grid.flags[ix] & cells::CF_STRIKE != 0 {
					cell.modifier.insert(Modifier::CROSSED_OUT);
				} else {
					cell.modifier.remove(Modifier::CROSSED_OUT);
				}
			}
		}
	}
}

/// Maps a packed `0xRRGGBB` kernel color to ratatui; unset falls back to the
/// terminal default, exactly like the ANSI painter's `NO_COLOR` handling.
const fn cell_color(rgb: u32, set: bool) -> Color {
	if set {
		Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
	} else {
		Color::Reset
	}
}

/// Resolves all four parallel Effects signal channels into driver signals.
fn collect_signals(inst: &kframe::Instance, eff: &dispatch::Effects, out: &mut Vec<Signal>) {
	for k in 0..eff.sig_name.len() {
		out.push(Signal {
			name: slab_kernel::slir::str_at(inst.doc(), eff.sig_name[k]).to_owned(),
			text: eff.sig_text[k].clone(),
			item: eff.sig_item[k].clone(),
			meta: eff.sig_meta[k].clone(),
		});
	}
}

/// Compiles a `.slab` file to encoded SLIR bytes; errors are the joined,
/// formatted compiler diagnostics.
fn compile(file: &Path) -> Result<Vec<u8>, String> {
	let src = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
	let opts = slab_compile::Options {
		embed_assets: true,
		base_dir:     file
			.parent()
			.map_or_else(|| Path::new(".").to_path_buf(), Path::to_path_buf),
		assets:       None,
		sources:      None,
		fonts:        std::collections::HashMap::new(),
	};
	let (slir, diags) = slab_compile::compile(&src, &opts);
	if diags.has_errors() {
		let name = file.display().to_string();
		let text: Vec<String> = diags.0.iter().map(|d| d.format(&name)).collect();
		return Err(text.join("\n"));
	}
	let slir = slir.ok_or("compile failed: no SLIR produced")?;
	Ok(slab_slir::write(&slir))
}
