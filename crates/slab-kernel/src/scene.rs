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
    style::Semantics,
};
use rustc_hash::FxHashMap;

/// A snapshot of the most recently flattened scene.
///
/// Geometry and interaction fields stay as parallel columns because hit
/// testing and focus scans read a few of them across every entry; the
/// accessibility block lives in per-entry [`Semantics`] rows because
/// exporters read all of it for one entry at a time.
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
    /// Accessibility semantics rows, read row-wise by exporters.
    pub sem: Vec<Semantics>,
    /// Whether each node is currently disabled.
    pub disabled: Vec<bool>,
    /// Whether each node currently owns kernel focus.
    pub focused: Vec<bool>,
    /// Whether each node is a text leaf with an active `field=` binder.
    pub editable: Vec<bool>,
    /// Node-id → scene-index map rebuilt by [`load`]; empty for hand-built
    /// scenes, which fall back to a linear scan in [`index_of`]. Synthetic
    /// list ids are monotonic, so a map bounds memory to live entries where
    /// a dense vector would grow to the id high-water mark.
    index: FxHashMap<u32, i32>,
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
    sc.sem.clear();
    sc.disabled.clear();
    sc.focused.clear();
    sc.editable.clear();

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
        sc.sem.push(entry.sem);
        sc.disabled.push(entry.disabled);
        sc.focused.push(entry.focused);
        sc.editable.push(entry.editable);
    }

    sc.authored_order.extend(0..fr.scene.len());
    sc.authored_order
        .sort_unstable_by_key(|&index| (fr.scene[index].authored_order, index));

    sc.index.clear();
    sc.index.reserve(sc.node.len());
    for (scene_index, &node) in sc.node.iter().enumerate() {
        // First occurrence wins, matching the previous linear-scan semantics.
        sc.index
            .entry(node)
            .or_insert_with(|| i32::try_from(scene_index).expect("scene index exceeds i32"));
    }
}

