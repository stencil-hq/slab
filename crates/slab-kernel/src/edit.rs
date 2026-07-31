//! Text-editing state and transitions.
//!
//! All string indices are codepoint offsets into [`EditState::text`] and land
//! on grapheme-cluster boundaries maintained by [`crate::graphemes`]. Mutation
//! operations report committed text-or-span changes. Undo and redo history use
//! capped reverse-delta stacks, with full text only as a non-contiguous
//! fallback.

use std::fmt::Write as _;

use unicode_segmentation::UnicodeSegmentation as _;

use crate::{
	graphemes,
	text::Text,
	textm::{self, TextLayout},
};

/// Maximum number of records retained in each history direction.
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

/// One host-supplied paint-only style over committed field text.
///
/// Offsets are codepoint positions. `flags & 1` requests synthetic italic
/// paint; neither color nor flags participate in text measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldStyle {
	pub start: i32,
	pub end:   i32,
	pub rgba:  u32,
	pub flags: u32,
}

impl FieldStyle {
	/// Paint-only italic flag.
	pub const ITALIC: u32 = 1;
}

const fn inserted_bounds(start: &mut i32, end: &mut i32, at: i32, len: i32) {
	if at <= *start {
		*start = start.wrapping_add(len);
		*end = end.wrapping_add(len);
	} else if at <= *end {
		*end = end.wrapping_add(len);
	}
}

const fn deleted_point(point: i32, start: i32, end: i32) -> i32 {
	if point <= start {
		point
	} else if point >= end {
		point - end.saturating_sub(start)
	} else {
		start
	}
}

/// Sorted, disjoint, non-empty codepoint ranges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ranges(pub Vec<(i32, i32)>);

