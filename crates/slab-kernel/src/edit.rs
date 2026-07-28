//! Text-editing state and transitions.
//!
//! All string indices are codepoint offsets into [`EditState::text`] and land
//! on grapheme-cluster boundaries maintained by [`crate::graphemes`]. Mutation
//! operations report committed text-or-span changes. Undo and redo history use
//! capped parallel stacks to preserve the kernel's stable data layout.

use std::fmt::Write as _;

use unicode_segmentation::UnicodeSegmentation as _;

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

/// Bold inline-style identifier used by host rich-field APIs.
pub const STYLE_BOLD: u32 = 0;
/// Italic inline-style identifier used by host rich-field APIs.
pub const STYLE_ITALIC: u32 = 1;
/// Underline inline-style identifier used by host rich-field APIs.
pub const STYLE_UNDERLINE: u32 = 2;
/// Strike-through inline-style identifier used by host rich-field APIs.
pub const STYLE_STRIKE: u32 = 3;
/// Monospace-family inline-style identifier used by host rich-field APIs.
pub const STYLE_CODE: u32 = 4;

/// Sorted, disjoint, non-empty codepoint ranges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ranges(pub Vec<(i32, i32)>);

impl Ranges {
	/// Removes empty ranges, sorts them, and merges overlap or adjacency.
	pub fn normalize(&mut self) {
		self.0.retain(|(a, b)| a < b);
		self.0.sort_unstable();
		let mut out: Vec<(i32, i32)> = Vec::with_capacity(self.0.len());
		for &(a, b) in &self.0 {
			match out.last_mut() {
				Some((_, previous_end)) if a <= *previous_end => {
					*previous_end = (*previous_end).max(b);
				},
				_ => out.push((a, b)),
			}
		}
		self.0 = out;
	}

	/// Adjusts ranges for `len` codepoints inserted at `at`.
	pub fn insert(&mut self, at: i32, len: i32) {
		for (start, end) in &mut self.0 {
			if at <= *start {
				*start = start.wrapping_add(len);
				*end = end.wrapping_add(len);
			} else if at <= *end {
				*end = end.wrapping_add(len);
			}
		}
	}

	/// Adjusts ranges for deletion of `[a, b)`.
	pub fn delete(&mut self, a: i32, b: i32) {
		let deleted = b.saturating_sub(a);
		if deleted == 0 {
			return;
		}
		let map = |point: i32| {
			if point <= a {
				point
			} else if point >= b {
				point - deleted
			} else {
				a
			}
		};
		for (start, end) in &mut self.0 {
			*start = map(*start);
			*end = map(*end);
		}
		self.normalize();
	}

	/// Reports whether every codepoint of `[a, b)` is covered.
	pub fn covers(&self, a: i32, b: i32) -> bool {
		if a >= b {
			return false;
		}
		let mut need = a;
		for &(start, end) in &self.0 {
			if start <= need && need < end {
				need = end;
				if need >= b {
					return true;
				}
			}
		}
		false
	}

	/// Reports whether `point` is inside a range.
	pub fn contains(&self, point: i32) -> bool {
		self
			.0
			.iter()
			.any(|&(start, end)| start <= point && point < end)
	}

	/// Removes `[a, b)` when fully covered, otherwise adds it.
	pub fn toggle(&mut self, a: i32, b: i32) {
		if a >= b {
			return;
		}
		if self.covers(a, b) {
			let mut out = Vec::with_capacity(self.0.len() + 1);
			for &(start, end) in &self.0 {
				if end <= a || start >= b {
					out.push((start, end));
				} else {
					if start < a {
						out.push((start, a));
					}
					if end > b {
						out.push((b, end));
					}
				}
			}
			self.0 = out;
		} else {
			self.0.push((a, b));
			self.normalize();
		}
	}

	/// Keeps the prefix here and returns the tail rebased to zero.
	pub fn split_off(&mut self, at: i32) -> Self {
		let mut right = Vec::new();
		let mut left = Vec::new();
		for &(start, end) in &self.0 {
			if end <= at {
				left.push((start, end));
			} else if start >= at {
				right.push((start - at, end - at));
			} else {
				left.push((start, at));
				right.push((0, end - at));
			}
		}
		self.0 = left;
		Self(right)
	}

