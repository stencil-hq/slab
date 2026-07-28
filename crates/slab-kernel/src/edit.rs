//! Text-editing state and transitions.
//!
//! All string indices are codepoint offsets into [`EditState::text`] and land
//! on grapheme-cluster boundaries maintained by [`crate::graphemes`]. Mutating
//! operations return `true` exactly when committed text changed. Undo and redo
//! history use capped parallel stacks to preserve the kernel's stable data
//! layout.

use crate::{
	graphemes,
	slir::Doc,
	textm::{self, TextLayout},
};

/// Maximum number of snapshots retained in each history direction.
pub const HIST_CAP: i32 = 100;

/// Mutation marker used when the next edit must begin a new undo group.
pub const MUT_NONE: u32 = 0;

/// Mutation marker used to coalesce consecutive insertions.
pub const MUT_INSERT: u32 = 1;

/// Mutation marker used to coalesce consecutive deletions.
pub const MUT_DELETE: u32 = 2;

const NO_GOAL_X: f64 = -1.0;

/// Mutable state for one editable text node.
#[derive(Clone, Debug)]
pub struct EditState {
	/// Node that owns this editing state.
	pub node:      u32,
	/// Committed text.
	pub text:      String,
	/// Active end of the selection, as a codepoint offset.
	pub caret:     i32,
	/// Fixed end of the selection, as a codepoint offset.
	pub anchor:    i32,
	/// Whether an input-method composition is active.
	pub composing: bool,
	/// Uncommitted composition text displayed at the caret.
	pub compose:   String,
	/// Horizontal scroll offset maintained by the caller.
	pub scroll_x:  f64,
	/// Desired horizontal caret position, negative until consecutive visual
	/// up/down movement establishes it.
	pub goal_x:    f64,
	/// Committed text snapshots in the undo stack.
	pub u_text:    Vec<String>,
	/// Caret positions parallel to [`Self::u_text`].
	pub u_caret:   Vec<i32>,
	/// Selection anchors parallel to [`Self::u_text`].
	pub u_anchor:  Vec<i32>,
	/// Committed text snapshots in the redo stack.
	pub r_text:    Vec<String>,
	/// Caret positions parallel to [`Self::r_text`].
	pub r_caret:   Vec<i32>,
	/// Selection anchors parallel to [`Self::r_text`].
	pub r_anchor:  Vec<i32>,
	/// Kind of the current coalesced undo group.
	pub last_kind: u32,
}

/// Creates editing state with the caret collapsed at the end of `text`.
pub fn es_new(node: u32, text: &str) -> EditState {
	let end = crate::rt::str_len(text);
	EditState {
		node,
		text: text.to_owned(),
		caret: end,
		anchor: end,
		composing: false,
		compose: String::new(),
		scroll_x: 0.0,
		goal_x: NO_GOAL_X,
		u_text: Vec::new(),
		u_caret: Vec::new(),
		u_anchor: Vec::new(),
		r_text: Vec::new(),
		r_caret: Vec::new(),
		r_anchor: Vec::new(),
		last_kind: MUT_NONE,
	}
}

/// Returns the committed text, excluding any active composition.
pub fn text_str(es: &EditState) -> String {
	es.text.clone()
}

/// Returns the text as displayed, with composition text inserted at the caret.
pub fn display_str(es: &EditState) -> String {
	let mut display = crate::rt::str_slice(&es.text, 0, es.caret);
	display.push_str(&es.compose);
	display.push_str(&crate::rt::str_slice(&es.text, es.caret, crate::rt::str_len(&es.text)));
	display
}

/// Returns the caret offset in [`display_str`], after composition text.
pub fn display_caret(es: &EditState) -> i32 {
	es.caret.wrapping_add(crate::rt::str_len(&es.compose))
}

/// Returns the lower codepoint offset of the current selection.
pub fn sel_lo(es: &EditState) -> i32 {
	es.caret.min(es.anchor)
}

/// Returns the upper codepoint offset of the current selection.
pub fn sel_hi(es: &EditState) -> i32 {
	es.caret.max(es.anchor)
}

