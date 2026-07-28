//! WebAssembly boundary for the deterministic slab kernel.
//!
//! Structured cold-path data crosses as JSON. Frame paint operations use the
//! compact typed streams in [`FrameBuf`]. Browser and Node bindings are emitted
//! by `cargo run -p xtask -- kernel-wasm`.

mod conformance;
mod frame_buf;
mod snapshot;

pub use frame_buf::FrameBuf;
use slab_kernel::{cells, dispatch, dumpjson, frame as kframe};
use wasm_bindgen::prelude::*;

/// One decoded, initialized kernel instance owned by JavaScript.
#[wasm_bindgen]
pub struct KInst {
	inner: kframe::Instance,
}

#[wasm_bindgen]
impl KInst {
	/// Decodes SLIR bytes and creates an initialized kernel instance.
	#[wasm_bindgen(constructor)]
	pub fn new(slir: &[u8]) -> Result<Self, JsValue> {
		let (inner, _) = slab_slir::kernel::instance(slir).map_err(js_error)?;
		Ok(Self { inner })
	}

	/// Updates viewport and client environment inputs.
	pub fn set_env(&mut self, vw: f64, vh: f64, client: u32, dark: bool, coarse: bool) {
		kframe::inst_set_env(&mut self.inner, vw, vh, client, dark, coarse);
	}

	/// Assigns one scalar parameter by document parameter index.
	pub fn set_param(
		&mut self,
		param: u32,
		kind: u32,
		num: f64,
		value: &str,
		rgba: u32,
		symbol: &str,
	) -> bool {
		kframe::inst_set_param(&mut self.inner, param, &kframe::ParamValue {
			kind,
			num,
			s: value.to_owned(),
			rgba,
			sym: symbol.to_owned(),
		})
	}

	/// Returns the item count for a root or nested list.
	pub fn list_len(&self, param: u32, path: &str) -> i32 {
		kframe::inst_list_len(&self.inner, param, path)
	}

	/// Changes the item count for a root or nested list.
	pub fn set_list_len(&mut self, param: u32, path: &str, length: i32) -> bool {
		kframe::inst_set_list_len(&mut self.inner, param, path, length)
	}

	/// Assigns one root or nested list item's stable key.
	pub fn set_list_key(&mut self, param: u32, path: &str, index: i32, key: &str) -> bool {
		kframe::inst_set_list_key(&mut self.inner, param, path, index, key)
	}

	/// Assigns one typed list field.
	pub fn set_list_field(
		&mut self,
		param: u32,
		path: &str,
		index: i32,
		field: &str,
		kind: u32,
		num: f64,
		value: &str,
		rgba: u32,
		symbol: &str,
	) -> bool {
		kframe::inst_set_list_field(&mut self.inner, param, path, index, field, &kframe::ParamValue {
			kind,
			num,
			s: value.to_owned(),
			rgba,
			sym: symbol.to_owned(),
		})
	}

	/// Registers runtime font metrics and returns the selected font-table index.
	pub fn font_register(
		&mut self,
		family: &str,
		weight: u32,
		upem: u32,
		ascent: i32,
		descent: i32,
		line_gap: i32,
		default_advance: u32,
		codepoints: &[u32],
		glyphs: &[u32],
		advances: &[u32],
	) -> i32 {
		kframe::inst_font_register(
			&mut self.inner,
			family,
			weight,
			upem,
			ascent,
			descent,
			line_gap,
			default_advance,
			codepoints,
			glyphs,
			advances,
		)
	}

	/// Enables or disables one document-level state by name.
	pub fn set_state(&mut self, name: &str, on: bool) {
		kframe::inst_set_state(&mut self.inner, name, on);
	}

	/// Enables or disables one node-local state, returning false for an unknown
	/// key.
	pub fn set_node_state(&mut self, key: &str, name: &str, on: bool) -> bool {
		kframe::inst_set_node_state(&mut self.inner, key, name, on)
	}

	/// Moves focus to a keyed focusable node; an empty key clears focus.
	pub fn set_focus(&mut self, key: &str, visible: bool) -> bool {
		kframe::inst_set_focus(&mut self.inner, key, visible)
	}

