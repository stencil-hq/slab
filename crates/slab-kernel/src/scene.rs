//! Retained scene geometry from the most recently flattened frame.
//!
//! An instance owns this structure so hit testing, focus traversal, and scroll
//! clamping use the exact geometry painted by the host. Entries remain in
//! painter-order pre-order; `authored_order` preserves semantic traversal when
//! paint promotion moves sticky children after normal siblings.

use crate::{
    flatten::Frame,
    list::{self, State},
    slir::{Doc, F_FOCUSABLE, F_INERT, NONE},
};

/// A structure-of-arrays snapshot of the most recently flattened scene.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    /// Document or synthetic node identifiers in scene order.
    pub node: Vec<u32>,
    /// Scene indices in materialized authored pre-order, independent of paint order.
    pub authored_order: Vec<usize>,
    /// Scene index of each entry's parent, or `-1` for the root.
    pub parent: Vec<i32>,
    /// Node kinds corresponding to [`Self::node`].
    pub kind: Vec<u32>,
    /// Absolute left edges.
    pub x: Vec<f64>,
    /// Absolute top edges.
    pub y: Vec<f64>,
    /// Solved widths.
    pub w: Vec<f64>,
    /// Solved heights.
    pub h: Vec<f64>,
    /// Solved corner radii.
    pub radius: Vec<f64>,
    /// Paint rotations in degrees, with zero meaning no rotation.
    pub rot: Vec<f64>,
    /// Absolute rotation-center x coordinates in the parent rotation frame.
    pub cx: Vec<f64>,
    /// Absolute rotation-center y coordinates in the parent rotation frame.
    pub cy: Vec<f64>,
    /// Effective flags; `F_CLIP` means the entry clips in this frame.
    pub flags: Vec<u32>,
    /// Main-axis child extents, including trailing padding, for scroll clamping.
    pub content_main: Vec<f64>,
    /// Current main-axis scroll offsets.
    pub scroll_off: Vec<f64>,
    /// Cross-axis child extents, including trailing padding.
    pub content_cross: Vec<f64>,
    /// Current cross-axis scroll offsets.
    pub scroll_cross: Vec<f64>,
    /// Whether each entry's main axis is horizontal.
    pub is_row: Vec<bool>,
    /// Resolved accessibility role string references.
    pub role: Vec<u32>,
    /// Resolved accessibility label string references.
    pub label: Vec<u32>,
    /// Resolved accessibility description string references.
    pub desc: Vec<u32>,
    /// Optional checked-state codes; 0 absent, 1 false, 2 true, 3 mixed.
    pub checked: Vec<u32>,
    /// Optional expanded-state codes; 0 absent, 1 false, 2 true.
    pub expanded: Vec<u32>,
    /// Optional selected-state codes; 0 absent, 1 false, 2 true.
    pub selected: Vec<u32>,
    /// Active-descendant full-key references into the instance scene string pool.
    pub active_descendant: Vec<u32>,
    /// Controlled-node full-key references into the instance scene string pool.
    pub controls: Vec<u32>,
    /// Optional current range values.
    pub value_now: Vec<Option<f64>>,
    /// Optional minimum range values.
    pub value_min: Vec<Option<f64>>,
    /// Optional maximum range values.
    pub value_max: Vec<Option<f64>>,
    /// Human-readable value references into the instance scene string pool.
    pub value_text: Vec<u32>,
    /// Optional modal-state codes; 0 absent, 1 false, 2 true.
    pub modal: Vec<u32>,
    /// Optional live-region codes; 0 absent, 1 off, 2 polite, 3 assertive.
    pub live: Vec<u32>,
    /// Optional live-region atomicity codes; 0 absent, 1 false, 2 true.
    pub live_atomic: Vec<u32>,
    /// Optional semantic hierarchy levels.
    pub level: Vec<Option<f64>>,
    /// Optional one-based positions within semantic sets.
    pub pos_in_set: Vec<Option<f64>>,
    /// Optional semantic set sizes, where -1 means unknown.
    pub set_size: Vec<Option<f64>>,
    /// Whether each node is currently disabled.
    pub disabled: Vec<bool>,
    /// Whether each node currently owns kernel focus.
    pub focused: Vec<bool>,
}

/// Creates an empty retained scene.
pub fn scene_new() -> Scene {
    Scene::default()
}

/// Returns the number of entries in the retained scene.
pub fn count(sc: &Scene) -> i32 {
    i32::try_from(sc.node.len()).expect("scene contains more than i32::MAX entries")
}