/// Removes the oldest undo snapshot when the history cap is reached.
pub fn trim_undo(es: &mut EditState) {
	if es.u_text.len() < usize::try_from(HIST_CAP).expect("negative history cap") {
		return;
	}
	es.u_text.remove(0);
	es.u_caret.remove(0);
	es.u_anchor.remove(0);
}

/// Removes the oldest redo snapshot when the history cap is reached.
pub fn trim_redo(es: &mut EditState) {
	if es.r_text.len() < usize::try_from(HIST_CAP).expect("negative history cap") {
		return;
	}
	es.r_text.remove(0);
	es.r_caret.remove(0);
	es.r_anchor.remove(0);
}

/// Saves the current committed text and selection to the undo stack.
pub fn push_undo(es: &mut EditState) {
	trim_undo(es);
	es.u_text.push(es.text.clone());
	es.u_caret.push(es.caret);
	es.u_anchor.push(es.anchor);
}

/// Saves the current committed text and selection to the redo stack.
pub fn push_redo(es: &mut EditState) {
	trim_redo(es);
	es.r_text.push(es.text.clone());
	es.r_caret.push(es.caret);
	es.r_anchor.push(es.anchor);
}

/// Starts or continues a coalesced mutation group and clears redo history.
pub fn begin_mutation(es: &mut EditState, kind: u32) {
	let starts_group = match es.last_kind {
		MUT_NONE => true,
		last_kind => last_kind != kind,
	};
	if starts_group {
		push_undo(es);
	}
	es.r_text.clear();
	es.r_caret.clear();
	es.r_anchor.clear();
	es.last_kind = kind;
	es.goal_x = NO_GOAL_X;
}

/// Ends mutation coalescing without eagerly saving a snapshot.
///
/// Paste, cut, composition commits, and kills call this before their mutation.
/// A no-op remains a no-op: the following successful mutation owns the
/// snapshot rather than adding a duplicate history entry.
pub const fn history_barrier(es: &mut EditState) {
	es.last_kind = MUT_NONE;
}

/// Removes the half-open codepoint range `lo..hi` from committed text.
pub fn splice_out(es: &mut EditState, lo: i32, hi: i32) {
	let mut text = crate::rt::str_slice(&es.text, 0, lo);
	text.push_str(&crate::rt::str_slice(&es.text, hi, crate::rt::str_len(&es.text)));
	es.text = text;
}

/// Deletes the selection without opening a mutation group.
pub fn delete_selection_raw(es: &mut EditState) -> bool {
	let lo = sel_lo(es);
	let hi = sel_hi(es);
	if lo == hi {
		return false;
	}
	splice_out(es, lo, hi);
	es.caret = lo;
	es.anchor = lo;
	true
}

/// Deletes the selection as part of a coalesced deletion group.
pub fn delete_selection(es: &mut EditState) -> bool {
	if sel_lo(es) == sel_hi(es) {
		return false;
	}
	begin_mutation(es, MUT_DELETE);
	delete_selection_raw(es)
}

/// Reports whether `text` ends in an ASCII space, tab, or line break.
pub fn ends_whitespace(text: &str) -> bool {
	matches!(text.chars().last(), Some(' ' | '\t' | '\n' | '\r'))
}

/// Replaces the selection with input inserted verbatim.
///
/// Dispatch is the single choke point that maps newlines to spaces for a
/// single-line field.
pub fn insert(es: &mut EditState, text: &str) -> bool {
	if text.is_empty() && sel_lo(es) == sel_hi(es) {
		return false;
	}

	begin_mutation(es, MUT_INSERT);
	delete_selection_raw(es);

	let added = crate::rt::str_len(text);
	if added > 0 {
		let mut committed = crate::rt::str_slice(&es.text, 0, es.caret);
		committed.push_str(text);
		committed.push_str(&crate::rt::str_slice(&es.text, es.caret, crate::rt::str_len(&es.text)));
		es.text = committed;
		es.caret = es.caret.wrapping_add(added);
		es.anchor = es.caret;
	}

	// Typing after a whitespace boundary starts a new coalesced undo group:
	// "ab cd" therefore undoes to "ab ", then to empty.
	if ends_whitespace(text) {
		es.last_kind = MUT_NONE;
	}
	true
}