	/// Clears kernel focus and any visible focus ring.
	pub fn clear_focus(&mut self) -> bool {
		kframe::inst_clear_focus(&mut self.inner)
	}

	/// Reveals, materializes, and focuses one virtual-list item.
	pub fn focus_item(&mut self, each_key: &str, item_index: i32) -> bool {
		kframe::inst_focus_item(&mut self.inner, each_key, item_index)
	}

	/// Returns the last failed focus request's actionable explanation.
	pub fn focus_note(&self) -> String {
		kframe::inst_focus_note(&self.inner).to_owned()
	}

	/// Replaces one keyed field edit buffer and queues its Change signal.
	pub fn set_field_text(&mut self, key: &str, text: &str) -> bool {
		kframe::inst_set_field_text(&mut self.inner, key, text)
	}

	/// Returns one keyed field's committed text.
	pub fn field_text(&self, key: &str) -> Option<String> {
		kframe::inst_field_text(&self.inner, key)
	}

	/// Returns the focused node, or `u32::MAX` when focus is clear.
	#[allow(
		clippy::missing_const_for_fn,
		reason = "wasm_bindgen exported methods cannot be const fn"
	)]
	pub fn focus(&self) -> u32 {
		kframe::inst_focus(&self.inner)
	}

	/// Returns one current parameter value as JSON.
	pub fn param_json(&self, name: &str) -> Option<String> {
		kframe::inst_param_json(&self.inner, name)
	}

	/// Selects a compiled theme by name.
	pub fn set_theme(&mut self, name: &str) -> bool {
		kframe::inst_set_theme(&mut self.inner, name)
	}

	/// Returns the current theme name.
	pub fn theme(&self) -> String {
		kframe::inst_theme(&self.inner)
	}

	/// Returns one active-theme token as typed JSON, or `undefined` when absent.
	pub fn get_token_json(&self, path: &str) -> Option<String> {
		match kframe::inst_get_token(&self.inner, path)? {
			kframe::TokenValue::Number(value) => {
				Some(serde_json::to_string(&value).expect("token number serializes"))
			},
			kframe::TokenValue::Color(rgba) => {
				let [r, g, b, a] = rgba.to_le_bytes();
				let css = if a == u8::MAX {
					format!("#{r:02x}{g:02x}{b:02x}")
				} else {
					format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
				};
				Some(serde_json::to_string(&css).expect("token color serializes"))
			},
			kframe::TokenValue::Text(value) => {
				Some(serde_json::to_string(value).expect("token text serializes"))
			},
		}
	}

	/// Changes one retained scroll offset by node key and axis.
	pub fn set_scroll(&mut self, key: &str, axis: u32, offset: f64) -> bool {
		kframe::inst_set_scroll(&mut self.inner, key, axis, offset)
	}

	/// Returns one retained scroll offset by node key and axis.
	pub fn get_scroll(&self, key: &str, axis: u32) -> f64 {
		kframe::inst_get_scroll(&self.inner, key, axis)
	}

	/// Scrolls ancestors minimally to reveal a keyed node.
	pub fn reveal(&mut self, key: &str, margin: f64) -> bool {
		kframe::inst_reveal(&mut self.inner, key, margin)
	}

	/// Reveals one item in a virtual list.
	pub fn reveal_item(&mut self, each: &str, index: i32, align: u32) -> bool {
		kframe::inst_reveal_item(&mut self.inner, each, index, align)
	}

	/// Returns a virtual list's materialized window as JSON.
	pub fn each_window_json(&self, each: &str) -> String {
		serde_json::to_string(&kframe::inst_each_window(&self.inner, each))
			.expect("list window serializes")
	}

	/// Sets one keyed divider extent overlay.
	pub fn set_divider(&mut self, key: &str, extent: f64) -> bool {
		kframe::inst_set_divider(&mut self.inner, key, extent)
	}

	/// Returns one keyed divider extent overlay.
	pub fn get_divider(&self, key: &str) -> f64 {
		kframe::inst_get_divider(&self.inner, key)
	}

	/// Updates measured slot content for one hole.
	pub fn set_hole_size(&mut self, hole: u32, width: f64, height: f64) {
		kframe::inst_set_hole_size(&mut self.inner, hole, width, height);
	}

	/// Returns absolute hole rectangles for the current solve as JSON.
	pub fn holes_json(&mut self) -> String {
		snapshot::holes_json(&kframe::inst_holes(&mut self.inner))
	}

	/// Marks every CSS-liftable animation binding driver-owned and returns
	/// their normalized keyframes as JSON. The caller MUST replay them
	/// (e.g. as CSS animations); lifted bindings no longer drive kernel
	/// motion. Idempotent.
	pub fn lift_animations_json(&mut self) -> String {
		snapshot::lifts_json(&kframe::inst_lift_animations(&mut self.inner))
	}

	/// Solves and lowers one frame into compact typed streams.
	pub fn frame(&mut self, time_ms: f64) -> FrameBuf {
		let frame = kframe::inst_frame(&mut self.inner, time_ms);
		FrameBuf::encode(frame, self.inner.dirty, self.inner.ms.active)
	}

	/// Drains signals queued while solving the preceding frame.
	pub fn take_signals_json(&mut self) -> String {
		snapshot::effects_json(&kframe::inst_take_signals(&mut self.inner))
	}

	/// Returns every distinct diagnostic observed since document assignment as
	/// JSON `{code, line, msg}` objects, in first-occurrence order. Unlike the
	/// per-solve [`FrameBuf::diagnostics_json`] stream, runtime notes here are
	/// never consumed by intermediate solves.
	pub fn diags_json(&self) -> String {
		#[derive(serde::Serialize)]
		struct DiagnosticJson<'a> {
			code: &'a str,
			line: u32,
			msg:  &'a str,
		}
		serde_json::to_string(
			&kframe::inst_diags(&self.inner)
				.iter()
				.map(|diagnostic| DiagnosticJson {
					code: &diagnostic.code,
					line: diagnostic.line,
					msg:  &diagnostic.msg,
				})
				.collect::<Vec<_>>(),
		)
		.expect("cumulative diagnostics serialize")
	}

	/// Drains settled-frame signals in the canonical conformance dump shape.
	pub fn take_signals_dump_json(&mut self) -> String {
		let effects = kframe::inst_take_signals(&mut self.inner);
		dumpjson::dump_effects(&self.inner.doc, &self.inner.st, &effects)
	}

	/// Dispatches one platform event and returns all effects as JSON.
	pub fn dispatch_json(
		&mut self,
		event_type: u32,
		x: f64,
		y: f64,
		dx: f64,
		dy: f64,
		button: u32,
		key: &str,
		text: &str,
		modifiers: u32,
		clicks: u32,
	) -> String {
		let effects = kframe::inst_dispatch(&mut self.inner, &dispatch::Event {
			etype: event_type,
			x,
			y,
			dx,
			dy,
			button,
			clicks,
			key: key.to_owned(),
			text: text.to_owned(),
			mods: modifiers,
		});
		snapshot::effects_json(&effects)
	}

	/// Dispatches one platform event and emits canonical conformance effects
	/// JSON.
	pub fn dispatch_dump_json(
		&mut self,
		event_type: u32,
		x: f64,
		y: f64,
		dx: f64,
		dy: f64,
		button: u32,
		key: &str,
		text: &str,
		modifiers: u32,
		clicks: u32,
	) -> String {
		let effects = kframe::inst_dispatch(&mut self.inner, &dispatch::Event {
			etype: event_type,
			x,
			y,
			dx,
			dy,
			button,
			clicks,
			key: key.to_owned(),
			text: text.to_owned(),
			mods: modifiers,
		});
		dumpjson::dump_effects(&self.inner.doc, &self.inner.st, &effects)
	}

	/// Recomputes caret and IME geometry from the latest solve.
	pub fn caret_effects_json(&self) -> String {
		let mut effects = dispatch::effects_new();
		dispatch::caret_effects(
			&self.inner.doc,
			&self.inner.st,
			&self.inner.lay,
			&self.inner.sc,
			&self.inner.ds,
			&mut effects,
		);
		snapshot::effects_json(&effects)
	}

	/// Returns immutable document pools and host schemas as JSON.
	pub fn statics_json(&self) -> String {
		snapshot::statics_json(&self.inner)
	}

	/// Registers or replaces a named runtime image.
	pub fn img_register(
		&mut self,
		name: &str,
		width: u32,
		height: u32,
		format: u32,
		bytes: &[u8],
	) -> i32 {
		kframe::inst_img_register(&mut self.inner, name, width, height, format, bytes)
	}

	/// Unregisters one named runtime image.
	pub fn img_unregister(&mut self, name: &str) -> bool {
		kframe::inst_img_unregister(&mut self.inner, name)
	}

	/// Returns one embedded or runtime image payload by unified table index.
	pub fn image_data(&self, image: i32) -> Vec<u8> {
		kframe::inst_img_bytes(&self.inner, image).to_vec()
	}

	/// Returns image dimensions, format, and generation as JSON.
	pub fn image_info_json(&self, image: i32) -> String {
		serde_json::to_string(&kframe::inst_img_info(&self.inner, image))
			.expect("image info serializes")
	}

	/// Returns retained scene geometry and resolved keys as JSON.
	pub fn scene_json(&self) -> String {
		snapshot::scene_json(&self.inner)
	}

	/// Tests a point against one retained scene index, including clips and
	/// rotations.
	pub fn hit_contains(&self, scene_index: i32, x: f64, y: f64) -> bool {
		snapshot::hit_contains(&self.inner, scene_index, x, y)
	}

	/// Returns one retained ancestor chain from root to target as JSON.
	pub fn chain_json(&self, scene_index: i32) -> String {
		snapshot::chain_json(&self.inner, scene_index)
	}

	/// Runs retained-scene hit testing and emits its canonical conformance JSON.
	pub fn hit_json(&self, x: f64, y: f64) -> String {
		let nodes = kframe::inst_hit(&self.inner, x, y);
		dumpjson::dump_hit(&self.inner.doc, &self.inner.st, &nodes)
	}

	/// Emits the canonical summary of retained trace state.
	pub fn trace_summary_json(&self) -> String {
		dumpjson::dump_trace_summary(&self.inner.doc, &self.inner.st, &self.inner)
	}

	/// Solves once and emits plain TUI cells for conformance comparison.
	pub fn cells_text(&mut self, time_ms: f64) -> String {
		conformance::cells_text(&mut self.inner, time_ms)
	}

	/// Solves once and emits the TUI attribute plane (`attrs.txt` golden):
	/// per-row runs of explicit fg/bg/strike cell state. Catches SGR-only
	/// regressions the plain cells golden cannot see.
	pub fn cells_attrs(&mut self, time_ms: f64) -> String {
		conformance::cells_attrs(&mut self.inner, time_ms)
	}

	/// Solves once and emits the truecolor ANSI grid a terminal client paints,
	/// caret included — the live counterpart of `slab render --client tui`.
	pub fn cells_ansi(&mut self, time_ms: f64) -> String {
		let frame = kframe::inst_frame(&mut self.inner, time_ms);
		cells::cells_to_text(&cells::cells_with_caret(&self.inner, &frame), false)
	}

	/// Solves once and reports capability degradation for a renderer client.
	///
	/// Client indices follow `slab_kernel::caps::CLIENTS`; an invalid index is
	/// rejected at the WASM boundary.
	pub fn caps_report(&mut self, time_ms: f64, client: u32) -> Result<String, JsValue> {
		conformance::caps_report(&mut self.inner, time_ms, client).map_err(js_error)
	}

	/// Solves once and returns decoded-pool and frame-operation counts as JSON.
	pub fn selftest_counts_json(&mut self, time_ms: f64) -> String {
		conformance::selftest_counts_json(&mut self.inner, time_ms)
	}

	/// Emits canonical `frame.json` for native/WASM conformance checks.
	pub fn frame_json(&mut self, time_ms: f64) -> String {
		let frame = kframe::inst_frame(&mut self.inner, time_ms);
		dumpjson::dump(&self.inner.doc, &self.inner.st, &frame)
	}
}

fn js_error(error: String) -> JsValue {
	JsValue::from_str(&error)
}