/// Refreshes the retained scene from a freshly flattened frame.
pub fn load(sc: &mut Scene, fr: &Frame) {
    sc.node.clear();
    sc.authored_order.clear();
    sc.parent.clear();
    sc.kind.clear();
    sc.x.clear();
    sc.y.clear();
    sc.w.clear();
    sc.h.clear();
    sc.radius.clear();
    sc.rot.clear();
    sc.cx.clear();
    sc.cy.clear();
    sc.flags.clear();
    sc.content_main.clear();
    sc.scroll_off.clear();
    sc.is_row.clear();
    sc.content_cross.clear();
    sc.scroll_cross.clear();
    sc.role.clear();
    sc.label.clear();
    sc.desc.clear();
    sc.checked.clear();
    sc.expanded.clear();
    sc.selected.clear();
    sc.active_descendant.clear();
    sc.controls.clear();
    sc.value_now.clear();
    sc.value_min.clear();
    sc.value_max.clear();
    sc.value_text.clear();
    sc.modal.clear();
    sc.live.clear();
    sc.live_atomic.clear();
    sc.level.clear();
    sc.pos_in_set.clear();
    sc.set_size.clear();
    sc.disabled.clear();
    sc.focused.clear();

    for entry in &fr.scene {
        sc.node.push(entry.node);
        sc.parent.push(entry.parent_ix);
        sc.kind.push(entry.kind);
        sc.x.push(entry.x);
        sc.y.push(entry.y);
        sc.w.push(entry.w);
        sc.h.push(entry.h);
        sc.radius.push(entry.radius);
        sc.rot.push(entry.rot_deg);
        sc.cx.push(entry.rot_cx);
        sc.cy.push(entry.rot_cy);
        sc.flags.push(entry.flags);
        sc.content_main.push(entry.content_main);
        sc.scroll_off.push(entry.scroll_off);
        sc.content_cross.push(entry.content_cross);
        sc.scroll_cross.push(entry.scroll_cross);
        sc.is_row.push(entry.is_row);
        sc.role.push(entry.role);
        sc.label.push(entry.label);
        sc.desc.push(entry.desc);
        sc.checked.push(entry.checked);
        sc.expanded.push(entry.expanded);
        sc.selected.push(entry.selected);
        sc.active_descendant.push(entry.active_descendant);
        sc.controls.push(entry.controls);
        sc.value_now.push(entry.value_now);
        sc.value_min.push(entry.value_min);
        sc.value_max.push(entry.value_max);
        sc.value_text.push(entry.value_text);
        sc.modal.push(entry.modal);
        sc.live.push(entry.live);
        sc.live_atomic.push(entry.live_atomic);
        sc.level.push(entry.level);
        sc.pos_in_set.push(entry.pos_in_set);
        sc.set_size.push(entry.set_size);
        sc.disabled.push(entry.disabled);
        sc.focused.push(entry.focused);
    }

    sc.authored_order.extend(0..fr.scene.len());
    sc.authored_order
        .sort_unstable_by_key(|&index| (fr.scene[index].authored_order, index));
}

/// Returns the scene index of `node`.
///
/// Returns `-1` when the node is absent from this frame, including a detached
/// patch child whose condition is off or an unknown node identifier.
pub fn index_of(sc: &Scene, node: u32) -> i32 {
    sc.node
        .iter()
        .position(|candidate| *candidate == node)
        .map_or(-1, |index| {
            i32::try_from(index).expect("scene index exceeds i32::MAX")
        })
}

/// Writes the chain from the root through `ix` as scene indices.
pub fn chain(sc: &Scene, ix: i32, out: &mut Vec<i32>) {
    out.clear();
    let mut current = ix;
    while current >= 0 {
        out.push(current);
        current = sc.parent[usize::try_from(current).expect("nonnegative scene index is invalid")];
    }

    // Parent links are followed leaf-to-root; callers consume root-to-leaf order.
    out.reverse();
}

/// Writes focusable node identifiers in materialized authored order.
///
/// An entry is included when its focusable flag is set, its effective inert
/// flag is unset, and its resolved disabled state is false. Nodes absent from
/// the retained scene, including unmaterialized virtual items, are excluded.
pub fn focusables(sc: &Scene, out: &mut Vec<u32>) {
    out.clear();
    let is_focusable = |index: usize| {
        let flags = sc.flags[index];
        flags & F_FOCUSABLE != 0
            && flags & F_INERT == 0
            && !sc.disabled.get(index).copied().unwrap_or(false)
    };
    if sc.authored_order.len() == sc.node.len()
        && sc.authored_order.iter().all(|&index| index < sc.node.len())
    {
        for &index in &sc.authored_order {
            if is_focusable(index) {
                out.push(sc.node[index]);
            }
        }
    } else {
        for index in 0..sc.node.len() {
            if is_focusable(index) {
                out.push(sc.node[index]);
            }
        }
    }
}