/// Deletes the grapheme immediately before the caret, or the selection.
pub fn backspace(es: &mut EditState) -> bool {
	if sel_lo(es) != sel_hi(es) {
		begin_mutation(es, MUT_DELETE);
		return delete_selection_raw(es);
	}
	if es.caret == 0 {
		return false;
	}

	begin_mutation(es, MUT_DELETE);
	let mut boundaries = Vec::new();
	graphemes::boundaries(&es.text, &mut boundaries);
	let previous = graphemes::prev_boundary(&boundaries, es.caret);
	splice_out(es, previous, es.caret);
	es.caret = previous;
	es.anchor = previous;
	true
}

/// Deletes the grapheme immediately after the caret, or the selection.
pub fn del(es: &mut EditState) -> bool {
	if sel_lo(es) != sel_hi(es) {
		begin_mutation(es, MUT_DELETE);
		return delete_selection_raw(es);
	}
	let end = crate::rt::str_len(&es.text);
	if es.caret >= end {
		return false;
	}

	begin_mutation(es, MUT_DELETE);
	let mut boundaries = Vec::new();
	graphemes::boundaries(&es.text, &mut boundaries);
	let next = graphemes::next_boundary(&boundaries, es.caret, end);
	splice_out(es, es.caret, next);
	es.anchor = es.caret;
	true
}

/// Returns the preceding space-delimited word boundary.
pub fn word_prev(text: &str, caret: i32) -> i32 {
	let before = crate::rt::str_slice(text, 0, caret);
	let split = crate::rt::str_rfind(before.trim_end(), " ");
	if split < 0 { 0 } else { split.wrapping_add(1) }
}

/// Returns the following space-delimited word boundary.
pub fn word_next(text: &str, caret: i32) -> i32 {
	let end = crate::rt::str_len(text);
	let after = crate::rt::str_slice(text, caret, end);
	let split = crate::rt::str_find(&after, " ");
	if split < 0 {
		return end;
	}
	let tail = crate::rt::str_slice(&after, split, crate::rt::str_len(&after));
	end.wrapping_sub(crate::rt::str_len(tail.trim_start()))
}

/// Deletes backward to the preceding word boundary, or deletes the selection.
pub fn word_back(es: &mut EditState) -> bool {
	if sel_lo(es) != sel_hi(es) {
		begin_mutation(es, MUT_DELETE);
		return delete_selection_raw(es);
	}
	let lo = word_prev(&es.text, es.caret);
	if lo == es.caret {
		return false;
	}

	begin_mutation(es, MUT_DELETE);
	splice_out(es, lo, es.caret);
	es.caret = lo;
	es.anchor = lo;
	true
}

/// Deletes forward to the following word boundary, or deletes the selection.
pub fn word_forward(es: &mut EditState) -> bool {
	if sel_lo(es) != sel_hi(es) {
		begin_mutation(es, MUT_DELETE);
		return delete_selection_raw(es);
	}
	let hi = word_next(&es.text, es.caret);
	if hi == es.caret {
		return false;
	}

	begin_mutation(es, MUT_DELETE);
	splice_out(es, es.caret, hi);
	es.anchor = es.caret;
	true
}

/// Moves the caret one grapheme or word in the sign of `delta`.
///
/// When `select` is false, an existing selection first collapses toward the
/// requested direction. When `word` is true, movement uses space-delimited
/// word boundaries.
pub fn move_caret(es: &mut EditState, delta: i32, select: bool, word: bool) {
	history_barrier(es);
	es.goal_x = NO_GOAL_X;

	let lo = sel_lo(es);
	let hi = sel_hi(es);
	if !select && lo != hi {
		es.caret = if delta < 0 { lo } else { hi };
		es.anchor = es.caret;
		return;
	}

	es.caret = if word {
		if delta < 0 {
			word_prev(&es.text, es.caret)
		} else {
			word_next(&es.text, es.caret)
		}
	} else {
		let mut boundaries = Vec::new();
		graphemes::boundaries(&es.text, &mut boundaries);
		if delta < 0 {
			graphemes::prev_boundary(&boundaries, es.caret)
		} else {
			graphemes::next_boundary(&boundaries, es.caret, crate::rt::str_len(&es.text))
		}
	};

	if !select {
		es.anchor = es.caret;
	}
}