impl Ranges {
	fn retain_nonempty(&mut self) {
		self.0.retain(|(start, end)| start < end);
	}
}

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
			inserted_bounds(start, end, at, len);
		}
	}

	/// Adjusts ranges for deletion of `[a, b)`.
	pub fn delete(&mut self, a: i32, b: i32) {
		if a >= b {
			return;
		}
		for (start, end) in &mut self.0 {
			*start = deleted_point(*start, a, b);
			*end = deleted_point(*end, a, b);
		}
		self.retain_nonempty();
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
	///
	/// Ranges are normalized (sorted, disjoint), so one lower-bound probe
	/// suffices; rich measurement calls this per codepoint.
	pub fn contains(&self, point: i32) -> bool {
		let index = self.0.partition_point(|&(_, end)| end <= point);
		self.0.get(index).is_some_and(|&(start, _)| start <= point)
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
	/// Creates an empty span set; usable in `const` contexts.
	pub const fn empty() -> Self {
		Self {
			bold:      Ranges(Vec::new()),
			italic:    Ranges(Vec::new()),
			underline: Ranges(Vec::new()),
			strike:    Ranges(Vec::new()),
			code:      Ranges(Vec::new()),
		}
	}
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

	/// Reports whether `next` equals this span set transformed by one
	/// contiguous text splice (`delta` in new-text coordinates).
	///
	/// True proves every boundary outside the edit window moved positionally,
	/// the invariant [`crate::textm::measure_text_spliced_into`] needs to
	/// retain prefix/suffix lines of a rich layout; style toggles or host
	/// span replacement compare unequal and force a full re-measure.
	pub fn follows_splice(&self, next: &Self, delta: crate::textm::TextDelta) -> bool {
		let mut expected = self.clone();
		expected.delete(delta.at, delta.at.wrapping_add(delta.removed));
		expected.insert(delta.at, delta.inserted);
		expected == *next
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

fn field_styles_insert(styles: &mut [FieldStyle], at: i32, len: i32) {
	for style in styles {
		inserted_bounds(&mut style.start, &mut style.end, at, len);
	}
}

fn field_styles_delete(styles: &mut Vec<FieldStyle>, start: i32, end: i32) {
	if start >= end {
		return;
	}
	for style in styles.iter_mut() {
		style.start = deleted_point(style.start, start, end);
		style.end = deleted_point(style.end, start, end);
	}
	styles.retain(|style| style.start < style.end);
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

/// Text transformation that restores one side of a history transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoRecord {
	/// Replaces `inserted_len` codepoints at `at` with `removed`.
	Splice { at: i32, removed: Vec<u32>, inserted_len: i32 },
	/// Complete target text for a mutation group that cannot be one splice.
	Full(Text),
}

/// One undo or redo transition, including the target selection and spans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoStep {
	pub record:       UndoRecord,
	pub spans:        InlineSpans,
	pub field_styles: Vec<FieldStyle>,
	pub caret:        i32,
	pub anchor:       i32,
	text_recorded:    bool,
}

impl UndoStep {
	/// Reports whether this step retains a full text snapshot.
	pub const fn is_full_text(&self) -> bool {
		matches!(self.record, UndoRecord::Full(_))
	}
}

/// Accumulated display-content transition since the last field sync.
///
/// `Splice` composes contiguous edits into one forward delta the measure
/// splice can replay. Anything that breaks splice lineage — non-contiguous
/// edit groups or any input-method display transition — degrades to `Full`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeasureDelta {
	/// Published content is unchanged.
	Unchanged,
	/// Content changed by exactly this contiguous splice.
	Splice(crate::textm::TextDelta),
	/// Content changed in a way no single splice represents.
	Full,
}

/// Mutable state for one editable text node.
#[derive(Clone, Debug)]
pub struct EditState {
	/// Node that owns this editing state.
	pub node:            u32,
	/// Committed text.
	pub text:            Text,
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
	/// Host-supplied paint-only ranges over committed text.
	pub field_styles:    Vec<FieldStyle>,
	/// Monotonic committed text-or-span change counter.
	pub revision:        u64,
	/// Reverse deltas in the undo stack.
	pub undo:            Vec<UndoStep>,
	/// Reverse deltas in the redo stack.
	pub redo:            Vec<UndoStep>,
	/// Kind of the current coalesced undo group.
	pub last_kind:       u32,
	/// Whether the next splice belongs to the mutation just begun.
	pending_splice:      bool,
	/// Display transition since the last field sync (see [`MeasureDelta`]).
	measure_delta:       MeasureDelta,
	/// Whether the published display text diverged from committed-splice
	/// lineage (any composition activity); forces full re-measure on sync.
	display_dirty:       bool,
}

impl EditState {
	/// Accumulates one committed forward splice into the pending transition.
	fn accumulate_delta(&mut self, next: crate::textm::TextDelta) {
		self.measure_delta = match self.measure_delta {
			MeasureDelta::Unchanged => MeasureDelta::Splice(next),
			MeasureDelta::Splice(mut delta) => {
				if delta.merge(next) {
					MeasureDelta::Splice(delta)
				} else {
					MeasureDelta::Full
				}
			},
			MeasureDelta::Full => MeasureDelta::Full,
		};
	}

	/// Consumes the pending transition for one field sync.
	///
	/// Composition activity poisons the lineage: every sync during a
	/// composition, and the first one after it ends, re-measures fully and
	/// re-baselines.
	pub const fn take_measure_delta(&mut self) -> MeasureDelta {
		if self.composing || self.display_dirty {
			self.display_dirty = self.composing;
			self.measure_delta = MeasureDelta::Unchanged;
			return MeasureDelta::Full;
		}
		std::mem::replace(&mut self.measure_delta, MeasureDelta::Unchanged)
	}

	/// Discards the pending transition after a host publishes full content.
	pub const fn reset_measure_delta(&mut self) {
		self.measure_delta = MeasureDelta::Unchanged;
		self.display_dirty = self.composing;
	}
}

/// Creates editing state with the caret collapsed at the end of `text`.
pub fn es_new(node: u32, text: &str) -> EditState {
	let text = Text::from(text);
	let end = text.len();
	EditState {
		node,
		text,
		caret: end,
		anchor: end,
		composing: false,
		compose: String::new(),
		compose_clauses: Vec::new(),
		scroll_x: 0.0,
		goal_x: NO_GOAL_X,
		spans: InlineSpans::default(),
		field_styles: Vec::new(),
		revision: 0,
		undo: Vec::new(),
		redo: Vec::new(),
		last_kind: MUT_NONE,
		pending_splice: false,
		measure_delta: MeasureDelta::Unchanged,
		display_dirty: false,
	}
}

/// Returns the committed text, excluding any active composition.
pub fn text_str(es: &EditState) -> String {
	es.text.to_utf8()
}
/// Returns spans adjusted to the uncommitted composition display.
pub fn display_spans(es: &EditState) -> InlineSpans {
	let mut spans = es.spans.clone();
	if es.composing {
		spans.insert(es.caret, crate::rt::str_len(&es.compose));
	}
	spans
}

/// Returns paint-only styles projected around uncommitted composition text.
///
/// The preedit itself remains unstyled; a range crossing the caret is split so
/// the committed text on both sides keeps its paint after the display shift.
pub fn display_field_styles(es: &EditState) -> std::borrow::Cow<'_, [FieldStyle]> {
	if !es.composing || es.compose.is_empty() || es.field_styles.is_empty() {
		return std::borrow::Cow::Borrowed(&es.field_styles);
	}
	let inserted = crate::rt::str_len(&es.compose);
	let mut display = Vec::with_capacity(es.field_styles.len().saturating_add(1));
	for style in &es.field_styles {
		if style.end <= es.caret {
			display.push(*style);
		} else if style.start >= es.caret {
			display.push(FieldStyle {
				start: style.start.wrapping_add(inserted),
				end: style.end.wrapping_add(inserted),
				..*style
			});
		} else {
			display.push(FieldStyle { end: es.caret, ..*style });
			display.push(FieldStyle {
				start: es.caret.wrapping_add(inserted),
				end: style.end.wrapping_add(inserted),
				..*style
			});
		}
	}
	std::borrow::Cow::Owned(display)
}

