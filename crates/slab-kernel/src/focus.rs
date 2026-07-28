//! Focus traversal and restoration (SPEC §15.3).
//!
//! Document order is tab order. Keyboard-driven focus sets both `focus` and
//! `focus-visible`, while pointer focus sets only `focus`. When a focused node
//! disappears or becomes ineligible after a re-solve, focus moves to the
//! nearest following entry in the previous focusables list, then the nearest
//! preceding entry, or clears.

use crate::{
    scene::{self, Scene},
    slir::{Doc, NONE},
    style::{self, St},
};

/// The current focus state and the focusable-node snapshot used for restoration.
#[derive(Clone, Debug)]
pub struct FSt {
    /// The focused node ID, or [`NONE`] when no node is focused.
    pub focus: u32,
    /// Whether keyboard-driven focus should display a focus ring.
    pub visible: bool,
    /// Node IDs that were focusable after the previous solve.
    pub last_focusables: Vec<u32>,
}

/// Creates an empty focus state.
pub fn fst_new() -> FSt {
    FSt {
        focus: NONE,
        visible: false,
        last_focusables: Vec::new(),
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

/// Traverses the current scene's focusable nodes in document order, wrapping.
///
/// `back` selects reverse traversal. The resulting focus is keyboard-driven,
/// so its focus ring is visible. Returns `true` when the focus state changed.
pub fn focus_next(d: &Doc, st: &mut St, sc: &Scene, fs: &mut FSt, back: bool) -> bool {
    let mut focusables = Vec::new();
    scene::focusables(sc, &mut focusables);
    if focusables.is_empty() {
        return set_focus(d, st, fs, NONE, false);
    }

    let current = focusables.iter().rposition(|&node| node == fs.focus);
    let next = match (current, back) {
        (Some(0), true) | (None, true) => focusables.len() - 1,
        (Some(index), true) => index - 1,
        (Some(index), false) => (index + 1) % focusables.len(),
        (None, false) => 0,
    };

    set_focus(d, st, fs, focusables[next], true)
}

/// Restores focus when the focused node is absent or ineligible in the new scene.
///
/// The nearest following entry in the previous focusables list is preferred,
/// followed by the nearest preceding entry in reverse order. If neither
/// remains, focus is cleared. Restored focus is pointer-grade and has no ring.
pub fn restore(d: &Doc, st: &mut St, sc: &Scene, fs: &mut FSt) -> bool {
    let previous_index = fs
        .last_focusables
        .iter()
        .rposition(|&node| node == fs.focus)
        .unwrap_or(0);

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
pub fn refresh(sc: &Scene, fs: &mut FSt) {
    scene::focusables(sc, &mut fs.last_focusables);
}