/// Moves the caret to the start of committed text.
pub const fn home(es: &mut EditState, select: bool) {
	history_barrier(es);
	es.goal_x = NO_GOAL_X;
	es.caret = 0;
	if !select {
		es.anchor = 0;
	}
}

/// Moves the caret to the end of committed text.
pub fn end(es: &mut EditState, select: bool) {
	history_barrier(es);
	es.goal_x = NO_GOAL_X;
	es.caret = crate::rt::str_len(&es.text);
	if !select {
		es.anchor = es.caret;
	}
}

/// Selects all committed text.
pub fn select_all(es: &mut EditState) {
	history_barrier(es);
	es.goal_x = NO_GOAL_X;
	es.anchor = 0;
	es.caret = crate::rt::str_len(&es.text);
}

/// Finds the visual line containing `caret`.
pub fn visual_line(tl: &TextLayout, caret: i32) -> i32 {
	if tl.src_ls.is_empty() {
		return 0;
	}

	for (index, _) in tl.src_ls.iter().enumerate() {
		let line_end = tl.src_le[index];
		let line = i32::try_from(index).expect("too many visual lines");
		if caret < line_end {
			return line;
		}
		if caret == line_end {
			if tl
				.src_ls
				.get(index + 1)
				.is_some_and(|&start| start == caret)
			{
				continue;
			}
			return line;
		}
	}

	i32::try_from(tl.src_ls.len())
		.expect("too many visual lines")
		.wrapping_sub(1)
}

/// Finds the nearest grapheme-boundary caret on `line` for horizontal `goal`.
// Keeping the text-metric inputs explicit makes this state transition
// auditable.
pub fn caret_for_x(
	d: &Doc,
	es: &EditState,
	tl: &TextLayout,
	line: i32,
	font: i32,
	size: f64,
	tracking: f64,
	goal: f64,
) -> i32 {
	let line = usize::try_from(line).expect("negative visual line");
	let start = tl.src_ls[line];
	let end = tl.src_le[line];

	let mut boundaries = Vec::new();
	graphemes::boundaries(&es.text, &mut boundaries);
	let chars: Vec<u32> = es.text.chars().map(u32::from).collect();
	let mut previous = start;
	let mut x = 0.0;

	for current in boundaries {
		if current <= start {
			continue;
		}
		if current > end {
			break;
		}
		let cluster_width = textm::slice_w(d, font, size, tracking, &chars, previous, current);
		if goal < x + cluster_width / 2.0 {
			return previous;
		}
		x += cluster_width;
		previous = current;
	}
	end
}

/// Moves vertically by visual lines while preserving the desired x position.
// Keeping the text-metric inputs explicit makes this state transition
// auditable.
pub fn visual_move(
	d: &Doc,
	es: &mut EditState,
	tl: &TextLayout,
	font: i32,
	size: f64,
	tracking: f64,
	delta: i32,
	select: bool,
) {
	history_barrier(es);
	let line = visual_line(tl, es.caret);
	if es.goal_x < 0.0 {
		let line_index = usize::try_from(line).expect("negative visual line");
		es.goal_x =
			textm::str_slice_w(d, font, size, tracking, &es.text, tl.src_ls[line_index], es.caret);
	}

	let target = line.wrapping_add(delta);
	let line_count = i32::try_from(tl.src_ls.len()).expect("too many visual lines");
	if target < 0 {
		es.caret = 0;
	} else if target >= line_count {
		es.caret = crate::rt::str_len(&es.text);
	} else {
		es.caret = caret_for_x(d, es, tl, target, font, size, tracking, es.goal_x);
	}
	if !select {
		es.anchor = es.caret;
	}
}