/// Returns the text as displayed, with composition text inserted at the caret.
pub fn display_text(es: &EditState) -> Text {
	if !es.composing && es.compose.is_empty() {
		// Hot path: committed text displays verbatim; one reference clone.
		return es.text.clone();
	}
	let cps = es.text.cps();
	let caret = usize::try_from(es.caret).expect("negative caret");
	let mut display = Vec::with_capacity(cps.len() + es.compose.len());
	display.extend_from_slice(&cps[..caret]);
	display.extend(es.compose.chars().map(u32::from));
	display.extend_from_slice(&cps[caret..]);
	Text::from_cps(display)
}

/// Returns the caret offset in [`display_text`], after composition text.
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

/// Removes the oldest undo record when the history cap is reached.
pub fn trim_undo(es: &mut EditState) {
	if es.undo.len() < usize::try_from(HIST_CAP).expect("negative history cap") {
		return;
	}
	es.undo.remove(0);
}

/// Removes the oldest redo record when the history cap is reached.
pub fn trim_redo(es: &mut EditState) {
	if es.redo.len() < usize::try_from(HIST_CAP).expect("negative history cap") {
		return;
	}
	es.redo.remove(0);
}

fn current_step(es: &EditState) -> UndoStep {
	UndoStep {
		record:        UndoRecord::Splice {
			at:           0,
			removed:      Vec::new(),
			inserted_len: 0,
		},
		spans:         es.spans.clone(),
		field_styles:  es.field_styles.clone(),
		caret:         es.caret,
		anchor:        es.anchor,
		text_recorded: false,
	}
}

/// Starts an undo record at the current spans and selection.
pub fn push_undo(es: &mut EditState) {
	trim_undo(es);
	es.undo.push(current_step(es));
}

/// Starts a redo record at the current spans and selection.
pub fn push_redo(es: &mut EditState) {
	trim_redo(es);
	es.redo.push(current_step(es));
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
	es.redo.clear();
	es.last_kind = kind;
	es.pending_splice = true;
	es.goal_x = NO_GOAL_X;
}

/// Ends mutation coalescing without eagerly saving a snapshot.
///
/// Paste, cut, composition commits, and kills call this before their mutation.
/// A no-op remains a no-op: the following successful mutation owns the
/// snapshot rather than adding a duplicate history entry.
pub const fn history_barrier(es: &mut EditState) {
	es.last_kind = MUT_NONE;
	es.pending_splice = false;
}
/// Discards both history directions and starts a hard host-transaction barrier.
///
/// Unlike [`history_barrier`], which only ends mutation coalescing, this makes
/// every earlier local edit unreachable by field-local undo.
pub fn reset_history(es: &mut EditState) {
	es.undo.clear();
	es.redo.clear();
	history_barrier(es);
}

