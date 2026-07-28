//! wasm-bindgen projection of the shared compact frame transport.

use slab_kernel::{frame_buf::FrameBuf as KernelFrameBuf, flatten::Frame};
use wasm_bindgen::prelude::*;

/// Binary frame payload decoded by `clients/web/frame-decode.ts`.
#[wasm_bindgen]
pub struct FrameBuf {
	inner:       KernelFrameBuf,
	rt_paths:    String,
	diagnostics: String,
}

impl FrameBuf {
	pub(crate) fn encode(frame: Frame, dirty: bool, motion_active: bool) -> Self {
		let inner = KernelFrameBuf::encode(frame, dirty, motion_active);
		let rt_paths = serde_json::to_string(
			&inner
				.rt_paths
				.iter()
				.map(|path| (&path.verbs, &path.coords))
				.collect::<Vec<_>>(),
		)
		.expect("runtime paths serialize");

		#[derive(serde::Serialize)]
		struct DiagnosticJson<'a> {
			code: &'a str,
			line: u32,
			msg:  &'a str,
		}
		let diagnostics = serde_json::to_string(
			&inner
				.diagnostics
				.iter()
				.map(|diagnostic| DiagnosticJson {
					code: &diagnostic.code,
					line: diagnostic.line,
					msg:  &diagnostic.msg,
				})
				.collect::<Vec<_>>(),
		)
		.expect("frame diagnostics serialize");

		Self { inner, rt_paths, diagnostics }
	}
}

#[wasm_bindgen]
impl FrameBuf {
	/// Returns operation tags and integer payloads.
	pub fn u32s(&self) -> Vec<u32> {
		self.inner.u32s.clone()
	}

	/// Returns frame dimensions followed by operation float payloads.
	pub fn f64s(&self) -> Vec<f64> {
		self.inner.f64s.clone()
	}

	/// Returns the frame-local string pool as JSON.
	pub fn strs_json(&self) -> String {
		serde_json::to_string(&self.inner.strings).expect("frame strings serialize")
	}

	/// Returns the flat uncovered-glyph run pool.
	pub fn uncovered_u32s(&self) -> Vec<u32> {
		self.inner.uncovered.clone()
	}

	/// Returns frame-local runtime paths as `[verbs, coords]` JSON pairs.
	pub fn rt_paths_json(&self) -> String {
		self.rt_paths.clone()
	}

	/// Returns host-visible frame diagnostics as JSON objects.
	pub fn diagnostics_json(&self) -> String {
		self.diagnostics.clone()
	}

	/// Reports whether the solve dirtied the instance for another frame.
	#[allow(
		clippy::missing_const_for_fn,
		reason = "wasm_bindgen exported methods cannot be const fn"
	)]
	pub fn dirty(&self) -> bool {
		self.inner.dirty
	}

	/// Reports whether animation or transition clocks remain active.
	#[allow(
		clippy::missing_const_for_fn,
		reason = "wasm_bindgen exported methods cannot be const fn"
	)]
	pub fn motion_active(&self) -> bool {
		self.inner.motion_active
	}
}
