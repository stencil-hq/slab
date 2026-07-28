//! Detection and reporting for support-chart features used by a solved document.
//!
//! This is the single capability scan shared by native and WebAssembly hosts.
//! It combines immutable SLIR pools with the solved frame operations so every
//! runtime reports the same features in [`crate::caps::FEATURES`] order.

use crate::{
    caps,
    flatten::{Frame, FrameOp},
    slir::{A_CONTENT, Doc, F_SCROLL, F_SCROLL_CROSS, F_STICKY, K_DIVIDER},
};

/// Reports whether a decoded document and solved frame use a support-chart feature.
///
/// `feature` is normally one of [`caps::FEATURES`]. Unknown feature names are
/// treated as unused.
pub fn uses(doc: &Doc, frame: &Frame, feature: &str) -> bool {
    let ops = &frame.ops;
    match feature {
        "radius" => ops.iter().any(|op| match op {
            FrameOp::Rect(rect) => rect.radius > 0.0,
            FrameOp::Image(image) => image.radius > 0.0,
            FrameOp::ClipPush(clip) => clip.radius > 0.0,
            FrameOp::Backdrop(backdrop) => backdrop.radius > 0.0,
            _ => false,
        }),
        "shadow" => ops
            .iter()
            .any(|op| matches!(op, FrameOp::Rect(rect) if rect.shadow_len > 0)),
        "blur" => ops
            .iter()
            .any(|op| matches!(op, FrameOp::GroupPush(group) if group.blur > 0.0)),
        "backdrop" => ops.iter().any(|op| matches!(op, FrameOp::Backdrop(_))),
        "gradient" => ops.iter().any(|op| match op {
            FrameOp::Rect(rect) => rect.bg_kind == 2 || rect.stroke_kind == 2,
            FrameOp::PathDraw(path) => path.bg_kind == 2 || path.stroke_kind == 2,
            _ => false,
        }),
        "path" => ops.iter().any(|op| matches!(op, FrameOp::PathDraw(_))),
        "path-runtime" => ops
            .iter()
            .any(|op| matches!(op, FrameOp::PathDraw(path) if path.path < 0)),
        "icon" => doc.node_kind.contains(&crate::slir::K_ICON),
        "scale" => doc.attr_id.contains(&crate::slir::A_SCALE),
        "tilt" => ops.iter().any(|op| matches!(op, FrameOp::TiltPush(_))),
        "gradient-conic" => ops.iter().any(|op| match op {
            FrameOp::Rect(rect) => {
                conic_paint(doc, rect.bg_kind, rect.bg)
                    || conic_paint(doc, rect.stroke_kind, rect.stroke)
            }
            FrameOp::PathDraw(path) => {
                conic_paint(doc, path.bg_kind, path.bg)
                    || conic_paint(doc, path.stroke_kind, path.stroke)
            }
            FrameOp::Text(text) => conic_paint(doc, text.color_kind, text.color),
            FrameOp::GroupPush(group) => conic_paint(doc, group.mask_kind, group.mask),
            FrameOp::Backdrop(backdrop) => conic_paint(doc, backdrop.mask_kind, backdrop.mask),
            _ => false,
        }),
        "gradient-text" => ops
            .iter()
            .any(|op| matches!(op, FrameOp::Text(text) if text.color_kind == 2)),
        "grain" => ops
            .iter()
            .any(|op| matches!(op, FrameOp::Rect(rect) if rect.grain_amount > 0.0)),
        "mask" => ops
            .iter()
            .any(|op| matches!(op, FrameOp::GroupPush(group) if group.mask_kind != 0)),
        "smooth" => ops.iter().any(|op| match op {
            FrameOp::Rect(rect) => rect.smooth > 0.0 && rect.radius > 0.0,
            FrameOp::Image(image) => image.smooth > 0.0 && image.radius > 0.0,
            FrameOp::ClipPush(clip) => clip.smooth > 0.0 && clip.radius > 0.0,
            FrameOp::Backdrop(backdrop) => backdrop.smooth > 0.0 && backdrop.radius > 0.0,
            _ => false,
        }),
        "backdrop-fade" => ops
            .iter()
            .any(|op| matches!(op, FrameOp::Backdrop(backdrop) if backdrop.mask_kind != 0)),
        "image" => ops.iter().any(|op| matches!(op, FrameOp::Image(_))),
        "img-runtime" => ops.iter().any(
            |op| matches!(op, FrameOp::Image(image) if usize::try_from(image.img).is_ok_and(|index| index >= doc.img_src.len())),
        ),
        "rotation" => ops.iter().any(|op| matches!(op, FrameOp::RotatePush(_))),
        "animation" => !doc.bind_node.is_empty(),
        "text-keyframes" => uses_text_keyframes(doc),
        "transition" => !doc.trans_node.is_empty(),
        "themes" => !doc.theme_name.is_empty(),
        "lists" => !doc.list_param.is_empty(),
        "input" | "signals" => !doc.sign_name.is_empty(),
        "a11y" => frame.scene.iter().any(|node| {
            node.role != 0
                || node.label != 0
                || node.desc != 0
                || node.checked != 0
                || node.expanded != 0
                || node.selected != 0
                || node.active_descendant != 0
                || node.controls != 0
                || node.value_now.is_some()
                || node.value_min.is_some()
                || node.value_max.is_some()
                || node.value_text != 0
                || node.modal != 0
                || node.live != 0
                || node.live_atomic != 0
                || node.level.is_some()
                || node.pos_in_set.is_some()
                || node.set_size.is_some()
        }),
        "ime" | "text-edit" => doc.sign_trigger.contains(&1),
        "scroll" => doc
            .node_flags
            .iter()
            .any(|flags| flags & (F_SCROLL | F_SCROLL_CROSS) != 0),
        "scroll-cross" => doc
            .node_flags
            .iter()
            .any(|flags| flags & F_SCROLL_CROSS != 0),
        "sticky" => doc
            .node_flags
            .iter()
            .any(|flags| flags & F_STICKY != 0),
        "divider" => doc.node_kind.contains(&K_DIVIDER),
        "holes" => !doc.hole_name.is_empty(),
        "text-strike" => ops
            .iter()
            .any(|op| matches!(op, FrameOp::Text(text) if text.strike)),
        "text-raster" => ops.iter().any(|op| matches!(op, FrameOp::Text(_))),
        "glyph-fallback" => ops
            .iter()
            .any(|op| matches!(op, FrameOp::Text(text) if text.uncov_len > 0)),
        _ => false,
    }
}

