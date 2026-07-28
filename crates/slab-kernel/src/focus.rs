//! Focus traversal and restoration (SPEC §15.3).
//!
//! Document order is tab order, except that an `attach=` overlay subtree
//! inserts into traversal immediately after its anchor node. Keyboard-driven
//! focus sets both `focus` and `focus-visible`, while pointer focus sets only
//! `focus`. When a focused node disappears or becomes ineligible after a
//! re-solve, focus moves to the anchor of a removed overlay that contained it,
//! then to the nearest following entry in the previous focusables list, then
//! the nearest preceding entry, or clears.

use crate::{
	scene::{self, Scene},
	slir::{Doc, NONE},
	style::{self, St},
};

/// The current focus state and the focusable-node snapshot used for
/// restoration.
#[derive(Clone, Debug)]
pub struct FSt {
	/// The focused node ID, or [`NONE`] when no node is focused.
	pub focus:           u32,
	/// Whether keyboard-driven focus should display a focus ring.
	pub visible:         bool,
	/// Node IDs that were focusable after the previous solve, in traversal
	/// order.
	pub last_focusables: Vec<u32>,
	/// Overlay roots parallel to [`Self::last_focusables`]; the innermost
	/// `attach=` subtree root containing each entry, or [`NONE`].
	pub last_overlays:   Vec<u32>,
	/// Overlay anchors parallel to [`Self::last_focusables`], or [`NONE`].
	pub last_anchors:    Vec<u32>,
}

/// Creates an empty focus state.
pub const fn fst_new() -> FSt {
	FSt {
		focus:           NONE,
		visible:         false,
		last_focusables: Vec::new(),
		last_overlays:   Vec::new(),
		last_anchors:    Vec::new(),
	}
}

/// Moves focus to `node`, or clears focus when `node` is [`NONE`].
///
/// `visible` marks keyboard-driven focus and enables its focus ring. Returns
/// `true` when either the focused node or its visibility changed.
pub fn set_focus(d: &Doc, st: &mut St, fs: &mut FSt, node: u32, visible: bool) -> bool {
	let visible = visible && node != NONE;
	if fs.focus == node && fs.visible == visible {
		return false;
	}

	if fs.focus != NONE {
		style::set_node_state(d, st, fs.focus, "focus", false);
		style::set_node_state(d, st, fs.focus, "focus-visible", false);
	}

	fs.focus = node;
	fs.visible = visible;
	if node != NONE {
		style::set_node_state(d, st, node, "focus", true);
		if visible {
			style::set_node_state(d, st, node, "focus-visible", true);
		}
	}

	true
}

/// Finds the innermost `attach=` overlay containing a scene entry.
///
/// Walks scene parents from `scene_index` inclusive and returns the first
/// node whose effective style carries `attach`, together with the resolved
/// anchor node, or `(NONE, NONE)`.
fn attach_overlay_of(d: &Doc, st: &St, sc: &Scene, mut scene_index: i32) -> (u32, u32) {
	while scene_index >= 0 {
		let index = usize::try_from(scene_index).expect("negative scene index");
		let entry = &sc.entries[index];
		let node = entry.node;
		if let Some(rule) = st.rs.iter().rev().find(|rule| rule.node == node)
			&& rule.has_attach
		{
			return (node, scene::node_by_key(d, &st.lists, &rule.attach));
		}
		scene_index = entry.parent_ix;
	}
	(NONE, NONE)
}

/// Maximum nesting depth honored when overlays anchor inside other overlays.
const ATTACH_DEPTH_CAP: u32 = 32;

/// Appends one node's lexicographic traversal-sort key.
///
/// A plain node's key is its authored rank. A node inside an `attach=`
/// overlay extends its anchor's key so the whole overlay subtree sorts
/// immediately after the anchor node and before the anchor's own focusable
/// descendants and following siblings.
fn push_traversal_key(
	d: &Doc,
	st: &St,
	sc: &Scene,
	ranks: &[u32],
	node: u32,
	depth: u32,
	key: &mut Vec<u32>,
) {
	let scene_index = scene::index_of(sc, node);
	if scene_index < 0 {
		key.push(u32::MAX);
		return;
	}
	let index = usize::try_from(scene_index).expect("negative scene index");
	let (overlay, anchor) = attach_overlay_of(d, st, sc, scene_index);
	if depth < ATTACH_DEPTH_CAP && overlay != NONE && anchor != NONE && anchor != node {
		let overlay_index = scene::index_of(sc, overlay);
		let anchor_index = scene::index_of(sc, anchor);
		if overlay_index >= 0 && anchor_index >= 0 {
			push_traversal_key(d, st, sc, ranks, anchor, depth + 1, key);
			key.push(1);
			key.push(ranks[usize::try_from(overlay_index).expect("negative scene index")]);
			key.push(ranks[index]);
			return;
		}
	}
	key.push(ranks[index]);
}