	/// Appends `other`, shifting it by `offset`.
	pub fn append(&mut self, other: &Self, offset: i32) {
		self.0.extend(
			other
				.0
				.iter()
				.map(|&(start, end)| (start.wrapping_add(offset), end.wrapping_add(offset))),
		);
		self.normalize();
	}
}

/// The five fixed inline-style span sets for one field.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InlineSpans {
	pub bold:      Ranges,
	pub italic:    Ranges,
	pub underline: Ranges,
	pub strike:    Ranges,
	pub code:      Ranges,
}

impl InlineSpans {
	/// Returns one style's ranges, or `None` for an unknown style identifier.
	pub const fn get(&self, style: u32) -> Option<&Ranges> {
		match style {
			STYLE_BOLD => Some(&self.bold),
			STYLE_ITALIC => Some(&self.italic),
			STYLE_UNDERLINE => Some(&self.underline),
			STYLE_STRIKE => Some(&self.strike),
			STYLE_CODE => Some(&self.code),
			_ => None,
		}
	}

	/// Returns one style's mutable ranges, or `None` for an unknown identifier.
	pub const fn get_mut(&mut self, style: u32) -> Option<&mut Ranges> {
		match style {
			STYLE_BOLD => Some(&mut self.bold),
			STYLE_ITALIC => Some(&mut self.italic),
			STYLE_UNDERLINE => Some(&mut self.underline),
			STYLE_STRIKE => Some(&mut self.strike),
			STYLE_CODE => Some(&mut self.code),
			_ => None,
		}
	}

	fn delete(&mut self, a: i32, b: i32) {
		self.bold.delete(a, b);
		self.italic.delete(a, b);
		self.underline.delete(a, b);
		self.strike.delete(a, b);
		self.code.delete(a, b);
	}

	fn insert(&mut self, at: i32, len: i32) {
		self.bold.insert(at, len);
		self.italic.insert(at, len);
		self.underline.insert(at, len);
		self.strike.insert(at, len);
		self.code.insert(at, len);
	}

	/// Reports whether all style sets are empty.
	pub const fn is_empty(&self) -> bool {
		self.bold.0.is_empty()
			&& self.italic.0.is_empty()
			&& self.underline.0.is_empty()
			&& self.strike.0.is_empty()
			&& self.code.0.is_empty()
	}
}

/// Encodes one rich-field change payload as deterministic JSON.
pub fn spans_json(revision: u64, spans: &InlineSpans) -> String {
	let count: usize = (0..=STYLE_CODE)
		.map(|style| spans.get(style).map_or(0, |ranges| ranges.0.len()))
		.sum();
	let mut json = String::with_capacity(24 + count * 32);
	write!(&mut json, "{{\"rev\":{revision},\"runs\":[").expect("writing to String cannot fail");
	let mut first = true;
	for style in 0..=STYLE_CODE {
		for &(start, end) in &spans.get(style).expect("known style").0 {
			if !first {
				json.push(',');
			}
			first = false;
			write!(&mut json, "{{\"style\":{style},\"start\":{start},\"end\":{end}}}")
				.expect("writing to String cannot fail");
		}
	}
	json.push_str("]}");
	json
}

const NO_GOAL_X: f64 = -1.0;

/// A logical selection whose endpoints belong to different editable fields.
///
/// Keys are escaped canonical full scene paths, so endpoints survive synthetic
/// node replacement, keyed list reorder, and virtual-window changes. Offsets
/// are committed-text codepoint positions on grapheme boundaries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrossFieldRange {
	/// Canonical field key containing the fixed endpoint.
	pub anchor_key:    String,
	/// Fixed endpoint offset.
	pub anchor_offset: i32,
	/// Canonical field key containing the active endpoint.
	pub head_key:      String,
	/// Active endpoint offset.
	pub head_offset:   i32,
}

