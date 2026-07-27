//! Compact frame transport for the browser painter.

use slab_kernel::flatten::{Frame, FrameOp};
use wasm_bindgen::prelude::*;

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

/// Binary frame payload decoded by `clients/web/frame-decode.ts`.
///
/// The f64 stream starts with frame width and height. Each u32 operation tag
/// then selects fixed u32 and f64 payload arities, avoiding per-frame JSON
/// allocation for paint operations.
#[wasm_bindgen]
pub struct FrameBuf {
    u32s: Vec<u32>,
    f64s: Vec<f64>,
    strings: Vec<String>,
    rt_paths: String,
    dirty: bool,
    motion_active: bool,
}

impl FrameBuf {
    pub(crate) fn encode(frame: Frame, dirty: bool, motion_active: bool) -> Self {
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
                }
                FrameOp::Text(text) => {
                    u32s.extend([
                        TEXT,
                        text.node,
                        signed_word(text.str_ref),
                        signed_word(text.font),
                        text.weight,
                        text.color,
                        text.color_kind,
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
                }
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
                }
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
                }
                FrameOp::ClipPush(clip) => {
                    u32s.push(CLIP_PUSH);
                    f64s.extend([clip.x, clip.y, clip.w, clip.h, clip.radius, clip.smooth]);
                }
                FrameOp::ClipPop => u32s.push(CLIP_POP),
                FrameOp::GroupPush(group) => {
                    u32s.extend([GROUP_PUSH, group.node, group.mask_kind, group.mask]);
                    f64s.extend([
                        group.opacity,
                        group.blur,
                        group.mx,
                        group.my,
                        group.mw,
                        group.mh,
                    ]);
                }
                FrameOp::GroupPop => u32s.push(GROUP_POP),
                FrameOp::RotatePush(rotation) => {
                    u32s.push(ROTATE_PUSH);
                    f64s.extend([rotation.cx, rotation.cy, rotation.deg]);
                }
                FrameOp::RotatePop => u32s.push(ROTATE_POP),
                FrameOp::ScalePush(scale) => {
                    u32s.push(SCALE_PUSH);
                    f64s.extend([scale.cx, scale.cy, scale.sx, scale.sy]);
                }
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
                }
                FrameOp::TiltPush(tilt) => {
                    u32s.push(TILT_PUSH);
                    f64s.extend([tilt.cx, tilt.cy, tilt.rx, tilt.ry, tilt.depth]);
                }
                FrameOp::TiltPop => u32s.push(TILT_POP),
            }
        }

        let rt_paths = serde_json::to_string(
            &frame
                .paths_rt
                .iter()
                .map(|path| (&path.verbs, &path.coords))
                .collect::<Vec<_>>(),
        )
        .expect("runtime paths serialize");

        Self {
            u32s,
            f64s,
            strings: frame.strings,
            dirty,
            motion_active,
            rt_paths,
        }
    }
}

#[wasm_bindgen]
impl FrameBuf {
    /// Returns operation tags and integer payloads.
    pub fn u32s(&self) -> Vec<u32> {
        self.u32s.clone()
    }

    /// Returns frame dimensions followed by operation float payloads.
    pub fn f64s(&self) -> Vec<f64> {
        self.f64s.clone()
    }

    /// Returns the frame-local string pool as JSON.
    pub fn strs_json(&self) -> String {
        serde_json::to_string(&self.strings).expect("frame strings serialize")
    }

    /// Returns frame-local runtime paths as `[verbs, coords]` JSON pairs.
    pub fn rt_paths_json(&self) -> String {
        self.rt_paths.clone()
    }

    /// Reports whether the solve dirtied the instance for another frame.
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Reports whether animation or transition clocks remain active.
    pub fn motion_active(&self) -> bool {
        self.motion_active
    }
}

fn signed_word(value: i32) -> u32 {
    u32::from_ne_bytes(value.to_ne_bytes())
}

#[cfg(test)]
mod tests {
    use super::{
        CLIP_POP, FrameBuf, GROUP_POP, GROUP_PUSH, ROTATE_POP, SCALE_POP, SCALE_PUSH, TEXT,
    };
    use slab_kernel::flatten::{Frame, FrameOp, OpGroup, OpScale, OpText, RtPath};

    #[test]
    fn encodes_dimensions_signed_indices_and_operation_tags() {
        let frame = Frame {
            width: 320.0,
            height: 180.0,
            ops: vec![
                FrameOp::Text(OpText {
                    node: 7,
                    x: 10.0,
                    y_baseline: 20.0,
                    str_ref: -1,
                    measured_w: 30.0,
                    font: -1,
                    size: 16.0,
                    weight: 400,
                    tracking: 0.5,
                    color: 0x1122_3344,
                    opacity: 0.75,
                    color_kind: 1,
                    gx: 0.0,
                    gy: 0.0,
                    gw: 0.0,
                    gh: 0.0,
                }),
                FrameOp::ClipPop,
                FrameOp::GroupPush(OpGroup {
                    node: 9,
                    opacity: 0.5,
                    blur: 2.0,
                    mask_kind: 0,
                    mask: 0,
                    mx: 1.0,
                    my: 2.0,
                    mw: 30.0,
                    mh: 40.0,
                }),
                FrameOp::GroupPop,
                FrameOp::RotatePop,
                FrameOp::ScalePush(OpScale {
                    cx: 4.0,
                    cy: 5.0,
                    sx: 2.0,
                    sy: 3.0,
                }),
                FrameOp::ScalePop,
            ],
            scene: Vec::new(),
            strings: vec!["hello".to_owned()],
            paths_rt: vec![RtPath {
                verbs: vec![0, 1],
                coords: vec![1.0, 2.0, 3.0, 4.0],
            }],
        };

        let encoded = FrameBuf::encode(frame, true, false);
        assert_eq!(
            encoded.u32s,
            [
                TEXT,
                7,
                u32::MAX,
                u32::MAX,
                400,
                0x1122_3344,
                1,
                CLIP_POP,
                GROUP_PUSH,
                9,
                0,
                0,
                GROUP_POP,
                ROTATE_POP,
                SCALE_PUSH,
                SCALE_POP,
            ]
        );
        assert_eq!(
            encoded.f64s,
            [
                320.0, 180.0, 10.0, 20.0, 30.0, 16.0, 0.5, 0.75, 0.0, 0.0, 0.0, 0.0, 0.5, 2.0, 1.0,
                2.0, 30.0, 40.0, 4.0, 5.0, 2.0, 3.0,
            ]
        );
        assert_eq!(encoded.strings, [String::from("hello")]);
        assert_eq!(encoded.rt_paths, "[[[0,1],[1.0,2.0,3.0,4.0]]]");
        assert!(encoded.dirty);
        assert!(!encoded.motion_active);
    }
}