/// Moves the caret to the start of its visual line.
pub fn visual_home(es: &mut EditState, tl: &TextLayout, select: bool) {
	history_barrier(es);
	let line = usize::try_from(visual_line(tl, es.caret)).expect("negative visual line");
	es.caret = tl.src_ls[line];
	es.goal_x = NO_GOAL_X;
	if !select {
		es.anchor = es.caret;
	}
}

/// Moves the caret to the end of its visual line.
pub fn visual_end(es: &mut EditState, tl: &TextLayout, select: bool) {
	history_barrier(es);
	let line = usize::try_from(visual_line(tl, es.caret)).expect("negative visual line");
	es.caret = tl.src_le[line];
	es.goal_x = NO_GOAL_X;
	if !select {
		es.anchor = es.caret;
	}
}

/// Deletes from the caret to the start of its visual line.
pub fn kill_start(es: &mut EditState, tl: &TextLayout) -> bool {
	if sel_lo(es) != sel_hi(es) {
		history_barrier(es);
		return delete_selection(es);
	}

	let line = usize::try_from(visual_line(tl, es.caret)).expect("negative visual line");
	let lo = tl.src_ls[line];
	if lo == es.caret {
		return false;
	}

	history_barrier(es);
	begin_mutation(es, MUT_DELETE);
	splice_out(es, lo, es.caret);
	es.caret = lo;
	es.anchor = lo;
	true
}

/// Deletes from the caret to the end of its visual line.
pub fn kill_end(es: &mut EditState, tl: &TextLayout) -> bool {
	if sel_lo(es) != sel_hi(es) {
		history_barrier(es);
		return delete_selection(es);
	}

	let line = usize::try_from(visual_line(tl, es.caret)).expect("negative visual line");
	let hi = tl.src_le[line];
	if hi == es.caret {
		return false;
	}

	history_barrier(es);
	begin_mutation(es, MUT_DELETE);
	splice_out(es, es.caret, hi);
	es.anchor = es.caret;
	true
}

fn finish_history_transition(es: &mut EditState) {
	es.compose.clear();
	es.composing = false;
	es.last_kind = MUT_NONE;
	es.goal_x = NO_GOAL_X;
}

/// Restores the most recent undo snapshot.
pub fn undo(es: &mut EditState) -> bool {
	if es.u_text.is_empty() {
		return false;
	}

	let text_changed = es.u_text.last().expect("checked non-empty undo history") != &es.text;
	push_redo(es);
	es.text = es.u_text.pop().expect("checked non-empty undo history");
	es.caret = es.u_caret.pop().expect("undo caret history out of sync");
	es.anchor = es.u_anchor.pop().expect("undo anchor history out of sync");
	finish_history_transition(es);
	text_changed
}

/// Reapplies the most recent redo snapshot.
pub fn redo(es: &mut EditState) -> bool {
	if es.r_text.is_empty() {
		return false;
	}

	let text_changed = es.r_text.last().expect("checked non-empty redo history") != &es.text;
	push_undo(es);
	es.text = es.r_text.pop().expect("checked non-empty redo history");
	es.caret = es.r_caret.pop().expect("redo caret history out of sync");
	es.anchor = es.r_anchor.pop().expect("redo anchor history out of sync");
	finish_history_transition(es);
	text_changed
}

/// Updates uncommitted composition text, first replacing any selection.
pub fn composition_update(es: &mut EditState, text: &str) -> bool {
	let committed_changed = delete_selection(es);
	text.clone_into(&mut es.compose);
	es.composing = true;
	committed_changed
}

/// Ends composition and inserts non-empty committed composition text.
pub fn composition_end(es: &mut EditState, text: &str) -> bool {
	es.compose.clear();
	es.composing = false;
	if text.is_empty() {
		return false;
	}
	history_barrier(es);
	insert(es, text)
}
