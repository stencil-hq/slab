//! Compact, allocation-conscious frame streams shared by binary host bridges.

use crate::flatten::{Frame, FrameDiagnostic, FrameOp, RtPath};

const RECT: u32 = 0;
const TEXT: u32 = 1;
const IMAGE: u32 = 2;
const PATH: u32 = 3;
const CLIP_PUSH: u32 = 4;
const CLIP_POP: u32 = 5;
const GROUP_PUSH: u32 = 6;
const GROUP_POP: u32 = 7;
const ROTATE_PUSH: u32 = 8;
const ROTATE_POP: u32 = 9;
const BACKDROP: u32 = 10;
const SCALE_PUSH: u32 = 11;
const SCALE_POP: u32 = 12;
const TILT_PUSH: u32 = 13;
const TILT_POP: u32 = 14;

/// Fixed-arity integer and float operation streams plus frame-local pools.
///
/// The float stream starts with frame width and height. Each integer operation
/// tag selects the payload arities documented in `spec/FRAME.md`, so hot-path
/// hosts avoid JSON objects and repeated field names.
pub struct FrameBuf {
	/// Operation tags and integer payload words.
	pub u32s:          Vec<u32>,
	/// Frame dimensions followed by operation float payloads.
	pub f64s:          Vec<f64>,
	/// Frame-local text strings addressed by Text operations.
	pub strings:       Vec<String>,
	/// Flat uncovered-glyph codepoint ranges.
	pub uncovered:     Vec<u32>,
	/// Frame-local paths addressed by negative Path operation indices.
	pub rt_paths:      Vec<RtPath>,
	/// Host-visible diagnostics emitted by this solve.
	pub diagnostics:   Vec<FrameDiagnostic>,
	/// Whether retained state requires another settling frame.
	pub dirty:         bool,
	/// Whether advancing the motion clock can change the next frame.
	pub motion_active: bool,
}

impl FrameBuf {
	/// Encodes and consumes a frame, moving its local pools without copying.
	pub fn encode(frame: Frame, dirty: bool, motion_active: bool) -> Self {
		let mut encoded = Self::encode_ops(&frame, dirty, motion_active);
		encoded.strings = frame.strings;
		encoded.uncovered = frame.uncovered;
		encoded.rt_paths = frame.paths_rt;
		encoded.diagnostics = frame.diagnostics;
		encoded
	}

	/// Encodes a retained frame while preserving its allocation-backed pools.
	pub fn encode_ref(frame: &Frame, dirty: bool, motion_active: bool) -> Self {
		let mut encoded = Self::encode_ops(frame, dirty, motion_active);
		encoded.strings.clone_from(&frame.strings);
		encoded.uncovered.clone_from(&frame.uncovered);
		encoded.rt_paths.clone_from(&frame.paths_rt);
		encoded.diagnostics.clone_from(&frame.diagnostics);
		encoded
	}

	fn encode_ops(frame: &Frame, dirty: bool, motion_active: bool) -> Self {
		let mut u32s = Vec::with_capacity(frame.ops.len() * 6);
		let mut f64s = Vec::with_capacity(2 + frame.ops.len() * 6);
		f64s.extend([frame.width, frame.height]);

		for op in &frame.ops {
			match op {
				FrameOp::Rect(rect) => {
					u32s.extend([
						RECT,
						rect.node,
						rect.bg_kind,
						rect.bg,
						rect.stroke_kind,
						rect.stroke,
						rect.stroke_align,
						rect.stroke_sides,
						u32::from(rect.has_dash),
						signed_word(rect.shadow_off),
						signed_word(rect.shadow_len),
					]);
					f64s.extend([
						rect.x,
						rect.y,
						rect.w,
						rect.h,
						rect.radius,
						rect.stroke_w,
						rect.dash_on,
						rect.dash_off,
						rect.opacity,
						rect.smooth,
						rect.grain_amount,
						rect.grain_size,
					]);
				},
				FrameOp::Text(text) => {
					u32s.extend([
						TEXT,
						text.node,
						signed_word(text.str_ref),
						signed_word(text.font),
						text.weight,
						text.color,
						text.color_kind,
						u32::from(text.strike),
						signed_word(text.uncov_off),
						signed_word(text.uncov_len),
					]);
					f64s.extend([
						text.x,
						text.y_baseline,
						text.measured_w,
						text.size,
						text.tracking,
						text.opacity,
						text.gx,
						text.gy,
						text.gw,
						text.gh,
					]);
				},
				FrameOp::Image(image) => {
					u32s.extend([IMAGE, image.node, signed_word(image.img), image.fit]);
					f64s.extend([
						image.x,
						image.y,
						image.w,
						image.h,
						image.radius,
						image.opacity,
						image.smooth,
					]);
				},
				FrameOp::PathDraw(path) => {
					u32s.extend([
						PATH,
						path.node,
						signed_word(path.path),
						path.bg_kind,
						path.bg,
						path.stroke_kind,
						path.stroke,
						u32::from(path.has_dash),
					]);
					f64s.extend([
						path.dx,
						path.dy,
						path.stroke_w,
						path.dash_on,
						path.dash_off,
						path.opacity,
					]);
				},
				FrameOp::ClipPush(clip) => {
					u32s.push(CLIP_PUSH);
					f64s.extend([clip.x, clip.y, clip.w, clip.h, clip.radius, clip.smooth]);
				},
				FrameOp::ClipPop => u32s.push(CLIP_POP),
				FrameOp::GroupPush(group) => {
					u32s.extend([GROUP_PUSH, group.node, group.mask_kind, group.mask]);
					f64s.extend([group.opacity, group.blur, group.mx, group.my, group.mw, group.mh]);
				},
				FrameOp::GroupPop => u32s.push(GROUP_POP),
				FrameOp::RotatePush(rotation) => {
					u32s.push(ROTATE_PUSH);
					f64s.extend([rotation.cx, rotation.cy, rotation.deg]);
				},
				FrameOp::RotatePop => u32s.push(ROTATE_POP),
				FrameOp::ScalePush(scale) => {
					u32s.push(SCALE_PUSH);
					f64s.extend([scale.cx, scale.cy, scale.sx, scale.sy]);
				},
				FrameOp::ScalePop => u32s.push(SCALE_POP),
				FrameOp::Backdrop(backdrop) => {
					u32s.extend([BACKDROP, backdrop.mask_kind, backdrop.mask]);
					f64s.extend([
						backdrop.x,
						backdrop.y,
						backdrop.w,
						backdrop.h,
						backdrop.radius,
						backdrop.blur,
						backdrop.saturate,
						backdrop.brightness,
						backdrop.smooth,
					]);
				},
				FrameOp::TiltPush(tilt) => {
					u32s.push(TILT_PUSH);
					f64s.extend([tilt.cx, tilt.cy, tilt.rx, tilt.ry, tilt.depth]);
				},
				FrameOp::TiltPop => u32s.push(TILT_POP),
			}
		}

		Self {
			u32s,
			f64s,
			strings: Vec::new(),
			uncovered: Vec::new(),
			rt_paths: Vec::new(),
			diagnostics: Vec::new(),
			dirty,
			motion_active,
		}
	}
}

const fn signed_word(value: i32) -> u32 {
	u32::from_ne_bytes(value.to_ne_bytes())
}