/// Returns the node identifier for a full key path such as `"col@0/a/box@0"`.
///
/// Returns [`NONE`] when the key is absent. The first match in node-identifier
/// order wins; keys are unique except where duplicate-key diagnostics apply.
/// A bare query without `/` (an authored `#id`, with or without the leading
/// `#`) also resolves against the final segment of hierarchical keys, so
/// hosts can address `#diff-scroll` as `"diff-scroll"` without knowing the
/// instantiation path.
pub fn node_by_key(d: &Doc, lists: &State, key: &str) -> u32 {
    if !key.is_empty() && list::key_index_ready(lists) {
        if let Some(node) = list::key_index_get(lists, key) {
            return node;
        }
        for &node in &lists.sy_id {
            if list::item_ix(lists, d, node) >= 0 && key_eq(d, lists, node, key) {
                return node;
            }
        }
        if !key.contains('/') {
            let want = key.strip_prefix('#').unwrap_or(key);
            if let Some(node) = list::key_leaf_get(lists, want) {
                return node;
            }
        }
        return NONE;
    }
    node_by_key_scan(d, lists, key)
}

/// Reports whether a synthetic node's hierarchical key equals `key` without
/// materializing it. Mirrors the `prefix~item/relative` shape built by
/// [`key_of`].
fn key_eq(d: &Doc, lists: &State, node: u32, key: &str) -> bool {
    let each = list::each_of(lists, d, node);
    if each == NONE {
        let Some(&key_ref) = d
            .node_key
            .get(usize::try_from(node).expect("node id fits usize"))
        else {
            return false;
        };
        let key_index = usize::try_from(key_ref).expect("string reference does not fit usize");
        return d.strs[key_index] == key;
    }
    let base = list::base(lists, d, node);
    let relative_ref = d.node_key[usize::try_from(base).expect("base node id fits usize")];
    let relative = &d.strs[usize::try_from(relative_ref).expect("string reference fits usize")];
    let Some(prefix_item) = key.strip_suffix(relative.as_str()) else {
        return false;
    };
    let Some(prefix_item) = prefix_item.strip_suffix('/') else {
        return false;
    };
    let Some(each_key) = prefix_item.strip_suffix(list::item_key_ref(lists, d, node)) else {
        return false;
    };
    let Some(each_key) = each_key.strip_suffix('~') else {
        return false;
    };
    key_eq(d, lists, each, each_key)
}

/// Linear key resolution for states built without [`list::init`]'s index.
fn node_by_key_scan(d: &Doc, lists: &State, key: &str) -> u32 {
    for (node, &key_ref) in d.node_key.iter().enumerate() {
        let key_index = usize::try_from(key_ref).expect("string reference does not fit usize");
        if d.strs[key_index] == key {
            return u32::try_from(node).expect("document has more than u32::MAX nodes");
        }
    }

    for &node in &lists.sy_id {
        if list::item_ix(lists, d, node) >= 0 && key_of(d, lists, node) == key {
            return node;
        }
    }

    if !key.is_empty() && !key.contains('/') {
        let want = key.strip_prefix('#').unwrap_or(key);
        for (node, &key_ref) in d.node_key.iter().enumerate() {
            let key_index = usize::try_from(key_ref).expect("string reference does not fit usize");
            let full = d.strs[key_index].as_str();
            let leaf = full.rsplit('/').next().unwrap_or(full);
            if leaf.strip_prefix('#').unwrap_or(leaf) == want {
                return u32::try_from(node).expect("document has more than u32::MAX nodes");
            }
        }
    }

    NONE
}

/// Returns the key string for `node`, or an empty string for [`NONE`].
pub fn key_of(d: &Doc, lists: &State, node: u32) -> String {
    if node == NONE {
        return String::new();
    }

    let each = list::each_of(lists, d, node);
    if each == NONE {
        let node_index = usize::try_from(node).expect("node identifier does not fit usize");
        let Some(&key_ref) = d.node_key.get(node_index) else {
            return String::new();
        };
        let key_index = usize::try_from(key_ref).expect("string reference does not fit usize");
        return d.strs[key_index].clone();
    }

    // Detached template keys are relative to their EACH node.
    let base = list::base(lists, d, node);
    let base_index = usize::try_from(base).expect("base node identifier does not fit usize");
    let relative_ref = d.node_key[base_index];
    let relative_index =
        usize::try_from(relative_ref).expect("relative key string reference does not fit usize");

    let prefix = key_of(d, lists, each);
    let relative = &d.strs[relative_index];
    let item = list::item_key(lists, d, node);
    let mut key = String::with_capacity(prefix.len() + item.len() + relative.len() + 2);
    key.push_str(&prefix);
    key.push('~');
    key.push_str(&item);
    key.push('/');
    key.push_str(relative);
    key
}