/// Reports whether a paint `(kind, handle)` pair references a conic gradient.
fn conic_paint(doc: &Doc, kind: u32, handle: u32) -> bool {
    kind == 2 && usize::try_from(handle).is_ok_and(|index| doc.grad_kind.get(index) == Some(&2))
}

/// Builds canonical support-chart lines for used features degraded or omitted by a client.
///
/// `client_index` indexes [`caps::CLIENTS`]. Lines retain the generated chart
/// row order and omit features with full support.
///
/// # Panics
///
/// Panics when `client_index` is not an index in [`caps::CLIENTS`].
pub fn chart_lines(doc: &Doc, frame: &Frame, client_index: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for (feature_index, &feature) in caps::FEATURES.iter().enumerate() {
        let level = caps::LEVELS[feature_index][client_index];
        if level == caps::FULL || !uses(doc, frame, feature) {
            continue;
        }
        let level_name = if level == caps::NONE {
            "none"
        } else {
            "degraded"
        };
        lines.push(format!(
            "chart {level_name} {feature}: {}",
            caps::NOTES[feature_index][client_index]
        ));
    }
    lines
}

fn uses_text_keyframes(doc: &Doc) -> bool {
    for &animation in &doc.bind_anim {
        let animation_index = index_u32(animation);
        let stop_start = index_i32(doc.anim_stop_off[animation_index]);
        let stop_length = index_i32(doc.anim_stop_len[animation_index]);
        for stop in stop_start..stop_start + stop_length {
            let attribute_start = index_i32(doc.anim_stop_attr_off[stop]);
            let attribute_length = index_i32(doc.anim_stop_attr_len[stop]);
            if doc.aattr_id[attribute_start..attribute_start + attribute_length]
                .contains(&A_CONTENT)
            {
                return true;
            }
        }
    }
    false
}

fn index_i32(value: i32) -> usize {
    usize::try_from(value).expect("kernel index must be nonnegative")
}

fn index_u32(value: u32) -> usize {
    usize::try_from(value).expect("kernel index exceeds usize")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{flatten, slir};

    #[test]
    fn detects_document_pool_capability_features_at_empty_and_nonempty_boundaries() {
        let mut doc = slir::doc_new();
        let solved = flatten::frame_new();
        for feature in [
            "animation",
            "transition",
            "input",
            "signals",
            "ime",
            "text-edit",
            "scroll",
            "divider",
            "holes",
            "themes",
            "lists",
        ] {
            assert!(!uses(&doc, &solved, feature), "{feature}");
        }

        doc.bind_node.push(0);
        doc.trans_node.push(0);
        doc.sign_name.push(0);
        doc.sign_trigger.push(1);
        doc.node_flags.push(F_SCROLL);
        doc.node_kind.push(K_DIVIDER);
        doc.hole_name.push(0);
        doc.theme_name.push(0);
        doc.list_param.push(0);
        for feature in [
            "animation",
            "transition",
            "input",
            "signals",
            "ime",
            "text-edit",
            "scroll",
            "divider",
            "holes",
            "themes",
            "lists",
        ] {
            assert!(uses(&doc, &solved, feature), "{feature}");
        }
    }

    #[test]
    fn detects_only_content_attributes_as_text_keyframes() {
        let mut doc = slir::doc_new();
        doc.bind_anim.push(0);
        doc.anim_stop_off.push(0);
        doc.anim_stop_len.push(1);
        doc.anim_stop_attr_off.push(0);
        doc.anim_stop_attr_len.push(1);
        doc.aattr_id.push(A_CONTENT.wrapping_add(1));
        assert!(!uses_text_keyframes(&doc));

        doc.aattr_id[0] = A_CONTENT;
        assert!(uses_text_keyframes(&doc));
    }

    #[test]
    fn capability_feature_table_is_exhaustively_classified() {
        let doc = slir::doc_new();
        let solved = flatten::frame_new();
        for feature in caps::FEATURES {
            assert!(!uses(&doc, &solved, feature), "{feature}");
        }
        assert!(!uses(&doc, &solved, "unknown"));
    }
}