/// Returns the scene index of `node`.
///
/// Returns `-1` when the node is absent from this frame, including a detached
/// patch child whose condition is off or an unknown node identifier.
pub fn index_of(sc: &Scene, node: u32) -> i32 {
    if sc.index.is_empty() {
        // Hand-built semantic scenes never pass through `load`; scan directly.
        return sc
            .node
            .iter()
            .position(|candidate| *candidate == node)
            .map_or(-1, |index| {
                i32::try_from(index).expect("scene index exceeds i32::MAX")
            });
    }
    sc.index.get(&node).copied().unwrap_or(-1)
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

fn focusable_index(sc: &Scene, index: usize) -> bool {
    let flags = sc.flags[index];
    flags & F_FOCUSABLE != 0
        && flags & F_INERT == 0
        && !sc.disabled.get(index).copied().unwrap_or(false)
        && focus_painted(sc, index)
}

/// Reports whether `node` is an eligible current keyboard focus target.
pub fn is_focusable(sc: &Scene, node: u32) -> bool {
    let index = index_of(sc, node);
    index >= 0 && focusable_index(sc, usize::try_from(index).expect("negative scene index"))
}

/// Writes focusable node identifiers in materialized authored order.
///
/// An entry is included when its focusable flag is set, its effective inert
/// flag is unset, its resolved disabled state is false, and it has nonempty
/// painted geometry. Non-scroll clipping ancestors also exclude descendants
/// wholly outside their clip. Scroll clips deliberately do not: off-screen
/// children remain keyboard targets so traversal can reveal them.
pub fn focusables(sc: &Scene, out: &mut Vec<u32>) {
    out.clear();
    let is_focusable = |index: usize| focusable_index(sc, index);
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

/// Reports whether an entry has nonempty painted geometry after non-scroll
/// ancestor clips. Scroll viewports are ignored so off-screen content can
/// remain in the focus ring and be revealed by keyboard traversal.
pub fn focus_painted(sc: &Scene, index: usize) -> bool {
    let Some((x, y, w, h)) =
        sc.x.get(index)
            .zip(sc.y.get(index))
            .zip(sc.w.get(index).zip(sc.h.get(index)))
            .map(|((&x, &y), (&w, &h))| (x, y, w, h))
    else {
        // Hand-built semantic scenes may omit geometry; loaded frames never do.
        return true;
    };
    let (mut left, mut top, mut right, mut bottom) = (x, y, x + w, y + h);
    if !left.is_finite()
        || !top.is_finite()
        || !right.is_finite()
        || !bottom.is_finite()
        || right <= left
        || bottom <= top
    {
        return false;
    }

    let mut parent = sc.parent.get(index).copied().unwrap_or(-1);
    while parent >= 0 {
        let parent_index = usize::try_from(parent).expect("negative scene index");
        let flags = sc.flags[parent_index];
        if flags & crate::slir::F_CLIP != 0
            && flags & (crate::slir::F_SCROLL | crate::slir::F_SCROLL_CROSS) == 0
        {
            left = left.max(sc.x[parent_index]);
            top = top.max(sc.y[parent_index]);
            right = right.min(sc.x[parent_index] + sc.w[parent_index]);
            bottom = bottom.min(sc.y[parent_index] + sc.h[parent_index]);
            if right <= left || bottom <= top {
                return false;
            }
        }
        parent = sc.parent[parent_index];
    }
    true
}

/// Result of resolving a canonical scene-key locator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyResolution {
    /// Exactly one node resolved.
    Found(u32),
    /// No node resolved; the keys are deterministic near matches or examples.
    Missing { candidates: Vec<String> },
    /// A shorthand locator matched more than one canonical full key.
    Ambiguous { candidates: Vec<String> },
}

fn detached_each_template(d: &Doc, node: u32) -> bool {
    let mut parent = d
        .node_parent
        .get(usize::try_from(node).unwrap_or(usize::MAX))
        .copied()
        .unwrap_or(NONE);
    while parent != NONE {
        let index = usize::try_from(parent).expect("node id exceeds usize");
        if d.node_kind.get(index) == Some(&crate::slir::K_EACH) {
            return true;
        }
        parent = d.node_parent.get(index).copied().unwrap_or(NONE);
    }
    false
}

fn candidate_nodes(d: &Doc, lists: &State) -> Vec<u32> {
    let mut nodes = Vec::with_capacity(d.node_key.len() + lists.sy_id.len());
    nodes.extend((0..d.node_key.len()).filter_map(|node| {
        let node = u32::try_from(node).expect("document has more than u32::MAX nodes");
        (!detached_each_template(d, node)).then_some(node)
    }));
    nodes.extend(
        lists
            .sy_id
            .iter()
            .copied()
            .filter(|&node| list::item_ix(lists, d, node) >= 0),
    );
    nodes
}

fn unique_keys(d: &Doc, lists: &State, nodes: &[u32]) -> Vec<String> {
    let mut keys = Vec::new();
    for &node in nodes {
        let key = key_of(d, lists, node);
        if !key.is_empty() && !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys.sort();
    keys
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, &right_char) in right.iter().enumerate() {
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + if left_char == right_char { 0 } else { 1 });
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

/// Scores one candidate key against an unresolved query.
///
/// The primary score is the best edit distance between any candidate path
/// segment (authored `#` stripped) and the query's final segment, so an id
/// typo such as `#fal` ranks every key routed through `#fall` first, and a
/// query missing one interior segment still ranks keys whose last segment
/// matches exactly at the top. The secondary score is the whole-key edit
/// distance, which prefers the shortest containing key among segment ties.
fn key_distance(candidate: &str, query: &str) -> (usize, usize) {
    let wanted = query.rsplit('/').next().unwrap_or(query);
    let wanted = wanted.strip_prefix('#').unwrap_or(wanted);
    let segment = candidate
        .split('/')
        .map(|segment| edit_distance(segment.strip_prefix('#').unwrap_or(segment), wanted))
        .min()
        .unwrap_or(usize::MAX);
    (segment, edit_distance(candidate, query))
}

/// Resolves an exact canonical key or an author-friendly unique locator.
///
/// Exact full keys win. A bare `#id`/`id` resolves the node carrying that
/// authored id (including a component call id placed on its first root), then
/// falls back to a unique final segment. A path beginning with `#id`, such as
/// `#feed/rows`, resolves a unique authored suffix. Ambiguous shorthands are
/// rejected with their canonical full-key candidates.
pub fn resolve_key(d: &Doc, lists: &State, key: &str) -> KeyResolution {
    if key.is_empty() {
        return KeyResolution::Missing {
            candidates: Vec::new(),
        };
    }
    if list::key_index_ready(lists) {
        if let Some(node) = list::key_index_get(lists, key)
            && !detached_each_template(d, node)
        {
            return KeyResolution::Found(node);
        }
    } else {
        for (node, &key_ref) in d.node_key.iter().enumerate() {
            let node = u32::try_from(node).expect("document has more than u32::MAX nodes");
            if !detached_each_template(d, node) && crate::slir::str_at(d, key_ref) == key {
                return KeyResolution::Found(node);
            }
        }
    }
    for &node in &lists.sy_id {
        if list::item_ix(lists, d, node) >= 0 && key_of(d, lists, node) == key {
            return KeyResolution::Found(node);
        }
    }
    let nodes = candidate_nodes(d, lists);

    let mut matches = Vec::new();
    if !key.contains('/') {
        let wanted = key.strip_prefix('#').unwrap_or(key);
        for &node in &nodes {
            let base = list::base(lists, d, node);
            let Some(&id_ref) = d.node_id.get(usize::try_from(base).unwrap_or(usize::MAX)) else {
                continue;
            };
            if id_ref != 0 && crate::slir::str_at(d, id_ref) == wanted {
                matches.push(node);
            }
        }
        if matches.is_empty() {
            for &node in &nodes {
                let full = key_of(d, lists, node);
                let leaf = full.rsplit('/').next().unwrap_or(full.as_str());
                if leaf.strip_prefix('#').unwrap_or(leaf) == wanted {
                    matches.push(node);
                }
            }
        }
    } else if key.starts_with('#') {
        for &node in &nodes {
            let full = key_of(d, lists, node);
            if full == key
                || full
                    .strip_suffix(key)
                    .is_some_and(|prefix| prefix.ends_with('/'))
            {
                matches.push(node);
            }
        }
    }

    if matches.len() == 1 {
        return KeyResolution::Found(matches[0]);
    }
    if !matches.is_empty() {
        return KeyResolution::Ambiguous {
            candidates: unique_keys(d, lists, &matches),
        };
    }

    let all = unique_keys(d, lists, &nodes);
    let mut scored: Vec<((usize, usize), String)> = all
        .into_iter()
        .map(|candidate| (key_distance(&candidate, key), candidate))
        .collect();
    scored.sort_by_key(|(score, _)| *score);
    KeyResolution::Missing {
        candidates: scored
            .into_iter()
            .take(5)
            .map(|(_, candidate)| candidate)
            .collect(),
    }
}

/// Returns the resolved node, or [`NONE`] for a missing or ambiguous locator.
pub fn node_by_key(d: &Doc, lists: &State, key: &str) -> u32 {
    match resolve_key(d, lists, key) {
        KeyResolution::Found(node) => node,
        KeyResolution::Missing { .. } | KeyResolution::Ambiguous { .. } => NONE,
    }
}

/// Escapes one authored key or item-key segment for an unambiguous full path.
///
/// `%`, `/`, and `~` are the structural reserved bytes and use uppercase
/// percent escapes. Other UTF-8 content remains readable.
pub fn escape_segment(segment: &str) -> String {
    let mut escaped = String::with_capacity(segment.len());
    for ch in segment.chars() {
        match ch {
            '%' => escaped.push_str("%25"),
            '/' => escaped.push_str("%2F"),
            '~' => escaped.push_str("%7E"),
            _ => escaped.push(ch),
        }
    }
    escaped
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
    let item = escape_segment(list::item_key_ref(lists, d, node));
    let mut key = String::with_capacity(prefix.len() + item.len() + relative.len() + 2);
    key.push_str(&prefix);
    key.push('~');
    key.push_str(&item);
    key.push('/');
    key.push_str(relative);
    key
}