/// Restores host-transaction state as a fresh field-local baseline with no
/// undo or redo entries.
pub fn restore_baseline(
	es: &mut EditState,
	text: &str,
	spans: InlineSpans,
	caret: i32,
	anchor: i32,
	goal_x: f64,
	revision: u64,
) {
	reset_history(es);
	es.text = Text::from(text);
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

fn text_with_reverse_splice(text: &Text, at: i32, removed: &[u32], inserted_len: i32) -> Text {
	let cps = text.cps();
	let at_index = usize::try_from(at).expect("negative splice offset");
	let old_end = usize::try_from(at.wrapping_add(inserted_len)).expect("negative splice end");
	let mut restored = Vec::with_capacity(
		cps.len()
			.saturating_add(removed.len())
			.saturating_sub(old_end - at_index),
	);
	restored.extend_from_slice(&cps[..at_index]);
	restored.extend_from_slice(removed);
	restored.extend_from_slice(&cps[old_end..]);
	Text::from_cps(restored)
}

fn record_splice(es: &mut EditState, lo: i32, hi: i32, insert_len: i32) {
	if !es.pending_splice {
		return;
	}
	es.pending_splice = false;
	let step = es
		.undo
		.last_mut()
		.expect("mutation group has an undo record");
	if !step.text_recorded {
		step.record = UndoRecord::Splice {
			at:           lo,
			removed:      es.text.slice_cps(lo, hi),
			inserted_len: insert_len,
		};
		step.text_recorded = true;
		return;
	}

	let UndoRecord::Splice { at, removed, inserted_len } = &step.record else {
		return;
	};
	let old_end = at.wrapping_add(*inserted_len);
	if hi < *at || lo > old_end {
		step.record =
			UndoRecord::Full(text_with_reverse_splice(&es.text, *at, removed, *inserted_len));
		return;
	}

	let start = (*at).min(lo);
	let end = old_end.max(hi);
	let mut baseline = es.text.slice_cps(start, *at);
	baseline.extend_from_slice(removed);
	baseline.extend_from_slice(
		&es.text.cps()[usize::try_from(old_end).expect("negative splice end")
			..usize::try_from(end).expect("negative splice end")],
	);
	step.record = UndoRecord::Splice {
		at:           start,
		removed:      baseline,
		inserted_len: lo
			.wrapping_sub(start)
			.wrapping_add(insert_len)
			.wrapping_add(end.wrapping_sub(hi)),
	};
}

/// Replaces committed `[lo, hi)` with `insert`, keeping every span consistent.
pub fn splice(es: &mut EditState, lo: i32, hi: i32, insert: &str) -> bool {
	if lo == hi && insert.is_empty() {
		return false;
	}
	let insert_cps: Vec<u32> = insert.chars().map(u32::from).collect();
	let added = i32::try_from(insert_cps.len()).expect("insert has too many codepoints");
	record_splice(es, lo, hi, added);
	es.accumulate_delta(crate::textm::TextDelta {
		at:       lo,
		removed:  hi.wrapping_sub(lo),
		inserted: added,
	});
	if hi > lo {
		es.spans.delete(lo, hi);
		field_styles_delete(&mut es.field_styles, lo, hi);
	}
	if added > 0 {
		es.spans.insert(lo, added);
		field_styles_insert(&mut es.field_styles, lo, added);
	}
	es.text.splice(lo, hi, &insert_cps);
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
	let previous = graphemes::prev_boundary_in(es.text.cps(), es.caret);
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
	let end = es.text.len();
	if es.caret >= end {
		return false;
	}

	begin_mutation(es, MUT_DELETE);
	let next = graphemes::next_boundary_in(es.text.cps(), es.caret);
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
pub fn word_prev(text: &Text, caret: i32) -> i32 {
	let text = text.to_utf8();
	let text = text.as_str();
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
pub fn word_next(text: &Text, caret: i32) -> i32 {
	let text = text.to_utf8();
	let text = text.as_str();
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
		if delta < 0 {
			graphemes::prev_boundary_in(es.text.cps(), es.caret)
		} else {
			graphemes::next_boundary_in(es.text.cps(), es.caret)
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
	es.caret = es.text.len();
	if !select {
		es.anchor = es.caret;
	}
}

/// Selects all committed text.
pub fn select_all(es: &mut EditState) {
	history_barrier(es);
	es.goal_x = NO_GOAL_X;
	es.anchor = 0;
	es.caret = es.text.len();
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
pub(crate) fn caret_for_x(shaper: textm::Shaper<'_>, tl: &TextLayout, line: i32, goal: f64) -> i32 {
	textm::caret_for_visual_x(shaper, tl, usize::try_from(line).expect("negative visual line"), goal)
}

/// Returns the final caret stop of the whole layout: the trailing edge of
/// the last line's last cluster, or the line's source start when empty.
fn doc_tail_boundary(shaper: textm::Shaper<'_>, tl: &TextLayout) -> Option<i32> {
	let line = tl.src_ls.len().checked_sub(1)?;
	let shaped = shaper.line(tl, line)?;
	if shaped.clusters.is_empty() {
		return Some(tl.src_ls[line]);
	}
	let base = tl.src_ls[line];
	let clamp = tl.src_le[line];
	Some(
		shaped
			.clusters
			.iter()
			.map(|cluster| cluster.end.wrapping_add(base).min(clamp))
			.max()
			.unwrap_or(tl.src_ls[line]),
	)
}

/// Moves one caret stop across shaped lines.
///
/// Stops are grapheme boundaries in LOGICAL order: on mixed-direction lines
/// a logical value can appear at two visual positions (run seams), and a
/// visual walk would match the wrong occurrence and orbit. Pure-LTR lines
/// produce the identical stream either way. Cluster edges only exist on
/// lines whose source range contains the caret, so scanning one line of
/// context on each side reproduces the full stream without shaping the
/// whole field.
pub(crate) fn visual_step(
	shaper: textm::Shaper<'_>,
	es: &mut EditState,
	tl: &TextLayout,
	delta: i32,
	select: bool,
) {
	history_barrier(es);
	es.goal_x = NO_GOAL_X;
	struct Step {
		previous: Option<i32>,
		last:     Option<i32>,
		found:    bool,
		target:   i32,
		done:     bool,
	}
	impl Step {
		fn visit(&mut self, boundary: i32, caret: i32, delta: i32) {
			if self.done || self.last == Some(boundary) {
				return;
			}
			if delta < 0 && boundary == caret {
				self.target = self.previous.unwrap_or(boundary);
				self.done = true;
			} else if delta >= 0 && self.found {
				self.target = boundary;
				self.done = true;
			} else if boundary == caret {
				self.found = true;
			}
			self.previous = Some(boundary);
			self.last = Some(boundary);
		}
	}
	let mut step =
		Step { previous: None, last: None, found: false, target: es.caret, done: false };
	let lines = tl.src_ls.len();
	let mut first = 0usize;
	while first + 1 < lines && tl.src_le[first] < es.caret {
		first += 1;
	}
	let scan_end = (first + 2).min(lines);
	for line in first.saturating_sub(1)..scan_end {
		let Some(shaped) = shaper.line(tl, line) else {
			continue;
		};
		if shaped.clusters.is_empty() {
			step.visit(tl.src_ls[line], es.caret, delta);
			continue;
		}
		let base = tl.src_ls[line];
		let clamp = tl.src_le[line];
		let mut edges: Vec<i32> = shaped
			.clusters
			.iter()
			.flat_map(|cluster| {
				[cluster.start.wrapping_add(base).min(clamp), cluster.end.wrapping_add(base).min(clamp)]
			})
			.collect();
		edges.sort_unstable();
		edges.dedup();
		for edge in edges {
			step.visit(edge, es.caret, delta);
		}
		if step.done {
			break;
		}
	}
	let target = if step.done || delta < 0 {
		step.target
	} else if step.found {
		step.previous.unwrap_or(es.caret)
	} else {
		// A caret inside a cluster never matches an edge; the full stream
		// would land on the layout's final boundary.
		doc_tail_boundary(shaper, tl).unwrap_or(es.caret)
	};
	es.caret = target;
	if !select {
		es.anchor = es.caret;
	}
}

/// Moves vertically by visual lines while preserving the desired x position.
pub(crate) fn visual_move(
	shaper: textm::Shaper<'_>,
	es: &mut EditState,
	tl: &TextLayout,
	delta: i32,
	select: bool,
) {
	history_barrier(es);
	let line = visual_line(tl, es.caret);
	if es.goal_x < 0.0 {
		let line_index = usize::try_from(line).expect("negative visual line");
		es.goal_x = textm::caret_x(shaper, tl, line_index, es.caret);
	}

	let target = line.wrapping_add(delta);
	let line_count = i32::try_from(tl.src_ls.len()).expect("too many visual lines");
	// Past the first/last visual line the caret and selection stay put and
	// the computed goal_x is retained, so the unchanged command bubbles to
	// `keys=` for cross-field navigation (SPEC §15.4).
	if target < 0 || target >= line_count {
		return;
	}
	es.caret = caret_for_x(shaper, tl, target, es.goal_x);
	if !select {
		es.anchor = es.caret;
	}
}

/// Moves the caret to the start of its visual line.
pub(crate) fn visual_home(
	shaper: textm::Shaper<'_>,
	es: &mut EditState,
	tl: &TextLayout,
	select: bool,
) {
	history_barrier(es);
	let line = usize::try_from(visual_line(tl, es.caret)).expect("negative visual line");
	es.caret = textm::caret_for_visual_x(shaper, tl, line, f64::NEG_INFINITY);
	es.goal_x = NO_GOAL_X;
	if !select {
		es.anchor = es.caret;
	}
}

/// Moves the caret to the end of its visual line.
pub(crate) fn visual_end(
	shaper: textm::Shaper<'_>,
	es: &mut EditState,
	tl: &TextLayout,
	select: bool,
) {
	history_barrier(es);
	let line = usize::try_from(visual_line(tl, es.caret)).expect("negative visual line");
	es.caret = textm::caret_for_visual_x(shaper, tl, line, f64::INFINITY);
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
	history_barrier(es);
	es.goal_x = NO_GOAL_X;
}

fn apply_history_step(es: &mut EditState, step: UndoStep) -> (UndoStep, bool) {
	let spans_changed = es.spans != step.spans;
	let (inverse_record, text_changed) = match step.record {
		UndoRecord::Splice { at, removed, inserted_len } => {
			let end = at.wrapping_add(inserted_len);
			let inverse_removed = es.text.slice_cps(at, end);
			let changed = inverse_removed != removed;
			let inverse_inserted_len =
				i32::try_from(removed.len()).expect("removed text has too many codepoints");
			es.accumulate_delta(crate::textm::TextDelta {
				at,
				removed: inserted_len,
				inserted: inverse_inserted_len,
			});
			es.text = text_with_reverse_splice(&es.text, at, &removed, inserted_len);
			(
				UndoRecord::Splice { at, removed: inverse_removed, inserted_len: inverse_inserted_len },
				changed,
			)
		},
		UndoRecord::Full(target) => {
			let changed = es.text != target;
			es.measure_delta = MeasureDelta::Full;
			(UndoRecord::Full(std::mem::replace(&mut es.text, target)), changed)
		},
	};
	let inverse_step = UndoStep {
		record:        inverse_record,
		spans:         std::mem::replace(&mut es.spans, step.spans),
		field_styles:  std::mem::replace(&mut es.field_styles, step.field_styles),
		caret:         std::mem::replace(&mut es.caret, step.caret),
		anchor:        std::mem::replace(&mut es.anchor, step.anchor),
		text_recorded: step.text_recorded,
	};
	(inverse_step, text_changed || spans_changed)
}

/// Restores the most recent undo record.
pub fn undo(es: &mut EditState) -> bool {
	let Some(step) = es.undo.pop() else {
		return false;
	};
	let (inverse, changed) = apply_history_step(es, step);
	trim_redo(es);
	es.redo.push(inverse);
	if changed {
		es.revision = es.revision.wrapping_add(1);
	}
	finish_history_transition(es);
	changed
}

/// Reapplies the most recent redo record.
pub fn redo(es: &mut EditState) -> bool {
	let Some(step) = es.redo.pop() else {
		return false;
	};
	let (inverse, changed) = apply_history_step(es, step);
	trim_undo(es);
	es.undo.push(inverse);
	if changed {
		es.revision = es.revision.wrapping_add(1);
	}
	finish_history_transition(es);
	changed
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
	// The synced display now carries the preedit; splice lineage is broken
	// until a non-composing sync re-baselines.
	es.display_dirty = true;
	committed_changed
}

/// Ends composition and inserts non-empty committed composition text.
pub fn composition_end(es: &mut EditState, text: &str) -> bool {
	es.compose.clear();
	es.compose_clauses.clear();
	es.composing = false;
	// The previously synced display carried the preedit; committed-splice
	// lineage is broken until the next sync re-baselines.
	es.display_dirty = true;
	if text.is_empty() {
		return false;
	}
	history_barrier(es);
	insert(es, text)
}