/// Mutable state for one editable text node.
#[derive(Clone, Debug)]
pub struct EditState {
	/// Node that owns this editing state.
	pub node:            u32,
	/// Committed text.
	pub text:            String,
	/// Active end of the selection, as a codepoint offset.
	pub caret:           i32,
	/// Fixed end of the selection, as a codepoint offset.
	pub anchor:          i32,
	/// Whether an input-method composition is active.
	pub composing:       bool,
	/// Uncommitted composition text displayed at the caret.
	pub compose:         String,
	/// Ordered codepoint ranges for the active preedit clauses, relative to
	/// [`Self::compose`].
	pub compose_clauses: Vec<(i32, i32)>,
	/// Horizontal scroll offset maintained by the caller.
	pub scroll_x:        f64,
	/// Desired horizontal caret position, negative until consecutive visual
	/// up/down movement establishes it.
	pub goal_x:          f64,
	/// Committed inline-style spans.
	pub spans:           InlineSpans,
	/// Monotonic committed text-or-span change counter.
	pub revision:        u64,
	/// Committed text snapshots in the undo stack.
	pub u_text:          Vec<String>,
	/// Inline spans parallel to [`Self::u_text`].
	pub u_spans:         Vec<InlineSpans>,
	/// Caret positions parallel to [`Self::u_text`].
	pub u_caret:         Vec<i32>,
	/// Selection anchors parallel to [`Self::u_text`].
	pub u_anchor:        Vec<i32>,
	/// Committed text snapshots in the redo stack.
	pub r_text:          Vec<String>,
	/// Inline spans parallel to [`Self::r_text`].
	pub r_spans:         Vec<InlineSpans>,
	/// Caret positions parallel to [`Self::r_text`].
	pub r_caret:         Vec<i32>,
	/// Selection anchors parallel to [`Self::r_text`].
	pub r_anchor:        Vec<i32>,
	/// Kind of the current coalesced undo group.
	pub last_kind:       u32,
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
		compose_clauses: Vec::new(),
		scroll_x: 0.0,
		goal_x: NO_GOAL_X,
		spans: InlineSpans::default(),
		revision: 0,
		u_text: Vec::new(),
		u_spans: Vec::new(),
		u_caret: Vec::new(),
		u_anchor: Vec::new(),
		r_text: Vec::new(),
		r_spans: Vec::new(),
		r_caret: Vec::new(),
		r_anchor: Vec::new(),
		last_kind: MUT_NONE,
	}
}