fn authored_ranks(sc: &Scene) -> Vec<u32> {
	let len = sc.entries.len();
	let mut ranks = (0..len)
		.map(|index| u32::try_from(index).expect("scene exceeds u32::MAX entries"))
		.collect::<Vec<u32>>();
	if sc.authored_order.len() == len
		&& sc.authored_order.iter().all(|&index| index < len)
	{
		for (position, &index) in sc.authored_order.iter().enumerate() {
			ranks[index] = u32::try_from(position).expect("scene exceeds u32::MAX entries");
		}
	}
	ranks
}

/// Writes focusable nodes in keyboard-traversal order.
///
/// This is materialized authored order, except that every `attach=` overlay
/// subtree is moved to sit immediately after its anchor node.
pub fn traversal_order(d: &Doc, st: &St, sc: &Scene, out: &mut Vec<u32>) {
	scene::focusables(sc, out);
	if out.is_empty() || !st.rs.iter().any(|rule| rule.has_attach) {
		return;
	}
	let ranks = authored_ranks(sc);
	let mut keyed = out
		.iter()
		.map(|&node| {
			let mut key = Vec::new();
			push_traversal_key(d, st, sc, &ranks, node, 0, &mut key);
			(key, node)
		})
		.collect::<Vec<(Vec<u32>, u32)>>();
	keyed.sort_by(|left, right| left.0.cmp(&right.0));
	out.clear();
	out.extend(keyed.iter().map(|(_, node)| *node));
}

/// Traverses the current scene's focusable nodes in traversal order, wrapping.
///
/// `back` selects reverse traversal. The resulting focus is keyboard-driven,
/// so its focus ring is visible. Returns `true` when the focus state changed.
pub fn focus_next(d: &Doc, st: &mut St, sc: &Scene, fs: &mut FSt, back: bool) -> bool {
	let mut focusables = Vec::new();
	traversal_order(d, st, sc, &mut focusables);
	if focusables.is_empty() {
		return set_focus(d, st, fs, NONE, false);
	}

	let current = focusables.iter().rposition(|&node| node == fs.focus);
	let next = match (current, back) {
		(Some(0) | None, true) => focusables.len() - 1,
		(Some(index), true) => index - 1,
		(Some(index), false) => (index + 1) % focusables.len(),
		(None, false) => 0,
	};

	set_focus(d, st, fs, focusables[next], true)
}

/// Restores focus when the focused node is absent or ineligible in the new
/// scene.
///
/// A focus inside an `attach=` overlay whose whole overlay left the scene
/// returns to the overlay's anchor. Otherwise the nearest following entry in
/// the previous focusables list is preferred, followed by the nearest
/// preceding entry in reverse order. If neither remains, focus is cleared.
/// Restored focus is pointer-grade and has no ring.
pub fn restore(d: &Doc, st: &mut St, sc: &Scene, fs: &mut FSt) -> bool {
	let previous_index = fs
		.last_focusables
		.iter()
		.rposition(|&node| node == fs.focus);

	if let Some(index) = previous_index
		&& fs.last_overlays.get(index).copied().unwrap_or(NONE) != NONE
		&& scene::index_of(sc, fs.last_overlays[index]) < 0
	{
		let anchor = fs.last_anchors[index];
		if anchor != NONE && scene::is_focusable(sc, anchor) {
			return set_focus(d, st, fs, anchor, false);
		}
	}

	let previous_index = previous_index.unwrap_or(0);
	let mut current = Vec::new();
	scene::focusables(sc, &mut current);
	let restored = fs
		.last_focusables
		.get(previous_index.saturating_add(1)..)
		.unwrap_or_default()
		.iter()
		.copied()
		.find(|node| current.contains(node))
		.or_else(|| {
			fs.last_focusables[..previous_index]
				.iter()
				.rev()
				.copied()
				.find(|node| current.contains(node))
		})
		.unwrap_or(NONE);

	set_focus(d, st, fs, restored, false)
}

/// Snapshots the new scene's focusable nodes for the next restoration.
///
/// Each entry also records its innermost `attach=` overlay root and that
/// overlay's anchor so a removed overlay can hand focus back to its anchor.
pub fn refresh(d: &Doc, st: &St, sc: &Scene, fs: &mut FSt) {
	traversal_order(d, st, sc, &mut fs.last_focusables);
	fs.last_overlays.clear();
	fs.last_anchors.clear();
	for &node in &fs.last_focusables {
		let (overlay, anchor) = attach_overlay_of(d, st, sc, scene::index_of(sc, node));
		fs.last_overlays.push(overlay);
		fs.last_anchors.push(anchor);
	}
}
