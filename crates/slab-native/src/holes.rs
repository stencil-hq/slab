//! Hole content: the host fills a `hole` viewport with `FrameOps`.
//!
//! `InstanceHole` mounts a child kernel `Instance` (another SLIR document)
//! and implements preferred-size measurement and viewport rendering.

use slab_kernel::{
	dispatch::{Effects, Event},
	flatten::Frame,
	frame::{self as kframe, Instance},
};

/// `FrameOp` provider for one hole. The renderer composites the returned
/// frame translated into the hole rect and clipped to it; `instance()`
/// exposes the backing kernel instance for `text_glyphs` and doc resources
/// (fonts, gradients, paths).
pub trait HoleContent {
	/// Hole viewport size changed (logical units) or env flags flipped.
	fn resize(&mut self, w: f64, h: f64, dark: bool, coarse: bool);
	/// Preferred content size measured without viewport bounds.
	fn natural(&mut self) -> (f64, f64);
	/// Produce the frame for clock `t_ms`.
	fn frame(&mut self, t_ms: f64) -> Frame;
	/// The backing instance (glyph positions, FONT/GRAD/PATH resources).
	fn instance(&self) -> &Instance;
	/// Forward an event translated into hole-local coordinates.
	fn dispatch(&mut self, ev: &Event) -> Effects;
	/// True while another frame is needed (dirty or animating).
	fn needs_frame(&self) -> bool;
}

/// A child kernel instance mounted into a hole.
pub struct InstanceHole {
	pub inst: Instance,
	pub imgs: Vec<Vec<u8>>,
	env:      (f64, f64, bool, bool),
	natural:  Option<(f64, f64)>,
}

impl InstanceHole {
	/// `None` when the SLIR bytes do not decode.
	pub fn new(slir: &[u8]) -> Option<Self> {
		let (inst, imgs) = slab_slir::instance(slir).ok()?;
		Some(Self { inst, imgs, env: (0.0, 0.0, false, false), natural: None })
	}
}

impl HoleContent for InstanceHole {
	fn resize(&mut self, w: f64, h: f64, dark: bool, coarse: bool) {
		let env = (w, h, dark, coarse);
		if env != self.env {
			self.env = env;
			kframe::inst_set_env(&mut self.inst, w, h, 1, dark, coarse);
		}
	}

	fn natural(&mut self) -> (f64, f64) {
		if let Some(size) = self.natural
			&& !self.inst.dirty
		{
			return size;
		}

		let env = self.env;
		let t_ms = self.inst.last_t;
		kframe::inst_set_env(&mut self.inst, slab_kernel::layout::INF, 0.0, 1, env.2, env.3);
		let preferred = kframe::inst_frame(&mut self.inst, t_ms);
		let size = (preferred.width, preferred.height);

		// Restore the resolved viewport scene so measurement is invisible to
		// rendering and event routing. A later resize remains a cheap no-op.
		kframe::inst_set_env(&mut self.inst, env.0, env.1, 1, env.2, env.3);
		let _ = kframe::inst_frame(&mut self.inst, t_ms);
		self.natural = Some(size);
		size
	}

	fn frame(&mut self, t_ms: f64) -> Frame {
		kframe::inst_frame(&mut self.inst, t_ms)
	}

	fn instance(&self) -> &Instance {
		&self.inst
	}

	fn dispatch(&mut self, ev: &Event) -> Effects {
		kframe::inst_dispatch(&mut self.inst, ev)
	}

	fn needs_frame(&self) -> bool {
		!self.inst.solved || self.inst.dirty || self.inst.ms.active
	}
}