/// Returns the committed text, excluding any active composition.
pub fn text_str(es: &EditState) -> String {
	es.text.clone()
}
/// Returns spans adjusted to the uncommitted composition display.
pub fn display_spans(es: &EditState) -> InlineSpans {
	let mut spans = es.spans.clone();
	if es.composing {
		spans.insert(es.caret, crate::rt::str_len(&es.compose));
	}
	spans
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
/// Replaces one field-local selection and resets movement coalescing state.
///
/// Callers own grapheme-boundary clamping because pointer and host paths
/// already resolve against the field's text layout or boundary table.
pub fn set_selection(es: &mut EditState, caret: i32, anchor: i32) -> bool {
	let changed = es.caret != caret || es.anchor != anchor || es.goal_x != NO_GOAL_X;
	history_barrier(es);
	es.caret = caret;
	es.anchor = anchor;
	es.goal_x = NO_GOAL_X;
	changed
}

/// Removes the oldest undo snapshot when the history cap is reached.
pub fn trim_undo(es: &mut EditState) {
	if es.u_text.len() < usize::try_from(HIST_CAP).expect("negative history cap") {
		return;
	}
	es.u_text.remove(0);
	es.u_spans.remove(0);
	es.u_caret.remove(0);
	es.u_anchor.remove(0);
}

/// Removes the oldest redo snapshot when the history cap is reached.
pub fn trim_redo(es: &mut EditState) {
	if es.r_text.len() < usize::try_from(HIST_CAP).expect("negative history cap") {
		return;
	}
	es.r_text.remove(0);
	es.r_spans.remove(0);
	es.r_caret.remove(0);
	es.r_anchor.remove(0);
}

/// Saves the current committed text and selection to the undo stack.
pub fn push_undo(es: &mut EditState) {
	trim_undo(es);
	es.u_text.push(es.text.clone());
	es.u_spans.push(es.spans.clone());
	es.u_caret.push(es.caret);
	es.u_anchor.push(es.anchor);
}

/// Saves the current committed text and selection to the redo stack.
pub fn push_redo(es: &mut EditState) {
	trim_redo(es);
	es.r_text.push(es.text.clone());
	es.r_spans.push(es.spans.clone());
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
	es.r_spans.clear();
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
/// Discards both history directions and starts a hard host-transaction barrier.
///
/// Unlike [`history_barrier`], which only ends mutation coalescing, this makes
/// every earlier local edit unreachable by field-local undo.
pub fn reset_history(es: &mut EditState) {
	es.u_text.clear();
	es.u_spans.clear();
	es.u_caret.clear();
	es.u_anchor.clear();
	es.r_text.clear();
	es.r_spans.clear();
	es.r_caret.clear();
	es.r_anchor.clear();
	history_barrier(es);
}

/// Restores host-transaction state as a fresh field-local baseline with no
/// undo or redo entries.
pub fn restore_baseline(
	es: &mut EditState,
	text: String,
	spans: InlineSpans,
	caret: i32,
	anchor: i32,
	goal_x: f64,
	revision: u64,
) {
	reset_history(es);
	es.text = text;
	es.spans = spans;
	es.caret = caret;
	es.anchor = anchor;
	es.goal_x = goal_x;
	es.revision = revision;
	es.compose.clear();
	es.compose_clauses.clear();
	es.composing = false;
	history_barrier(es);
}

/// Toggles one style over the non-empty current selection as one undo step.
pub fn toggle_style(es: &mut EditState, style: u32) -> bool {
	let lo = sel_lo(es);
	let hi = sel_hi(es);
	if lo >= hi || es.spans.get(style).is_none() {
		return false;
	}
	history_barrier(es);
	begin_mutation(es, MUT_NONE);
	es.spans
		.get_mut(style)
		.expect("validated style")
		.toggle(lo, hi);
	es.revision = es.revision.wrapping_add(1);
	history_barrier(es);
	true
}

/// Replaces all inline spans as one undo step without touching text.
pub fn replace_spans(es: &mut EditState, spans: InlineSpans) -> bool {
	if es.spans == spans {
		return false;
	}
	history_barrier(es);
	begin_mutation(es, MUT_NONE);
	es.spans = spans;
	es.revision = es.revision.wrapping_add(1);
	history_barrier(es);
	true
}

/// Replaces committed `[lo, hi)` with `insert`, keeping every span consistent.
pub fn splice(es: &mut EditState, lo: i32, hi: i32, insert: &str) -> bool {
	if lo == hi && insert.is_empty() {
		return false;
	}
	if hi > lo {
		es.spans.delete(lo, hi);
	}
	let added = crate::rt::str_len(insert);
	if added > 0 {
		es.spans.insert(lo, added);
	}
	let mut text = crate::rt::str_slice(&es.text, 0, lo);
	text.push_str(insert);
	text.push_str(&crate::rt::str_slice(&es.text, hi, crate::rt::str_len(&es.text)));
	es.text = text;
	es.revision = es.revision.wrapping_add(1);
	true
}

/// Removes the half-open codepoint range `lo..hi` from committed text.
pub fn splice_out(es: &mut EditState, lo: i32, hi: i32) {
	splice(es, lo, hi, "");
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

/// Reports whether `text` ends in Unicode whitespace.
pub fn ends_whitespace(text: &str) -> bool {
	text.chars().last().is_some_and(char::is_whitespace)
}

/// Replaces the selection with input inserted verbatim.
///
/// Dispatch is the single choke point that maps newlines to spaces for a
/// single-line field.
pub fn insert(es: &mut EditState, text: &str) -> bool {
	let lo = sel_lo(es);
	let hi = sel_hi(es);
	if text.is_empty() && lo == hi {
		return false;
	}

	begin_mutation(es, MUT_INSERT);
	let added = crate::rt::str_len(text);
	splice(es, lo, hi, text);
	es.caret = lo.wrapping_add(added);
	es.anchor = es.caret;

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

fn byte_offset(text: &str, codepoint: i32) -> usize {
	let codepoint = usize::try_from(codepoint).expect("negative codepoint offset");
	text
		.char_indices()
		.map(|(byte, _)| byte)
		.chain(std::iter::once(text.len()))
		.nth(codepoint)
		.expect("codepoint offset out of bounds")
}

fn codepoint_offset(text: &str, byte: usize) -> i32 {
	i32::try_from(text[..byte].chars().count()).expect("string has too many codepoints")
}

fn visit_word_stops(text: &str, mut visit: impl FnMut(usize) -> bool) {
	let mut previous_end = 0;
	let mut previous_is_word = false;
	for (byte, segment) in text.split_word_bound_indices() {
		let is_whitespace = segment.chars().all(char::is_whitespace);
		let is_word = segment.chars().any(char::is_alphanumeric);
		if is_whitespace {
			previous_is_word = false;
		} else if is_word {
			if !visit(byte) {
				return;
			}
			previous_is_word = true;
		} else if !(previous_is_word && byte == previous_end) {
			if !visit(byte) {
				return;
			}
			previous_is_word = false;
		}
		previous_end = byte.wrapping_add(segment.len());
	}
}

/// Returns the current or preceding editor word stop.
///
/// Stops begin at UAX #29 word segments and standalone non-whitespace
/// segments. Trailing punctuation and symbols merge into an adjacent preceding
/// word. At a stop, the preceding stop is chosen.
pub fn word_prev(text: &str, caret: i32) -> i32 {
	let caret_byte = byte_offset(text, caret);
	let mut previous = 0;
	visit_word_stops(text, |byte| {
		if byte >= caret_byte {
			return false;
		}
		previous = byte;
		true
	});
	codepoint_offset(text, previous)
}

/// Returns the following editor word stop, or the end of the text.
///
/// Stops begin at UAX #29 word segments and standalone non-whitespace
/// segments. Trailing punctuation and symbols merge into an adjacent preceding
/// word. A stop at the caret is skipped.
pub fn word_next(text: &str, caret: i32) -> i32 {
	let caret_byte = byte_offset(text, caret);
	let mut following = text.len();
	visit_word_stops(text, |byte| {
		if byte <= caret_byte {
			return true;
		}
		following = byte;
		false
	});
	codepoint_offset(text, following)
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
/// requested direction. When `word` is true, movement skips non-word spans
/// between UAX #29 word starts.
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

/// Finds the nearest shaped-cluster caret on `line` for horizontal `goal`.
// The legacy metric arguments remain explicit for callers that already own
// them; shaped layout is the sole geometry authority.
pub fn caret_for_x(
	_d: &Doc,
	_es: &EditState,
	tl: &TextLayout,
	line: i32,
	_font: i32,
	_size: f64,
	_tracking: f64,
	goal: f64,
) -> i32 {
	textm::caret_for_visual_x(tl, usize::try_from(line).expect("negative visual line"), goal)
}

/// Moves one caret stop in visual order across shaped lines.
pub fn visual_step(es: &mut EditState, tl: &TextLayout, delta: i32, select: bool) {
	history_barrier(es);
	es.goal_x = NO_GOAL_X;
	let mut previous = None;
	let mut last = None;
	let mut found = false;
	let mut target = es.caret;
	let mut done = false;
	let mut visit = |boundary: i32| {
		if done || last == Some(boundary) {
			return;
		}
		if delta < 0 && boundary == es.caret {
			target = previous.unwrap_or(boundary);
			done = true;
		} else if delta >= 0 && found {
			target = boundary;
			done = true;
		} else if boundary == es.caret {
			found = true;
		}
		previous = Some(boundary);
		last = Some(boundary);
	};
	for (line, shaped) in tl.shaped.iter().enumerate() {
		if shaped.clusters.is_empty() {
			visit(tl.src_ls[line]);
			continue;
		}
		for cluster in &shaped.clusters {
			visit(if cluster.rtl {
				cluster.end
			} else {
				cluster.start
			});
			visit(if cluster.rtl {
				cluster.start
			} else {
				cluster.end
			});
		}
	}
	if !done && delta >= 0 {
		target = previous.unwrap_or(es.caret);
	}
	es.caret = target;
	if !select {
		es.anchor = es.caret;
	}
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
		es.goal_x = textm::caret_x(tl, line_index, es.caret);
	}

	let target = line.wrapping_add(delta);
	let line_count = i32::try_from(tl.src_ls.len()).expect("too many visual lines");
	// Past the first/last visual line the caret and selection stay put and
	// the computed goal_x is retained, so the unchanged command bubbles to
	// `keys=` for cross-field navigation (SPEC §15.4).
	if target < 0 || target >= line_count {
		return;
	}
	es.caret = caret_for_x(d, es, tl, target, font, size, tracking, es.goal_x);
	if !select {
		es.anchor = es.caret;
	}
}

/// Moves the caret to the start of its visual line.
pub fn visual_home(es: &mut EditState, tl: &TextLayout, select: bool) {
	history_barrier(es);
	let line = usize::try_from(visual_line(tl, es.caret)).expect("negative visual line");
	es.caret = textm::caret_for_visual_x(tl, line, f64::NEG_INFINITY);
	es.goal_x = NO_GOAL_X;
	if !select {
		es.anchor = es.caret;
	}
}

/// Moves the caret to the end of its visual line.
pub fn visual_end(es: &mut EditState, tl: &TextLayout, select: bool) {
	history_barrier(es);
	let line = usize::try_from(visual_line(tl, es.caret)).expect("negative visual line");
	es.caret = textm::caret_for_visual_x(tl, line, f64::INFINITY);
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
	es.compose_clauses.clear();
	es.composing = false;
	es.last_kind = MUT_NONE;
	es.goal_x = NO_GOAL_X;
}

/// Restores the most recent undo snapshot.
pub fn undo(es: &mut EditState) -> bool {
	if es.u_text.is_empty() {
		return false;
	}

	let spans_changed = es.u_spans.last().expect("undo span history out of sync") != &es.spans;
	let text_changed = es.u_text.last().expect("checked non-empty undo history") != &es.text;
	push_redo(es);
	es.text = es.u_text.pop().expect("checked non-empty undo history");
	es.spans = es.u_spans.pop().expect("undo span history out of sync");
	es.caret = es.u_caret.pop().expect("undo caret history out of sync");
	es.anchor = es.u_anchor.pop().expect("undo anchor history out of sync");
	if text_changed || spans_changed {
		es.revision = es.revision.wrapping_add(1);
	}
	finish_history_transition(es);
	text_changed || spans_changed
}

/// Reapplies the most recent redo snapshot.
pub fn redo(es: &mut EditState) -> bool {
	if es.r_text.is_empty() {
		return false;
	}

	let spans_changed = es.r_spans.last().expect("redo span history out of sync") != &es.spans;
	let text_changed = es.r_text.last().expect("checked non-empty redo history") != &es.text;
	push_undo(es);
	es.text = es.r_text.pop().expect("checked non-empty redo history");
	es.spans = es.r_spans.pop().expect("redo span history out of sync");
	es.caret = es.r_caret.pop().expect("redo caret history out of sync");
	es.anchor = es.r_anchor.pop().expect("redo anchor history out of sync");
	if text_changed || spans_changed {
		es.revision = es.revision.wrapping_add(1);
	}
	finish_history_transition(es);
	text_changed || spans_changed
}

/// Updates uncommitted composition text, first replacing any selection.
pub fn composition_update(es: &mut EditState, text: &str) -> bool {
	composition_update_clauses(es, text, &[])
}

/// Updates uncommitted composition text and its codepoint-relative clauses.
///
/// Empty or single-clause payloads use one whole-preedit clause. Multi-clause
/// payloads retain their ordered boundaries after clamping to the preedit.
pub fn composition_update_clauses(es: &mut EditState, text: &str, clauses: &[(i32, i32)]) -> bool {
	let committed_changed = delete_selection(es);
	text.clone_into(&mut es.compose);
	es.compose_clauses.clear();
	let len = crate::rt::str_len(text);
	if len > 0 {
		if clauses.len() <= 1 {
			es.compose_clauses.push((0, len));
		} else {
			es.compose_clauses
				.extend(clauses.iter().filter_map(|&(start, end)| {
					let start = start.clamp(0, len);
					let end = end.clamp(0, len);
					(start < end).then_some((start, end))
				}));
		}
	}
	es.composing = true;
	committed_changed
}

/// Ends composition and inserts non-empty committed composition text.
pub fn composition_end(es: &mut EditState, text: &str) -> bool {
	es.compose.clear();
	es.compose_clauses.clear();
	es.composing = false;
	if text.is_empty() {
		return false;
	}
	history_barrier(es);
	insert(es, text)
}
