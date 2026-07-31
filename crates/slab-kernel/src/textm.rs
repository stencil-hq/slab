//! Text measurement using only SLIR font tables without host metrics.
//!
//! Wrapping preserves the original `VectorMetrics` contract for Latin and
//! space-delimited text: spaces split words (including empty words),
//! non-breaking spaces remain glued, overlong words hard-break at grapheme
//! boundaries, and ellipsis truncation strips trailing whitespace. UAX #14
//! opportunities additionally wrap scripts such as CJK without permitting
//! line-initial closing punctuation. Complex-context (SA-class) scripts such
//! as Thai, Lao, and Khmer deliberately use grapheme-cluster fallback rather
//! than dictionary segmentation. These details are part of the conformance
//! contract.
//!
//! Advances come from the selected font table as
//! `advance(glyph) * size / units_per_em + tracking`, with the default advance
//! used for painting codepoints missing from the cmap. Joiners and variation
//! selectors consume neither advance nor tracking. Font weight is represented
//! by a real table rather than an artificial width multiplier.
//!
//! Ascent uses CSS half-leading over the hhea box:
//! `ascent * size / units_per_em + (line_height - (ascent - descent) * size /
//! units_per_em) / 2`. This keeps kernel baselines aligned with browser-painted
//! glyphs.

use std::{cell::RefCell, fmt, hash::Hasher, rc::Rc, sync::Arc};

use rustc_hash::{FxHashMap, FxHasher};
use unicode_linebreak::{BreakClass, BreakOpportunity, break_property, linebreaks};

use crate::{
	graphemes,
	slir::{self, Doc},
};

/// Unicode codepoint inserted when a line is ellipsized.
pub const ELLIPSIS: u32 = 0x2026;
/// Unicode non-breaking-space codepoint.
pub const NBSP: u32 = 0xa0;
const WIDTH_EPSILON: f64 = 1e-6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ShapePlanKey {
	font:   i32,
	script: u32,
	rtl:    bool,
}

const SHAPED_LINE_CACHE_LIMIT: usize = 4096;

/// Wrap-memo generations grow to twice the largest hard-line sweep, within
/// these bounds, so one keystroke in a large field never evicts the sweep
/// that the next keystroke replays.
const WRAP_MEMO_FLOOR: usize = 4096;
const WRAP_MEMO_CEIL: usize = 1 << 18;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ShapedLineKey {
	font:     i32,
	size:     u64,
	tracking: u64,
	hash:     u64,
	len:      u32,
}

#[derive(Clone, Debug)]
struct CachedShapedLine {
	chars: Rc<[u32]>,
	line:  Rc<ShapedLine>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct WrapMemoKey {
	font:       i32,
	size:       u64,
	tracking:   u64,
	max_w:      u64,
	wrap:       bool,
	hash:       u64,
	len:        u32,
	/// `FxHash` of the hard line's rebased span signature; zero when plain.
	spans_hash: u64,
}

/// One style range rebased to a hard line: `(style, start, end)`.
type SpanSig = (u8, i32, i32);

/// Measured output of one hard line: line ranges relative to the segment
/// start, source ranges relative to the hard-line start, and normative
/// advance widths. Paint geometry is never memoized; it shapes lazily.
#[derive(Clone, Debug)]
struct WrapMemoEntry {
	/// Exact source codepoints; hits validate content, never just the hash.
	chars:     Rc<[u32]>,
	/// Exact rebased span signature; hits validate it alongside `chars`.
	spans_sig: Rc<[SpanSig]>,
	out_chars: Rc<[u32]>,
	ls:        Rc<[i32]>,
	le:        Rc<[i32]>,
	src_ls:    Rc<[i32]>,
	src_le:    Rc<[i32]>,
	line_w:    Rc<[f64]>,
}

struct CachedFace {
	face:  rustybuzz::Face<'static>,
	_data: Box<[u8]>,
}

impl CachedFace {
	fn new(data: &[u8], weight: Option<u32>) -> Option<Self> {
		let data = Box::<[u8]>::from(data);
		let mut face = rustybuzz::Face::from_slice(&data, 0)?;
		if let Some(weight) = weight {
			face.set_variations(&[rustybuzz::Variation {
				tag:   rustybuzz::ttf_parser::Tag::from_bytes(b"wght"),
				value: f32::from(u16::try_from(weight).expect("font weight exceeds u16")),
			}]);
		}
		// SAFETY: the face borrows the boxed allocation stored beside it. Moving
		// the box preserves that allocation, and `face` is dropped before `_data`.
		let face =
			unsafe { std::mem::transmute::<rustybuzz::Face<'_>, rustybuzz::Face<'static>>(face) };
		Some(Self { face, _data: data })
	}
}

/// Retains parsed font faces, expensive OpenType plans, immutable shaped
/// lines, and per-hard-line wrap results across frames.
#[derive(Default)]
pub(crate) struct ShapeCache {
	plans:      FxHashMap<ShapePlanKey, Arc<rustybuzz::ShapePlan>>,
	faces:      Vec<Option<CachedFace>>,
	lines:      FxHashMap<ShapedLineKey, CachedShapedLine>,
	lines_cold: FxHashMap<ShapedLineKey, CachedShapedLine>,
	wraps:      FxHashMap<WrapMemoKey, WrapMemoEntry>,
	wraps_cold: FxHashMap<WrapMemoKey, WrapMemoEntry>,
	wrap_cap:   usize,
	buffer:     Option<rustybuzz::UnicodeBuffer>,
}

impl Clone for ShapeCache {
	fn clone(&self) -> Self {
		Self {
			plans:      self.plans.clone(),
			faces:      Vec::new(),
			lines:      self.lines.clone(),
			lines_cold: self.lines_cold.clone(),
			wraps:      self.wraps.clone(),
			wraps_cold: self.wraps_cold.clone(),
			wrap_cap:   self.wrap_cap,
			buffer:     None,
		}
	}
}

impl fmt::Debug for ShapeCache {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("ShapeCache")
			.field("plans", &self.plans.len())
			.field("lines", &self.lines.len())
			.field("faces", &self.faces.iter().flatten().count())
			.field("lines_cold", &self.lines_cold.len())
			.field("wraps", &self.wraps.len())
			.field("wraps_cold", &self.wraps_cold.len())
			.finish()
	}
}

impl ShapeCache {
	fn face_and_plan(
		&mut self,
		font: i32,
		weight: Option<u32>,
		data: &[u8],
		rtl: bool,
		script: rustybuzz::Script,
	) -> Option<(&rustybuzz::Face<'static>, Arc<rustybuzz::ShapePlan>)> {
		let index = usize::try_from(font).ok()?;
		if self.faces.len() <= index {
			self.faces.resize_with(index + 1, || None);
		}
		if self.faces[index].is_none() {
			self.faces[index] = CachedFace::new(data, weight);
		}
		let Self { plans, faces, .. } = self;
		let face = &faces[index].as_ref()?.face;
		let key = ShapePlanKey { font, script: script.tag().0, rtl };
		let plan = plans
			.entry(key)
			.or_insert_with(|| {
				Arc::new(rustybuzz::ShapePlan::new(
					face,
					if rtl {
						rustybuzz::Direction::RightToLeft
					} else {
						rustybuzz::Direction::LeftToRight
					},
					Some(script),
					None,
					&[],
				))
			})
			.clone();
		Some((face, plan))
	}

	fn take_buffer(&mut self) -> rustybuzz::UnicodeBuffer {
		self.buffer.take().unwrap_or_default()
	}

	fn recycle_buffer(&mut self, buffer: rustybuzz::UnicodeBuffer) {
		self.buffer = Some(buffer);
	}

	fn line_key(font: i32, size: f64, tracking: f64, chars: &[u32]) -> ShapedLineKey {
		let mut hasher = FxHasher::default();
		for &codepoint in chars {
			hasher.write_u32(codepoint);
		}
		ShapedLineKey {
			font,
			size: size.to_bits(),
			tracking: tracking.to_bits(),
			hash: hasher.finish(),
			len: u32::try_from(chars.len()).expect("text exceeds u32"),
		}
	}

	fn line(&mut self, key: ShapedLineKey, chars: &[u32]) -> Option<Rc<ShapedLine>> {
		if let Some(entry) = self.lines.get(&key)
			&& entry.chars.as_ref() == chars
		{
			return Some(Rc::clone(&entry.line));
		}
		if let Some(entry) = self.lines_cold.remove(&key)
			&& entry.chars.as_ref() == chars
		{
			let line = Rc::clone(&entry.line);
			self.insert_entry(key, entry);
			return Some(line);
		}
		None
	}

	fn insert_entry(&mut self, key: ShapedLineKey, entry: CachedShapedLine) {
		if self.lines.len() >= SHAPED_LINE_CACHE_LIMIT && !self.lines.contains_key(&key) {
			std::mem::swap(&mut self.lines, &mut self.lines_cold);
			self.lines.clear();
		}
		self.lines.insert(key, entry);
	}

	fn insert_line(&mut self, key: ShapedLineKey, chars: &[u32], line: Rc<ShapedLine>) {
		self.insert_entry(key, CachedShapedLine { chars: Rc::from(chars), line });
	}

	fn wrap_key(
		font: i32,
		size: f64,
		tracking: f64,
		max_w: f64,
		wrap: bool,
		chars: &[u32],
		spans_sig: &[SpanSig],
	) -> WrapMemoKey {
		let mut hasher = FxHasher::default();
		for &codepoint in chars {
			hasher.write_u32(codepoint);
		}
		let spans_hash = if spans_sig.is_empty() {
			0
		} else {
			let mut span_hasher = FxHasher::default();
			for &(style, start, end) in spans_sig {
				span_hasher.write_u8(style);
				span_hasher.write_i32(start);
				span_hasher.write_i32(end);
			}
			span_hasher.finish()
		};
		WrapMemoKey {
			font,
			size: size.to_bits(),
			tracking: tracking.to_bits(),
			max_w: max_w.to_bits(),
			wrap,
			hash: hasher.finish(),
			len: u32::try_from(chars.len()).expect("text exceeds u32"),
			spans_hash,
		}
	}

	/// Grows the wrap-memo budget to hold `sweep` hard lines in both
	/// generations; the budget never shrinks while a document is loaded.
	fn ensure_wrap_cap(&mut self, sweep: usize) {
		let want = sweep
			.saturating_mul(2)
			.clamp(WRAP_MEMO_FLOOR, WRAP_MEMO_CEIL);
		self.wrap_cap = self.wrap_cap.max(want);
	}

	fn wrap_get(
		&mut self,
		key: WrapMemoKey,
		chars: &[u32],
		spans_sig: &[SpanSig],
	) -> Option<WrapMemoEntry> {
		if let Some(entry) = self.wraps.get(&key)
			&& entry.chars.as_ref() == chars
			&& entry.spans_sig.as_ref() == spans_sig
		{
			return Some(entry.clone());
		}
		if let Some(entry) = self.wraps_cold.remove(&key)
			&& entry.chars.as_ref() == chars
			&& entry.spans_sig.as_ref() == spans_sig
		{
			self.wrap_insert(key, entry.clone());
			return Some(entry);
		}
		None
	}

	fn wrap_insert(&mut self, key: WrapMemoKey, entry: WrapMemoEntry) {
		if self.wraps.len() >= self.wrap_cap.max(WRAP_MEMO_FLOOR) && !self.wraps.contains_key(&key) {
			std::mem::swap(&mut self.wraps, &mut self.wraps_cold);
			self.wraps.clear();
		}
		self.wraps.insert(key, entry);
	}

	/// Drops retained data tied to the previously initialized document.
	pub(crate) fn clear(&mut self) {
		self.plans.clear();
		self.faces.clear();
		self.lines.clear();
		self.lines_cold.clear();
		self.wraps.clear();
		self.wraps_cold.clear();
		self.wrap_cap = 0;
	}
}

/// One glyph emitted by OpenType shaping, positioned relative to its visual
/// line.
#[derive(Clone, Debug)]
pub struct ShapedGlyph {
	pub font:    i32,
	pub gid:     u32,
	/// Line-local source codepoint offset; rebase with the line's
	/// [`TextLayout::src_ls`] entry for field-source coordinates.
	pub cluster: i32,
	pub x:       f64,
	pub y:       f64,
}

/// One visually contiguous font and direction run.
///
/// Offsets are line-local: rebase `start`/`end` with the line's
/// [`TextLayout::ls`] entry to slice [`TextLayout::chars`].
#[derive(Clone, Debug)]
pub struct ShapedRun {
	/// Line-local slice bounds in logical order.
	pub start:       i32,
	pub end:         i32,
	pub font:        i32,
	/// Bitset of `1 << edit::STYLE_*` inline overrides.
	pub style:       u32,
	/// Index into the field's paint-only style list, or `-1`.
	pub field_style: i32,
	pub rtl:         bool,
	pub x:           f64,
	pub width:       f64,
	pub glyphs:      Vec<ShapedGlyph>,
}

/// A shaped caret unit with visual geometry and logical source bounds.
///
/// `start`/`end` are line-local: rebase with the line's
/// [`TextLayout::src_ls`] entry and clamp to its [`TextLayout::src_le`]
/// entry for field-source coordinates.
#[derive(Clone, Debug)]
pub struct ShapedCluster {
	pub start: i32,
	pub end:   i32,
	pub x0:    f64,
	pub x1:    f64,
	pub rtl:   bool,
}

/// Shaped visual runs and caret clusters for one output line.
#[derive(Clone, Debug, Default)]
pub struct ShapedLine {
	pub runs:     Vec<ShapedRun>,
	pub clusters: Vec<ShapedCluster>,
	pub width:    f64,
}

/// A measured and optionally wrapped text layout.
///
/// Output lines are slices `chars[ls[i]..le[i]]`. `src_ls` and `src_le` hold
/// the corresponding codepoint offsets in the original string. They remain
/// source offsets when wrapping normalizes the output or ellipsis synthesizes
/// characters.
///
/// Measurement (`line_w`, `w`, wrapping) is FRAME.md-normative advance
/// arithmetic and never shapes. OpenType paint geometry per line fills
/// lazily through [`line_shaped`] for plain and rich lines alike.
#[derive(Clone, Debug)]
pub struct TextLayout {
	/// Codepoints for all output lines.
	pub chars:      Vec<u32>,
	/// Inclusive output start offset for each line.
	pub ls:         Vec<i32>,
	/// Exclusive output end offset for each line.
	pub le:         Vec<i32>,
	/// Inclusive original-text codepoint offset for each line.
	pub src_ls:     Vec<i32>,
	/// Exclusive original-text codepoint offset for each line.
	pub src_le:     Vec<i32>,
	/// Measured advance for each line (FONT-table advances, not shaping).
	pub line_w:     Vec<f64>,
	/// Lazily shaped paint geometry, parallel to `line_w`; access through
	/// [`line_shaped`], never directly. Plain lines hold weak handles so the
	/// bounded [`ShapeCache`] owns retention; rich lines pin their shapes.
	pub shaped:     RefCell<Vec<LineShape>>,
	/// First output-line index of each hard (newline-delimited) line; filled
	/// by the memo measure path and empty elsewhere. Powers delta splicing.
	pub hard_lines: Vec<i32>,
	/// Source codepoint offset where each hard line starts, parallel to
	/// `hard_lines`.
	pub hard_src:   Vec<i32>,
	/// Rich spans this layout was measured with; empty for plain text.
	pub spans:      crate::edit::InlineSpans,
	/// FIFO of currently pinned rich line indices; bounded by
	/// [`RICH_PIN_CAP`] so sweeping a rich document cannot pin every line.
	pub rich_pins:  RefCell<std::collections::VecDeque<usize>>,
	/// Primary font this layout was measured with, for lazy shaping.
	pub font:       i32,
	/// Font size this layout was measured with.
	pub size:       f64,
	/// Letter spacing this layout was measured with.
	pub tracking:   f64,
	/// Maximum measured line width.
	pub w:          f64,
	/// Total layout height.
	pub h:          f64,
	/// First-baseline offset from a line's top.
	pub ascent:     f64,
	/// Height of one line.
	pub line_h:     f64,
	/// Whether line or width limits removed or replaced any text.
	pub truncated:  bool,
}

/// Maximum rich lines one layout pins before evicting the oldest.
pub const RICH_PIN_CAP: usize = 1024;

/// Retention state of one line's paint geometry.
///
/// Plain text downgrades to [`Weak`] so a full-document scroll sweep cannot
/// pin every shaped line; the two-generation [`ShapeCache`] decides what
/// survives. Rich spans shape outside that cache and stay pinned until the
/// layout is replaced or the pin FIFO evicts them.
#[derive(Clone, Debug, Default)]
pub enum LineShape {
	/// Never shaped, or evicted and re-shapeable on demand.
	#[default]
	Unshaped,
	/// Plain line owned by the shape cache.
	Plain(std::rc::Weak<ShapedLine>),
	/// Rich line pinned by this layout.
	Rich(Rc<ShapedLine>),
}

/// A cached [`measure_text`] result with the exact inputs that produced it.
///
/// Layout stores one entry per text node and treats an entry as valid only
/// when every input matches, so style or content changes re-measure.
#[derive(Clone, Debug)]
pub struct TextCacheEntry {
	pub font:        i32,
	/// Bit patterns of the size, leading, tracking, and width budget.
	pub size:        u64,
	pub leading:     u64,
	pub tracking:    u64,
	pub max_w:       u64,
	pub wrap:        bool,
	pub ellipsis:    bool,
	pub max_lines:   i32,
	/// Rich-field spans that participated in this layout.
	pub spans:       crate::edit::InlineSpans,
	/// Resolved text content at measurement time.
	pub content:     crate::text::Text,
	/// [`crate::style::FieldTextRev`] revision this content was measured at;
	/// zero when the node's content is not revision-tracked.
	pub content_rev: u64,
	pub layout:      std::rc::Rc<TextLayout>,
}

/// Extracts one hard line's rebased span signature into `out`.
///
/// Ranges are clipped to `[hs, he)` and rebased to the hard-line start, so
/// identical styling produces identical signatures wherever the line sits.
fn span_sig(spans: &crate::edit::InlineSpans, hs: i32, he: i32, out: &mut Vec<SpanSig>) {
	out.clear();
	if spans.is_empty() {
		return;
	}
	for style in 0..=crate::edit::STYLE_CODE {
		let Some(ranges) = spans.get(style) else {
			continue;
		};
		// Normalized ranges are sorted and disjoint: lower-bound to the
		// first range touching the hard line, stop at the first past it.
		let from = ranges.0.partition_point(|&(_, end)| end <= hs);
		for &(start, end) in &ranges.0[from..] {
			if start >= he {
				break;
			}
			out.push((
				u8::try_from(style).expect("style id exceeds u8"),
				start.max(hs).wrapping_sub(hs),
				end.min(he).wrapping_sub(hs),
			));
		}
	}
}

/// Creates an empty text layout.
pub const fn tl_new() -> TextLayout {
	TextLayout {
		chars:      Vec::new(),
		ls:         Vec::new(),
		le:         Vec::new(),
		src_ls:     Vec::new(),
		src_le:     Vec::new(),
		line_w:     Vec::new(),
		shaped:     RefCell::new(Vec::new()),
		hard_lines: Vec::new(),
		hard_src:   Vec::new(),
		spans:      crate::edit::InlineSpans::empty(),
		rich_pins:  RefCell::new(std::collections::VecDeque::new()),
		font:       -1,
		size:       0.0,
		tracking:   0.0,
		w:          0.0,
		h:          0.0,
		ascent:     0.0,
		line_h:     0.0,
		truncated:  false,
	}
}

/// Returns the shaped paint geometry for one line, shaping it on first
/// access. `None` only for out-of-range lines.
///
/// Plain lines shape through the retained line cache and are held weakly,
/// so scrolling re-uses geometry across frames while the cache's bounded
/// generations decide retention.
pub(crate) fn line_shaped(
	d: &Doc,
	cache: &RefCell<ShapeCache>,
	layout: &TextLayout,
	line: usize,
) -> Option<Rc<ShapedLine>> {
	{
		let shaped = layout.shaped.borrow();
		match shaped.get(line) {
			None => return None,
			Some(LineShape::Rich(existing)) => return Some(Rc::clone(existing)),
			Some(LineShape::Plain(weak)) => {
				if let Some(existing) = weak.upgrade() {
					return Some(existing);
				}
			},
			Some(LineShape::Unshaped) => {},
		}
	}
	let start = usize::try_from(layout.ls[line]).expect("negative line start");
	let end = usize::try_from(layout.le[line]).expect("negative line end");
	if layout.spans.is_empty() {
		let shared = shape_line_shared_cached(
			d,
			layout.font,
			layout.size,
			layout.tracking,
			&layout.chars[start..end],
			&mut cache.borrow_mut(),
		);
		layout.shaped.borrow_mut()[line] = LineShape::Plain(Rc::downgrade(&shared));
		return Some(shared);
	}
	// Rich lines shape outside the shared cache (their geometry depends on
	// the span masks); a bounded FIFO caps how many stay pinned.
	let shaped = Rc::new(shape_rich_line(
		d,
		layout.font,
		layout.size,
		layout.tracking,
		&layout.chars[start..end],
		layout.src_ls[line],
		layout.src_le[line],
		&layout.spans,
		&[],
		&mut cache.borrow_mut(),
	));
	{
		let mut store = layout.shaped.borrow_mut();
		let mut pins = layout.rich_pins.borrow_mut();
		if pins.len() >= RICH_PIN_CAP
			&& let Some(evicted) = pins.pop_front()
			&& evicted != line
		{
			store[evicted] = LineShape::Unshaped;
		}
		store[line] = LineShape::Rich(Rc::clone(&shaped));
		pins.push_back(line);
	}
	Some(shaped)
}

/// Returns the number of output lines.
pub fn line_count(tl: &TextLayout) -> i32 {
	i32::try_from(tl.ls.len()).expect("text line count exceeds i32")
}

/// Returns one codepoint's advance, including letter spacing after the glyph,
/// matching CSS letter-spacing semantics.
///
/// A codepoint the cmap does not cover charges a deterministic fallback
/// advance: the font's default advance, doubled for East-Asian-Width wide
/// codepoints in mono-class families so uncovered CJK and emoji reserve the
/// two terminal cells the grid gives them. Vector families keep the single
/// replacement advance. Fallback painters fill exactly this charged width.
pub fn char_w(d: &Doc, f: i32, size: f64, tracking: f64, cp: u32) -> f64 {
	if graphemes::is_glyph_modifier(cp) {
		return 0.0;
	}
	if f < 0 {
		return 0.6f64.mul_add(size, tracking);
	}

	let font = usize::try_from(f).expect("nonnegative font index");
	let upem = f64::from(d.font_upem.get(font).copied().unwrap_or(0));
	if upem <= 0.0 {
		return 0.6f64.mul_add(size, tracking);
	}

	// Metrics-only fonts (e.g. runtime registrations without a cmap) charge
	// the deterministic fallback advance for every codepoint.
	let cmap_index = if d.font_cmap_len.get(font).copied().unwrap_or(0) > 0 {
		slir::font_cmap_ix(d, f, cp)
	} else {
		-1
	};
	if cmap_index < 0 {
		let mut advance = f64::from(d.font_default_adv.get(font).copied().unwrap_or(0));
		if d.font_class.get(font).copied().unwrap_or(0) == 1 && graphemes::cp_wide(cp) {
			advance *= 2.0;
		}
		return advance * size / upem + tracking;
	}
	let cmap_index = usize::try_from(cmap_index).expect("nonnegative cmap index");
	f64::from(d.font_adv[cmap_index]) * size / upem + tracking
}

fn font_covers(d: &Doc, font: i32, chars: &[u32]) -> bool {
	font >= 0
		&& chars.iter().all(|&codepoint| {
			!graphemes::requires_glyph(codepoint) || slir::font_gid(d, font, codepoint) != 0
		})
}

/// Whether `font` publishes a cmap the kernel can check coverage against.
///
/// A runtime registration may carry metrics only (no cmap), which declares no
/// coverage: FRAME.md makes the selected font's cmap authoritative, so such a
/// font keeps every cluster and reports them as uncovered runs for driver-side
/// fallback paint instead of being swapped for another family.
fn coverage_known(d: &Doc, font: i32) -> bool {
	usize::try_from(font)
		.ok()
		.and_then(|index| d.font_cmap_len.get(index))
		.is_some_and(|&len| len > 0)
}

/// Whether `chars` stay on `primary` — either it proves coverage or it
/// publishes none for the kernel to fall back from.
fn keeps_primary(d: &Doc, primary: i32, chars: &[u32]) -> bool {
	primary >= 0 && (!coverage_known(d, primary) || font_covers(d, primary, chars))
}

fn fallback_font(d: &Doc, primary: i32, chars: &[u32]) -> i32 {
	if keeps_primary(d, primary, chars) {
		return primary;
	}
	for font in (0..d.font_upem.len()).rev() {
		let font = i32::try_from(font).expect("font table exceeds i32");
		if font != primary && coverage_known(d, font) && font_covers(d, font, chars) {
			return font;
		}
	}
	primary
}

fn font_assignments(d: &Doc, primary: i32, text: &str, chars: &[u32]) -> Vec<i32> {
	let mut assigned = vec![primary; chars.len()];
	if keeps_primary(d, primary, chars) {
		return assigned;
	}
	let mut boundaries = Vec::new();
	graphemes::boundaries(text, &mut boundaries);
	for pair in boundaries.windows(2) {
		let start = usize::try_from(pair[0]).expect("negative grapheme boundary");
		let end = usize::try_from(pair[1]).expect("negative grapheme boundary");
		let font = fallback_font(d, primary, &chars[start..end]);
		if font_covers(d, font, &chars[start..end]) {
			assigned[start..end].fill(font);
		} else {
			for index in start..end {
				assigned[index] = fallback_font(d, primary, &chars[index..=index]);
			}
		}
	}
	assigned
}

fn push_cluster(
	clusters: &mut Vec<ShapedCluster>,
	start: i32,
	end: i32,
	x0: f64,
	x1: f64,
	rtl: bool,
) {
	clusters.push(ShapedCluster { start, end, x0, x1, rtl });
}

/// Shapes one font run; all emitted offsets are line-local (zero-based).
fn shape_font_run(
	d: &Doc,
	chars: &[u32],
	start: usize,
	end: usize,
	font: i32,
	size: f64,
	tracking: f64,
	rtl: bool,
	line_x: f64,
	cache: &mut ShapeCache,
) -> (ShapedRun, Vec<ShapedCluster>) {
	let run_start = i32::try_from(start).expect("text exceeds i32");
	let run_end = i32::try_from(end).expect("text exceeds i32");
	let mut glyphs = Vec::new();
	let mut clusters = Vec::new();
	let data = slir::face_data(d, font);
	let upem = usize::try_from(font)
		.ok()
		.and_then(|index| d.font_upem.get(index))
		.copied()
		.unwrap_or(0);
	if upem > 0 {
		let weight = usize::try_from(font)
			.ok()
			.and_then(|index| d.font_weight.get(index))
			.copied();
		let mut buffer = cache.take_buffer();
		for (local, &codepoint) in chars[start..end].iter().enumerate() {
			buffer.add(
				char::from_u32(codepoint).unwrap_or(char::REPLACEMENT_CHARACTER),
				u32::try_from(run_start.wrapping_add(i32::try_from(local).expect("text exceeds i32")))
					.expect("negative shape cluster"),
			);
		}
		buffer.set_cluster_level(rustybuzz::BufferClusterLevel::MonotoneGraphemes);
		buffer.set_direction(if rtl {
			rustybuzz::Direction::RightToLeft
		} else {
			rustybuzz::Direction::LeftToRight
		});
		buffer.guess_segment_properties();
		if let Some((face, plan)) = cache.face_and_plan(font, weight, data, rtl, buffer.script()) {
			let shaped = rustybuzz::shape_with_plan(face, plan.as_ref(), buffer);
			let infos = shaped.glyph_infos();
			let positions = shaped.glyph_positions();
			glyphs.reserve(infos.len());
			clusters.reserve(infos.len());
			let scale = size / f64::from(upem);
			let mut cursor = line_x;
			let mut group_cluster = None;
			let mut group_x = cursor;
			let mut groups = Vec::with_capacity(infos.len());
			for (index, (info, position)) in infos.iter().zip(positions).enumerate() {
				let cluster = i32::try_from(info.cluster).expect("shape cluster exceeds i32");
				if group_cluster.is_some_and(|current| current != cluster) {
					cursor += tracking;
					groups.push((group_cluster.expect("shape cluster"), group_x, cursor));
					group_x = cursor;
				}
				group_cluster = Some(cluster);
				glyphs.push(ShapedGlyph {
					font,
					gid: info.glyph_id,
					cluster,
					x: f64::from(position.x_offset).mul_add(scale, cursor),
					y: -f64::from(position.y_offset) * scale,
				});
				cursor = f64::from(position.x_advance).mul_add(scale, cursor);
				if index + 1 == infos.len() {
					cursor += tracking;
					groups.push((cluster, group_x, cursor));
				}
			}
			let mut logical_starts: Vec<i32> = groups.iter().map(|group| group.0).collect();
			logical_starts.sort_unstable();
			logical_starts.dedup();
			for (cluster, x0, x1) in groups {
				let logical = logical_starts
					.binary_search(&cluster)
					.expect("shape cluster is in logical starts");
				let end = logical_starts.get(logical + 1).copied().unwrap_or(run_end);
				push_cluster(&mut clusters, cluster, end, x0, x1, rtl);
			}
			let run = ShapedRun {
				start: run_start,
				end: run_end,
				font,
				style: 0,
				field_style: -1,
				rtl,
				x: line_x,
				width: cursor - line_x,
				glyphs,
			};
			cache.recycle_buffer(shaped.clear());
			return (run, clusters);
		}
		buffer.clear();
		cache.recycle_buffer(buffer);
	}

	let text: String = chars[start..end]
		.iter()
		.map(|&codepoint| char::from_u32(codepoint).expect("valid codepoint"))
		.collect();
	let mut boundaries = Vec::new();
	graphemes::boundaries(&text, &mut boundaries);
	let ranges: Box<dyn Iterator<Item = &[i32]>> = if rtl {
		Box::new(boundaries.windows(2).rev())
	} else {
		Box::new(boundaries.windows(2))
	};
	let mut cursor = line_x;
	for pair in ranges {
		let cluster_start = start + usize::try_from(pair[0]).expect("negative grapheme boundary");
		let cluster_end = start + usize::try_from(pair[1]).expect("negative grapheme boundary");
		let cluster = i32::try_from(cluster_start).expect("text exceeds i32");
		let x0 = cursor;
		for &codepoint in &chars[cluster_start..cluster_end] {
			let width = char_w(d, font, size, tracking, codepoint);
			glyphs.push(ShapedGlyph {
				font,
				gid: if font < 0 || graphemes::is_glyph_modifier(codepoint) {
					0
				} else {
					slir::font_gid(d, font, codepoint)
				},
				cluster,
				x: cursor,
				y: 0.0,
			});
			cursor += width;
		}
		push_cluster(
			&mut clusters,
			cluster,
			i32::try_from(cluster_end).expect("text exceeds i32"),
			x0,
			cursor,
			rtl,
		);
	}
	(
		ShapedRun {
			start: run_start,
			end: run_end,
			font,
			style: 0,
			field_style: -1,
			rtl,
			x: line_x,
			width: cursor - line_x,
			glyphs,
		},
		clusters,
	)
}

/// Sequential prefix folds of the normative per-codepoint advances.
///
/// `select` charges each local codepoint to its spec-selected font. The
/// result has `chars.len() + 1` entries; entry `i` is the width of the
/// first `i` codepoints, bit-identical to the wrap fold over the same
/// sequence.
fn advance_positions(
	d: &Doc,
	size: f64,
	tracking: f64,
	chars: &[u32],
	mut select: impl FnMut(usize) -> i32,
) -> Vec<f64> {
	let mut pos = Vec::with_capacity(chars.len() + 1);
	let mut width = 0.0;
	pos.push(width);
	for (local, &cp) in chars.iter().enumerate() {
		width += char_w(d, select(local), size, tracking, cp);
		pos.push(width);
	}
	pos
}

/// Rebases one shaped run into FRAME.md-normative advance space and splits
/// merged clusters back to grapheme grain.
///
/// Cluster and run x-extents become prefix-fold differences — the same
/// space measurement, wrapping, alignment, and scrolling use. Ligatures may
/// merge several graphemes into one shaped cluster; splitting them at
/// grapheme boundaries keeps every caret stop addressable, with positions
/// from the normative folds (RTL mirrored inside the run box). Glyphs shift
/// with their original cluster, preserving intra-cluster shaping. Returns
/// the grapheme-grain clusters and the run's advance width.
fn advance_normalize_run(
	run: &mut ShapedRun,
	clusters: Vec<ShapedCluster>,
	grapheme_starts: &[i32],
	pos: &[f64],
	run_x: f64,
) -> (Vec<ShapedCluster>, f64) {
	let rs = usize::try_from(run.start).expect("negative run start");
	let re = usize::try_from(run.end).expect("negative run end");
	let rtl = run.rtl;
	let width = pos[re] - pos[rs];
	let advance_x = move |from: usize, to: usize| {
		if rtl {
			(run_x + (pos[re] - pos[to]), run_x + (pos[re] - pos[from]))
		} else {
			(run_x + (pos[from] - pos[rs]), run_x + (pos[to] - pos[rs]))
		}
	};
	let mut out = Vec::with_capacity(clusters.len());
	let mut glyph = 0usize;
	for cluster in clusters {
		let cs = usize::try_from(cluster.start).expect("negative cluster start");
		let ce = usize::try_from(cluster.end).expect("negative cluster end");
		// Glyphs stay anchored to the merged extent so ligature glyph
		// placement survives the split.
		let (merged_x0, _) = advance_x(cs, ce);
		let shift = merged_x0 - cluster.x0;
		while glyph < run.glyphs.len() && run.glyphs[glyph].cluster == cluster.start {
			run.glyphs[glyph].x += shift;
			glyph += 1;
		}
		// Split at grapheme boundaries in logical order, then emit visually:
		// an RTL run paints (and navigates) reversed-logical, so its
		// subclusters must land in that order too.
		let mut subs: Vec<(i32, i32)> = Vec::new();
		let mut sub_start = cluster.start;
		for &boundary in grapheme_starts
			.iter()
			.filter(|&&boundary| boundary > cluster.start && boundary < cluster.end)
		{
			subs.push((sub_start, boundary));
			sub_start = boundary;
		}
		subs.push((sub_start, cluster.end));
		if cluster.rtl {
			subs.reverse();
		}
		for (start, end) in subs {
			push_advance_cluster(&mut out, start, end, cluster.rtl, &advance_x);
		}
	}
	run.x = run_x;
	run.width = width;
	(out, width)
}

fn push_advance_cluster(
	out: &mut Vec<ShapedCluster>,
	start: i32,
	end: i32,
	rtl: bool,
	advance_x: &impl Fn(usize, usize) -> (f64, f64),
) {
	let (x0, x1) = advance_x(
		usize::try_from(start).expect("negative cluster start"),
		usize::try_from(end).expect("negative cluster end"),
	);
	out.push(ShapedCluster { start, end, x0, x1, rtl });
}

/// Shapes and visually reorders one logical line.
///
/// All emitted run, glyph, and cluster offsets are line-local (zero-based);
/// consumers rebase them with the line's output and source starts.
pub fn shape_line(
	d: &Doc,
	primary_font: i32,
	size: f64,
	tracking: f64,
	chars: &[u32],
) -> ShapedLine {
	shape_line_uncached(d, primary_font, size, tracking, chars, &mut ShapeCache::default())
}

fn shape_line_uncached(
	d: &Doc,
	primary_font: i32,
	size: f64,
	tracking: f64,
	chars: &[u32],
	cache: &mut ShapeCache,
) -> ShapedLine {
	if chars.is_empty() {
		return ShapedLine::default();
	}
	let pos = advance_positions(d, size, tracking, chars, |_| primary_font);
	let text: String = chars
		.iter()
		.map(|&codepoint| char::from_u32(codepoint).unwrap_or(char::REPLACEMENT_CHARACTER))
		.collect();
	let mut grapheme_starts = Vec::new();
	graphemes::boundaries(&text, &mut grapheme_starts);
	if chars.iter().all(|&codepoint| codepoint <= 0x7f) && keeps_primary(d, primary_font, chars) {
		let (mut run, clusters) =
			shape_font_run(d, chars, 0, chars.len(), primary_font, size, tracking, false, 0.0, cache);
		let (clusters, _) = advance_normalize_run(&mut run, clusters, &grapheme_starts, &pos, 0.0);
		let width = pos[chars.len()];
		return ShapedLine { runs: vec![run], clusters, width };
	}
	let assigned = font_assignments(d, primary_font, &text, chars);
	let mut byte_offsets: Vec<usize> = text.char_indices().map(|(offset, _)| offset).collect();
	byte_offsets.push(text.len());
	let bidi = unicode_bidi::BidiInfo::new(&text, None);
	let Some(paragraph) = bidi.paragraphs.first() else {
		return ShapedLine::default();
	};
	let (levels, visual_runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
	let mut line = ShapedLine::default();
	for visual in visual_runs {
		let start = byte_offsets
			.binary_search(&visual.start)
			.expect("bidi run starts at a character boundary");
		let end = byte_offsets
			.binary_search(&visual.end)
			.expect("bidi run ends at a character boundary");
		let rtl = levels.get(visual.start).is_some_and(|level| level.is_rtl());
		let mut logical_runs = Vec::new();
		let mut run_start = start;
		while run_start < end {
			let font = assigned[run_start];
			let mut run_end = run_start + 1;
			while run_end < end && assigned[run_end] == font {
				run_end += 1;
			}
			logical_runs.push((run_start, run_end, font));
			run_start = run_end;
		}
		if rtl {
			logical_runs.reverse();
		}
		for (start, end, font) in logical_runs {
			let (mut run, clusters) =
				shape_font_run(d, chars, start, end, font, size, tracking, rtl, line.width, cache);
			let (mut clusters, run_w) =
				advance_normalize_run(&mut run, clusters, &grapheme_starts, &pos, line.width);
			line.width += run_w;
			line.runs.push(run);
			line.clusters.append(&mut clusters);
		}
	}
	line.width = pos[chars.len()];
	line
}

/// Returns immutable line-local shaped geometry, retained across frames.
pub(crate) fn shape_line_shared_cached(
	d: &Doc,
	primary_font: i32,
	size: f64,
	tracking: f64,
	chars: &[u32],
	cache: &mut ShapeCache,
) -> Rc<ShapedLine> {
	if chars.is_empty() {
		return Rc::new(ShapedLine::default());
	}
	let key = ShapeCache::line_key(primary_font, size, tracking, chars);
	if let Some(line) = cache.line(key, chars) {
		return line;
	}
	let line = Rc::new(shape_line_uncached(d, primary_font, size, tracking, chars, cache));
	cache.insert_line(key, chars, Rc::clone(&line));
	line
}

/// Borrowed shaping context for lazy paint-geometry access.
///
/// Bundles the document's font tables with the retained shape cache so
/// caret, selection, and paint consumers can fill [`TextLayout`] lines on
/// demand.
#[derive(Clone, Copy)]
pub(crate) struct Shaper<'a> {
	pub d:     &'a Doc,
	pub cache: &'a RefCell<ShapeCache>,
}

impl Shaper<'_> {
	/// Shaped paint geometry for one line; `None` only when out of range.
	pub(crate) fn line(&self, layout: &TextLayout, line: usize) -> Option<Rc<ShapedLine>> {
		line_shaped(self.d, self.cache, layout, line)
	}

	/// Shapes a field line with paint-only style boundaries.
	///
	/// This bypasses retained geometry because host ranges may change without
	/// invalidating measurement. The base layout and its advances are untouched.
	pub(crate) fn field_line(
		&self,
		layout: &TextLayout,
		line: usize,
		styles: &[crate::edit::FieldStyle],
	) -> Option<Rc<ShapedLine>> {
		let start = usize::try_from(*layout.ls.get(line)?).expect("negative line start");
		let end = usize::try_from(layout.le[line]).expect("negative line end");
		Some(Rc::new(shape_rich_line(
			self.d,
			layout.font,
			layout.size,
			layout.tracking,
			&layout.chars[start..end],
			layout.src_ls[line],
			layout.src_le[line],
			&layout.spans,
			styles,
			&mut self.cache.borrow_mut(),
		)))
	}
}

/// Returns the visual caret coordinate for a source position on one line.
pub(crate) fn caret_x(shaper: Shaper<'_>, layout: &TextLayout, line: usize, at: i32) -> f64 {
	let Some(shaped) = shaper.line(layout, line) else {
		return 0.0;
	};
	let base = layout.src_ls[line];
	let clamp = layout.src_le[line];
	for cluster in &shaped.clusters {
		let start = cluster.start.wrapping_add(base).min(clamp);
		let end = cluster.end.wrapping_add(base).min(clamp);
		if at == start {
			return if cluster.rtl { cluster.x1 } else { cluster.x0 };
		}
		if at == end {
			return if cluster.rtl { cluster.x0 } else { cluster.x1 };
		}
		if at > start && at < end {
			return if cluster.rtl { cluster.x0 } else { cluster.x1 };
		}
	}
	if at <= layout.src_ls[line] {
		0.0
	} else {
		shaped.width
	}
}

/// Finds the nearest shaped-cluster caret on one visual line.
pub(crate) fn caret_for_visual_x(
	shaper: Shaper<'_>,
	layout: &TextLayout,
	line: usize,
	goal: f64,
) -> i32 {
	let Some(shaped) = shaper.line(layout, line) else {
		return layout.src_ls.get(line).copied().unwrap_or(0);
	};
	let base = layout.src_ls[line];
	let clamp = layout.src_le[line];
	let rebase = |value: i32| value.wrapping_add(base).min(clamp);
	for cluster in &shaped.clusters {
		let midpoint = f64::midpoint(cluster.x0, cluster.x1);
		if goal < midpoint {
			return rebase(if cluster.rtl {
				cluster.end
			} else {
				cluster.start
			});
		}
		if goal <= cluster.x1 {
			return rebase(if cluster.rtl {
				cluster.start
			} else {
				cluster.end
			});
		}
	}
	shaped.clusters.last().map_or_else(
		|| layout.src_ls.get(line).copied().unwrap_or(0),
		|cluster| {
			rebase(if cluster.rtl {
				cluster.start
			} else {
				cluster.end
			})
		},
	)
}

/// Returns coalesced visual bands for a logical source selection on one line.
pub(crate) fn selection_bands(
	shaper: Shaper<'_>,
	layout: &TextLayout,
	line: usize,
	lo: i32,
	hi: i32,
) -> Vec<(f64, f64)> {
	let Some(shaped) = shaper.line(layout, line) else {
		return Vec::new();
	};
	let base = layout.src_ls[line];
	let clamp = layout.src_le[line];
	let mut spans: Vec<(f64, f64)> = shaped
		.clusters
		.iter()
		.filter(|cluster| {
			cluster.end.wrapping_add(base).min(clamp) > lo
				&& cluster.start.wrapping_add(base).min(clamp) < hi
		})
		.map(|cluster| (cluster.x0, cluster.x1))
		.collect();
	spans.sort_by(|left, right| left.0.total_cmp(&right.0));
	let mut bands: Vec<(f64, f64)> = Vec::new();
	for span in spans {
		if let Some(last) = bands.last_mut()
			&& span.0 <= last.1 + WIDTH_EPSILON
		{
			last.1 = last.1.max(span.1);
		} else {
			bands.push(span);
		}
	}
	bands
}
fn rich_font(d: &Doc, base_font: i32, style: u32) -> i32 {
	let bold = style & (1 << crate::edit::STYLE_BOLD) != 0;
	let weight = if bold {
		700
	} else {
		usize::try_from(base_font)
			.ok()
			.and_then(|index| d.font_weight.get(index))
			.copied()
			.unwrap_or(400)
	};
	if style & (1 << crate::edit::STYLE_CODE) != 0
		&& let Some((index, family)) = d
			.font_class
			.iter()
			.zip(&d.font_family)
			.enumerate()
			.find_map(|(index, (&class, &family))| (class == 1).then_some((index, family)))
	{
		let selected = slir::font_select(d, family, weight);
		return if selected >= 0 {
			selected
		} else {
			i32::try_from(index).expect("font index exceeds i32")
		};
	}
	usize::try_from(base_font)
		.ok()
		.and_then(|index| d.font_family.get(index))
		.map_or(base_font, |&family| slir::font_select(d, family, weight))
}

fn rich_mask(spans: &crate::edit::InlineSpans, point: i32) -> u32 {
	let mut mask = 0;
	for style in 0..=crate::edit::STYLE_CODE {
		if spans
			.get(style)
			.is_some_and(|ranges| ranges.contains(point))
		{
			mask |= 1 << style;
		}
	}
	mask
}

fn field_style_index(styles: &[crate::edit::FieldStyle], point: i32) -> i32 {
	let index = styles.partition_point(|style| style.end <= point);
	styles
		.get(index)
		.filter(|style| style.start <= point)
		.map_or(-1, |_| i32::try_from(index).expect("field style count exceeds i32"))
}

#[allow(
	clippy::too_many_arguments,
	reason = "rich shaping keeps metric and source inputs explicit"
)]
/// Shapes one rich line. `source_start`/`source_end` locate the line in the
/// field's source text for span-mask lookups only; emitted geometry is
/// line-local like [`shape_line`].
fn shape_rich_line(
	d: &Doc,
	base_font: i32,
	size: f64,
	tracking: f64,
	chars: &[u32],
	source_start: i32,
	source_end: i32,
	spans: &crate::edit::InlineSpans,
	field_styles: &[crate::edit::FieldStyle],
	cache: &mut ShapeCache,
) -> ShapedLine {
	if chars.is_empty() {
		return ShapedLine::default();
	}
	let masks: Vec<(u32, i32)> = (0..chars.len())
		.map(|local| {
			let offset = source_start.wrapping_add(i32::try_from(local).expect("text exceeds i32"));
			let point = if source_end > source_start {
				offset.min(source_end - 1)
			} else {
				source_start
			};
			(rich_mask(spans, point), field_style_index(field_styles, point))
		})
		.collect();
	let mut assigned = Vec::with_capacity(chars.len());
	let mut segment_start = 0;
	while segment_start < chars.len() {
		let mask = masks[segment_start].0;
		let mut segment_end = segment_start + 1;
		while segment_end < chars.len() && masks[segment_end].0 == mask {
			segment_end += 1;
		}
		let text: String = chars[segment_start..segment_end]
			.iter()
			.map(|&codepoint| char::from_u32(codepoint).unwrap_or(char::REPLACEMENT_CHARACTER))
			.collect();
		assigned.extend(font_assignments(
			d,
			rich_font(d, base_font, mask),
			&text,
			&chars[segment_start..segment_end],
		));
		segment_start = segment_end;
	}
	// Paint geometry lands in the same advance space rich measurement uses:
	// each codepoint charged to its span-selected font.
	let pos =
		advance_positions(d, size, tracking, chars, |local| rich_font(d, base_font, masks[local].0));

	let text: String = chars
		.iter()
		.map(|&codepoint| char::from_u32(codepoint).unwrap_or(char::REPLACEMENT_CHARACTER))
		.collect();
	let mut byte_offsets: Vec<usize> = text.char_indices().map(|(offset, _)| offset).collect();
	byte_offsets.push(text.len());
	let mut grapheme_starts = Vec::new();
	graphemes::boundaries(&text, &mut grapheme_starts);
	let bidi = unicode_bidi::BidiInfo::new(&text, None);
	let Some(paragraph) = bidi.paragraphs.first() else {
		return ShapedLine::default();
	};
	let (levels, visual_runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
	let mut line = ShapedLine::default();
	for visual in visual_runs {
		let start = byte_offsets
			.binary_search(&visual.start)
			.expect("bidi run starts at a character boundary");
		let end = byte_offsets
			.binary_search(&visual.end)
			.expect("bidi run ends at a character boundary");
		let rtl = levels.get(visual.start).is_some_and(|level| level.is_rtl());
		let mut logical_runs = Vec::new();
		let mut run_start = start;
		while run_start < end {
			let font = assigned[run_start];
			let (style, field_style) = masks[run_start];
			let mut run_end = run_start + 1;
			while run_end < end && assigned[run_end] == font && masks[run_end] == (style, field_style)
			{
				run_end += 1;
			}
			logical_runs.push((run_start, run_end, font, style, field_style));
			run_start = run_end;
		}
		if rtl {
			logical_runs.reverse();
		}
		for (start, end, font, style, field_style) in logical_runs {
			let (mut run, clusters) =
				shape_font_run(d, chars, start, end, font, size, tracking, rtl, line.width, cache);
			run.style = style;
			run.field_style = field_style;
			let (mut clusters, run_w) =
				advance_normalize_run(&mut run, clusters, &grapheme_starts, &pos, line.width);
			line.width += run_w;
			line.runs.push(run);
			line.clusters.append(&mut clusters);
		}
	}
	line.width = pos[chars.len()];
	line
}

/// FRAME.md-normative rich width: Σ [`char_w`] with each codepoint charged
/// to its span-selected font. Never shapes.
fn rich_range_width(
	d: &Doc,
	base_font: i32,
	size: f64,
	tracking: f64,
	src: &[u32],
	start: i32,
	end: i32,
	spans: &crate::edit::InlineSpans,
) -> f64 {
	let mut width = 0.0;
	for at in start..end {
		let point = if end > start { at.min(end - 1) } else { start };
		let font = rich_font(d, base_font, rich_mask(spans, point));
		width +=
			char_w(d, font, size, tracking, src[usize::try_from(at).expect("negative rich index")]);
	}
	width
}

/// Sequential rich advance fold anchored at a line's source start.
///
/// Each codepoint is charged to its span-selected font, exactly like
/// [`rich_range_width`]; incremental peeks continue the same op sequence,
/// so every returned width is bit-identical to a fresh fold.
struct RichFold {
	peek_end: i32,
	peek_w:   f64,
}

impl RichFold {
	const fn start_at(at: i32) -> Self {
		Self { peek_end: at, peek_w: 0.0 }
	}

	/// Width of `base..target`, continuing the running fold monotonically.
	#[allow(clippy::too_many_arguments, reason = "hot rich measurement fold")]
	fn to(
		&mut self,
		d: &Doc,
		base_font: i32,
		size: f64,
		tracking: f64,
		src: &[u32],
		spans: &crate::edit::InlineSpans,
		target: i32,
	) -> f64 {
		debug_assert!(target >= self.peek_end, "rich fold targets are monotone");
		while self.peek_end < target {
			let at = self.peek_end;
			let font = rich_font(d, base_font, rich_mask(spans, at));
			self.peek_w +=
				char_w(d, font, size, tracking, src[usize::try_from(at).expect("negative rich index")]);
			self.peek_end = self.peek_end.wrapping_add(1);
		}
		self.peek_w
	}
}

#[allow(
	clippy::too_many_arguments,
	reason = "wrapping mutates retained layout with explicit metric inputs"
)]
fn rich_wrap_hard(
	d: &Doc,
	base_font: i32,
	size: f64,
	tracking: f64,
	src: &[u32],
	a: i32,
	b: i32,
	max_w: f64,
	spans: &crate::edit::InlineSpans,
	layout: &mut TextLayout,
) {
	if a == b {
		finish_line(layout, i32::try_from(layout.chars.len()).expect("text exceeds i32"), a, b, 0.0);
		return;
	}
	let mut opportunities = Vec::new();
	line_break_boundaries(src, a, b, &mut opportunities);
	let mut breaks: Vec<i32> = opportunities
		.into_iter()
		.map(|(position, _)| position)
		.filter(|&position| position > a && position <= b)
		.collect();
	if breaks.last().copied() != Some(b) {
		breaks.push(b);
	}
	breaks.sort_unstable();
	breaks.dedup();

	let text: String = src[usize::try_from(a).expect("negative hard start")
		..usize::try_from(b).expect("negative hard end")]
		.iter()
		.map(|&codepoint| char::from_u32(codepoint).expect("valid codepoint"))
		.collect();
	let mut grapheme_offsets = Vec::new();
	graphemes::boundaries(&text, &mut grapheme_offsets);
	let grapheme_breaks: Vec<i32> = grapheme_offsets
		.into_iter()
		.skip(1)
		.map(|offset| a.wrapping_add(offset))
		.collect();

	let mut line_start = a;
	while line_start < b {
		let mut chosen = None;
		// Widths grow monotonically with the candidate end (trailing
		// whitespace is excluded, so trimmed ends never regress), letting one
		// running fold serve every probe on this line.
		let mut fold = RichFold::start_at(line_start);
		for &candidate in breaks.iter().filter(|&&position| position > line_start) {
			let mut content_end = candidate;
			while content_end > line_start
				&& matches!(src[usize::try_from(content_end - 1).expect("negative rich trim")], 9 | 32)
			{
				content_end -= 1;
			}
			let width = fold.to(d, base_font, size, tracking, src, spans, content_end);
			if width <= max_w + WIDTH_EPSILON {
				chosen = Some((candidate, content_end, width));
			} else {
				break;
			}
		}
		let (next, content_end, width) = if let Some(chosen) = chosen {
			chosen
		} else {
			let mut fallback = None;
			let mut fold = RichFold::start_at(line_start);
			for next in grapheme_breaks
				.iter()
				.copied()
				.filter(|&position| position > line_start)
			{
				let width = fold.to(d, base_font, size, tracking, src, spans, next);
				if fallback.is_none() || width <= max_w + WIDTH_EPSILON {
					fallback = Some((next, next, width));
				}
				if width > max_w + WIDTH_EPSILON {
					break;
				}
			}
			fallback.unwrap_or((b, b, 0.0))
		};
		let output_start = i32::try_from(layout.chars.len()).expect("text exceeds i32");
		append_range(layout, src, line_start, content_end);
		finish_line(layout, output_start, line_start, content_end, width);
		line_start = next;
		while line_start < b
			&& matches!(src[usize::try_from(line_start).expect("negative rich skip")], 9 | 32)
		{
			line_start += 1;
		}
	}
}

/// Span-charged width of an output slice, with positions past the source
/// range taking the clamped trailing mask exactly like rich paint shaping.
#[allow(clippy::too_many_arguments, reason = "rich measurement keeps metric inputs explicit")]
fn rich_output_w(
	d: &Doc,
	base_font: i32,
	size: f64,
	tracking: f64,
	spans: &crate::edit::InlineSpans,
	chars: &[u32],
	src_start: i32,
	src_end: i32,
) -> f64 {
	let mut width = 0.0;
	for (local, &cp) in chars.iter().enumerate() {
		let offset = src_start.wrapping_add(i32::try_from(local).expect("line length exceeds i32"));
		let point = if src_end > src_start {
			offset.min(src_end - 1)
		} else {
			src_start
		};
		width += char_w(d, rich_font(d, base_font, rich_mask(spans, point)), size, tracking, cp);
	}
	width
}

/// Cuts one over-width line at a grapheme boundary and appends `…`, all in
/// span-charged widths; the replacement lands in place.
#[allow(clippy::too_many_arguments, reason = "rich measurement keeps metric inputs explicit")]
fn rich_cut_line(
	d: &Doc,
	base_font: i32,
	size: f64,
	tracking: f64,
	spans: &crate::edit::InlineSpans,
	layout: &mut TextLayout,
	max_w: f64,
	line: usize,
) {
	let start = usize::try_from(layout.ls[line]).expect("negative line start");
	let end = usize::try_from(layout.le[line]).expect("negative line end");
	let src_start = layout.src_ls[line];
	let src_end = layout.src_le[line];
	let text: String = layout.chars[start..end]
		.iter()
		.map(|&cp| char::from_u32(cp).expect("valid codepoint"))
		.collect();
	let mut boundaries = Vec::new();
	graphemes::boundaries(&text, &mut boundaries);
	// Longest grapheme prefix whose width plus a span-charged ellipsis fits;
	// the ellipsis takes the clamped mask at its output position.
	let mut retained = 0_i32;
	for &boundary in boundaries.iter().skip(1) {
		let cut = usize::try_from(boundary).expect("negative boundary") + start;
		let prefix_w = rich_output_w(
			d,
			base_font,
			size,
			tracking,
			spans,
			&layout.chars[start..cut],
			src_start,
			src_end,
		);
		// The ellipsis paints at the cut position, whose mask clamps to the
		// LAST RETAINED source point (`src_le - 1` after the cut) — charge
		// the candidate identically so selection and paint agree at span
		// boundaries.
		let point = if boundary > 0 {
			src_start.wrapping_add(boundary - 1)
		} else {
			src_start
		};
		let ellipsis_w =
			char_w(d, rich_font(d, base_font, rich_mask(spans, point)), size, tracking, ELLIPSIS);
		if prefix_w + ellipsis_w > max_w + WIDTH_EPSILON {
			break;
		}
		retained = boundary;
	}
	let mut retained_end = retained;
	while retained_end > 0
		&& is_strippable(
			layout.chars[start + usize::try_from(retained_end - 1).expect("negative strip index")],
		) {
		retained_end -= 1;
	}
	// Whitespace stripping can move the ellipsis mask onto a different span,
	// so enforce the budget against the exact stored width. With monotone
	// candidate acceptance the post-strip width equals an earlier accepted
	// candidate and always fits; this loop is a defensive guard for that
	// invariant, retreating grapheme-wise if it is ever violated.
	loop {
		let src_le = src_start.wrapping_add(retained_end).min(src_end);
		let mut candidate: Vec<u32> =
			Vec::with_capacity(usize::try_from(retained_end).expect("negative retained length") + 1);
		candidate.extend_from_slice(
			&layout.chars
				[start..start + usize::try_from(retained_end).expect("negative retained length")],
		);
		candidate.push(ELLIPSIS);
		let width = rich_output_w(d, base_font, size, tracking, spans, &candidate, src_start, src_le);
		if width <= max_w + WIDTH_EPSILON || retained_end == 0 {
			let output_start = i32::try_from(layout.chars.len()).expect("text exceeds i32");
			layout.chars.extend_from_slice(&candidate);
			layout.ls[line] = output_start;
			layout.le[line] = i32::try_from(layout.chars.len()).expect("text exceeds i32");
			layout.src_le[line] = src_le;
			layout.line_w[line] = width;
			break;
		}
		// Retreat one full grapheme (never through a combining/ZWJ
		// sequence), then re-strip whitespace; every strippable codepoint is
		// its own grapheme, so the result stays on a boundary.
		let back = boundaries.partition_point(|&boundary| boundary < retained_end);
		retained_end = if back > 0 { boundaries[back - 1] } else { 0 };
		while retained_end > 0
			&& is_strippable(
				layout.chars[start + usize::try_from(retained_end - 1).expect("negative strip index")],
			) {
			retained_end -= 1;
		}
	}
}

/// Ellipsizes a rich layout with span-aware widths.
///
/// Mirrors the plain measurement's ellipsis contract (FRAME.md): over-width
/// lines cut at grapheme boundaries with trailing whitespace stripped, and a
/// layout truncated by `max_lines` still receives its terminal ellipsis on
/// the last line even when that line fits.
fn rich_ellipsize_layout(
	d: &Doc,
	base_font: i32,
	size: f64,
	tracking: f64,
	spans: &crate::edit::InlineSpans,
	layout: &mut TextLayout,
	max_w: f64,
) {
	if spans.is_empty() {
		return;
	}
	let mut cut_any = false;
	for line in 0..layout.ls.len() {
		if layout.line_w[line] <= max_w + WIDTH_EPSILON {
			continue;
		}
		layout.truncated = true;
		cut_any = true;
		rich_cut_line(d, base_font, size, tracking, spans, layout, max_w, line);
	}
	// A layout truncated by `max_lines` still owes its terminal ellipsis on
	// the (possibly fitting) last line, mirroring the plain contract.
	let lines = layout.ls.len();
	if layout.truncated && lines > 0 {
		let last = lines - 1;
		let start = usize::try_from(layout.ls[last]).expect("negative line start");
		let end = usize::try_from(layout.le[last]).expect("negative line end");
		let ends_with_ellipsis = end > start && layout.chars[end - 1] == ELLIPSIS;
		if !ends_with_ellipsis {
			let src_start = layout.src_ls[last];
			let src_end = layout.src_le[last];
			let line_len = i32::try_from(end - start).expect("line length exceeds i32");
			let point = if src_end > src_start {
				src_start.wrapping_add(line_len).min(src_end - 1)
			} else {
				src_start
			};
			let ellipsis_w =
				char_w(d, rich_font(d, base_font, rich_mask(spans, point)), size, tracking, ELLIPSIS);
			cut_any = true;
			if layout.line_w[last] + ellipsis_w <= max_w + WIDTH_EPSILON {
				// Rebuild the line at the buffer tail with the ellipsis
				// appended; the source range is unchanged.
				let output_start = i32::try_from(layout.chars.len()).expect("text exceeds i32");
				for k in start..end {
					layout.chars.push(layout.chars[k]);
				}
				layout.chars.push(ELLIPSIS);
				layout.ls[last] = output_start;
				layout.le[last] = i32::try_from(layout.chars.len()).expect("text exceeds i32");
				layout.line_w[last] += ellipsis_w;
			} else {
				rich_cut_line(d, base_font, size, tracking, spans, layout, max_w, last);
			}
		}
	}
	if cut_any {
		// Cut lines invalidate the hard tables; a delta splice must never
		// replay a truncated layout.
		layout.hard_lines.clear();
		layout.hard_src.clear();
	}
	layout.w = layout.line_w.iter().copied().fold(0.0_f64, f64::max);
}

/// Rebuilds wrapped line ranges using rich-segment advances.
#[allow(clippy::too_many_arguments, reason = "rich rewrap mirrors the text measurement contract")]
pub(crate) fn rewrap_rich_layout(
	d: &Doc,
	base_font: i32,
	size: f64,
	tracking: f64,
	text: &[u32],
	max_w: f64,
	max_lines: i32,
	spans: &crate::edit::InlineSpans,
	layout: &mut TextLayout,
) {
	if spans.is_empty() {
		return;
	}
	let src = text;
	layout.chars.clear();
	layout.ls.clear();
	layout.le.clear();
	layout.src_ls.clear();
	layout.src_le.clear();
	layout.line_w.clear();
	layout.shaped.get_mut().clear();
	// Legacy rich rewrap rebuilds lines wholesale; hard tables would be
	// stale, and stale tables must never feed a delta splice.
	layout.hard_lines.clear();
	layout.hard_src.clear();
	layout.truncated = false;
	let source_len = i32::try_from(src.len()).expect("text exceeds i32");
	let mut hard_start = 0;
	loop {
		let mut hard_end = hard_start;
		while hard_end < source_len
			&& src[usize::try_from(hard_end).expect("negative hard offset")] != 10
		{
			hard_end += 1;
		}
		rich_wrap_hard(d, base_font, size, tracking, src, hard_start, hard_end, max_w, spans, layout);
		if hard_end >= source_len {
			break;
		}
		hard_start = hard_end + 1;
	}
	if max_lines >= 0 && line_count(layout) > max_lines {
		let keep = usize::try_from(max_lines.max(1)).expect("negative rich line limit");
		layout.ls.truncate(keep);
		layout.le.truncate(keep);
		layout.src_ls.truncate(keep);
		layout.src_le.truncate(keep);
		layout.line_w.truncate(keep);
		layout.truncated = true;
	}
	*layout.shaped.get_mut() = vec![LineShape::Unshaped; layout.ls.len()];
	layout.w = layout.line_w.iter().copied().fold(0.0_f64, f64::max);
	layout.h = layout.line_h * f64::from(line_count(layout)).max(1.0);
}

/// FRAME.md-normative measured width: sequential Σ [`char_w`] of the
/// selected font over the slice. Measurement never shapes; a cmap miss
/// charges the selected table's deterministic fallback advance.
pub fn measured_w(d: &Doc, f: i32, size: f64, tracking: f64, chars: &[u32]) -> f64 {
	let mut width = 0.0;
	for &cp in chars {
		width += char_w(d, f, size, tracking, cp);
	}
	width
}

/// Measures a codepoint slice without allocating another character buffer.
pub fn advance_slice_w(
	d: &Doc,
	f: i32,
	size: f64,
	tracking: f64,
	chars: &[u32],
	a: i32,
	b: i32,
) -> f64 {
	slice_w(d, f, size, tracking, chars, a, b)
}

/// Measures a codepoint slice without allocating another character buffer.
pub fn slice_w(d: &Doc, f: i32, size: f64, tracking: f64, chars: &[u32], a: i32, b: i32) -> f64 {
	let start = usize::try_from(a).expect("negative character index");
	let end = usize::try_from(b).expect("negative character index");
	measured_w(d, f, size, tracking, &chars[start..end])
}

/// Measures a string's codepoint slice without materializing codepoints.
pub fn str_slice_w(d: &Doc, f: i32, size: f64, tracking: f64, text: &str, a: i32, b: i32) -> f64 {
	assert!(!(a < 0 || b < a), "invalid string slice");
	let chars: Vec<u32> = text.chars().map(u32::from).collect();
	slice_w(d, f, size, tracking, &chars, a, b)
}

/// Computes the line height from font size and leading.
pub fn line_h(size: f64, leading: f64) -> f64 {
	size * leading
}

/// Returns the first-baseline offset from the line top using the CSS
/// half-leading model.
pub fn ascent(d: &Doc, f: i32, size: f64, leading: f64) -> f64 {
	let line_height = line_h(size, leading);
	if f < 0 {
		return size * (0.76 + (leading - 1.0) / 2.0);
	}

	let font = usize::try_from(f).expect("nonnegative font index");
	let upem = f64::from(d.font_upem.get(font).copied().unwrap_or(0));
	if upem <= 0.0 {
		return size * (0.76 + (leading - 1.0) / 2.0);
	}

	let asc = f64::from(d.font_ascent[font]) * size / upem;
	// Font descents are stored as negative hhea values.
	let desc = f64::from(d.font_descent[font]) * size / upem;
	asc + (line_height - (asc - desc)) / 2.0
}

/// Returns underline offset below the baseline and thickness in layout units.
pub fn underline_geometry(d: &Doc, font: i32, size: f64) -> (f64, f64) {
	if font < 0 {
		return (size / 10.0, size / 16.0);
	}
	let index = usize::try_from(font).expect("nonnegative font index");
	let upem = f64::from(d.font_upem[index]);
	if upem <= 0.0 {
		return (size / 10.0, size / 16.0);
	}
	(
		-f64::from(
			d.font_underline_position
				.get(index)
				.copied()
				.unwrap_or(-(d.font_upem[index] as i32 / 10)),
		) * size
			/ upem,
		f64::from(
			d.font_underline_thickness
				.get(index)
				.copied()
				.unwrap_or_else(|| (d.font_upem[index] as i32 / 20).max(1)),
		)
		.abs() * size
			/ upem,
	)
}

/// Returns whether ellipsis truncation may strip this codepoint.
pub const fn is_strippable(cp: u32) -> bool {
	matches!(cp, 32 | 9 | 10 | 13 | NBSP)
}

/// Truncates `chars[a..b]` at a grapheme boundary and appends an ellipsis.
///
/// Source offsets continue to refer to the original line rather than the
/// synthesized output buffer.
// The arguments are the complete text-metric and source-range inputs to this
// standalone kernel operation; grouping them would only move the same data.
pub fn cut_line(
	d: &Doc,
	f: i32,
	size: f64,
	tracking: f64,
	tl: &mut TextLayout,
	a: i32,
	b: i32,
	src_a: i32,
	src_b: i32,
	max_w: f64,
) {
	let start = usize::try_from(a).expect("nonnegative character index");
	let end = usize::try_from(b).expect("nonnegative character index");
	let text: String = tl.chars[start..end]
		.iter()
		.map(|&codepoint| char::from_u32(codepoint).expect("valid codepoint"))
		.collect();
	let mut boundaries = Vec::new();
	graphemes::boundaries(&text, &mut boundaries);
	let mut candidate = Vec::with_capacity(end.saturating_sub(start) + 1);
	let mut retained = a;
	for &boundary in boundaries.iter().skip(1) {
		let candidate_end = a.wrapping_add(boundary);
		candidate.clear();
		candidate.extend_from_slice(
			&tl.chars[start..usize::try_from(candidate_end).expect("nonnegative boundary")],
		);
		candidate.push(ELLIPSIS);
		let width = measured_w(d, f, size, tracking, &candidate);
		if width > max_w + WIDTH_EPSILON {
			break;
		}
		retained = candidate_end;
	}

	let mut retained_end = retained;
	while retained_end > a
		&& is_strippable(
			tl.chars
				[usize::try_from(retained_end.wrapping_sub(1)).expect("nonnegative character index")],
		) {
		retained_end = retained_end.wrapping_sub(1);
	}
	let output_start = i32::try_from(tl.chars.len()).expect("text exceeds i32");
	for k in a..retained_end {
		tl.chars
			.push(tl.chars[usize::try_from(k).expect("nonnegative character index")]);
	}
	tl.chars.push(ELLIPSIS);
	let output_end = i32::try_from(tl.chars.len()).expect("text exceeds i32");
	tl.ls.push(output_start);
	tl.le.push(output_end);
	tl.src_ls.push(src_a);
	tl.src_le
		.push(src_a.wrapping_add(retained_end.wrapping_sub(a)).min(src_b));
	tl.line_w
		.push(slice_w(d, f, size, tracking, &tl.chars, output_start, output_end));
}

pub(crate) fn line_break_boundaries(src: &[u32], a: i32, b: i32, out: &mut Vec<(i32, bool)>) {
	out.clear();
	let mut text = String::new();
	let mut byte_ends =
		Vec::with_capacity(usize::try_from(b.wrapping_sub(a)).expect("nonnegative line-break range"));
	for i in a..b {
		let cp = src[usize::try_from(i).expect("nonnegative character index")];
		text.push(char::from_u32(cp).expect("valid codepoint"));
		byte_ends.push(text.len());
	}
	let mut character = 0usize;
	for (byte, opportunity) in linebreaks(&text) {
		while byte_ends.get(character).is_some_and(|&end| end <= byte) {
			character += 1;
		}
		out.push((
			a.wrapping_add(i32::try_from(character).expect("line-break offset exceeds i32")),
			opportunity == BreakOpportunity::Mandatory,
		));
	}
}

pub(crate) fn fallback_break_allowed(src: &[u32], boundary: i32) -> bool {
	let before = break_property(
		src[usize::try_from(boundary.wrapping_sub(1)).expect("nonnegative fallback boundary")],
	);
	let after =
		break_property(src[usize::try_from(boundary).expect("nonnegative fallback boundary")]);
	!matches!(before, BreakClass::NonBreakingGlue | BreakClass::WordJoiner)
		&& !matches!(
			after,
			BreakClass::NonBreakingGlue
				| BreakClass::WordJoiner
				| BreakClass::ClosePunctuation
				| BreakClass::CloseParenthesis
				| BreakClass::Exclamation
				| BreakClass::Inseparable
				| BreakClass::NonStarter
				| BreakClass::ConditionalJapaneseStarter
		) && !matches!(before, BreakClass::OpenPunctuation)
}

fn append_range(tl: &mut TextLayout, src: &[u32], a: i32, b: i32) {
	for k in a..b {
		tl.chars
			.push(src[usize::try_from(k).expect("nonnegative character index")]);
	}
}

/// Sequential advance fold anchored at a line's source start.
///
/// Extending toward a larger end never re-adds earlier codepoints, so every
/// returned width is bit-identical to a fresh left-to-right [`measured_w`]
/// over the same range — the property the wrap memo and the delta splice
/// rely on.
struct LineFold {
	end: i32,
	w:   f64,
}

impl LineFold {
	const fn start_at(at: i32) -> Self {
		Self { end: at, w: 0.0 }
	}

	/// Width through `target` without committing the extension.
	fn peek(&self, d: &Doc, f: i32, size: f64, tracking: f64, src: &[u32], target: i32) -> f64 {
		let mut width = self.w;
		for k in self.end..target {
			width += char_w(d, f, size, tracking, src[usize::try_from(k).expect("fold index")]);
		}
		width
	}

	/// Extends the fold through `target` and returns the committed width.
	fn advance_to(
		&mut self,
		d: &Doc,
		f: i32,
		size: f64,
		tracking: f64,
		src: &[u32],
		target: i32,
	) -> f64 {
		while self.end < target {
			self.w +=
				char_w(d, f, size, tracking, src[usize::try_from(self.end).expect("fold index")]);
			self.end = self.end.wrapping_add(1);
		}
		self.w
	}
}

fn append_fallback_range(
	d: &Doc,
	f: i32,
	size: f64,
	tracking: f64,
	tl: &mut TextLayout,
	src: &[u32],
	range_start: i32,
	range_end: i32,
	max_w: f64,
	line_start: &mut i32,
	source_start: &mut i32,
	fold: &mut LineFold,
) {
	let text: String = src[usize::try_from(range_start).expect("nonnegative range start")
		..usize::try_from(range_end).expect("nonnegative range end")]
		.iter()
		.map(|&codepoint| char::from_u32(codepoint).expect("valid codepoint"))
		.collect();
	let mut boundaries = Vec::new();
	graphemes::boundaries(&text, &mut boundaries);
	for pair in boundaries.windows(2) {
		let cluster_start = range_start.wrapping_add(pair[0]);
		let cluster_end = range_start.wrapping_add(pair[1]);
		let candidate_width = fold.peek(d, f, size, tracking, src, cluster_end);
		let line_nonempty = i32::try_from(tl.chars.len()).expect("text exceeds i32") > *line_start;
		if line_nonempty
			&& candidate_width > max_w + WIDTH_EPSILON
			&& fallback_break_allowed(src, cluster_start)
		{
			finish_line(tl, *line_start, *source_start, cluster_start, fold.w);
			*line_start = i32::try_from(tl.chars.len()).expect("text exceeds i32");
			*source_start = cluster_start;
			*fold = LineFold::start_at(cluster_start);
		}
		append_range(tl, src, cluster_start, cluster_end);
		fold.advance_to(d, f, size, tracking, src, cluster_end);
	}
}
/// Greedily wraps one hard line at spaces and UAX #14 opportunities, falling
/// back to grapheme-cluster boundaries for oversized runs. Widths are
/// FRAME.md-normative advance folds; wrapping never shapes.
// The arguments preserve the public wrapping primitive's explicit metric,
// source-range, and width inputs.
pub fn wrap_hard(
	d: &Doc,
	f: i32,
	size: f64,
	tracking: f64,
	tl: &mut TextLayout,
	src: &[u32],
	a: i32,
	b: i32,
	max_w: f64,
) {
	wrap_hard_metrics(d, f, size, tracking, tl, src, a, b, max_w);
}

fn wrap_hard_metrics(
	d: &Doc,
	f: i32,
	size: f64,
	tracking: f64,
	tl: &mut TextLayout,
	src: &[u32],
	a: i32,
	b: i32,
	max_w: f64,
) {
	let mut line_start = i32::try_from(tl.chars.len()).expect("text exceeds i32");
	let mut source_start = a;
	let mut fold = LineFold::start_at(a);
	let mut word_start = a;
	let mut all_breaks = Vec::new();
	line_break_boundaries(src, a, b, &mut all_breaks);
	let mut word_breaks = Vec::new();

	loop {
		let mut word_end = word_start;
		while word_end < b
			&& src[usize::try_from(word_end).expect("nonnegative character index")] != 32
		{
			word_end = word_end.wrapping_add(1);
		}

		let word_width = slice_w(d, f, size, tracking, src, word_start, word_end);
		word_breaks.clear();
		word_breaks.extend(
			all_breaks
				.iter()
				.copied()
				.filter(|&(position, _)| position > word_start && position < word_end),
		);
		let use_internal_breaks = !word_breaks.is_empty();
		if use_internal_breaks {
			word_breaks.push((word_end, false));
		}

		if use_internal_breaks {
			let mut unit_start = word_start;
			for &(unit_end, mandatory) in &word_breaks {
				let line_nonempty =
					i32::try_from(tl.chars.len()).expect("text exceeds i32") > line_start;
				let candidate_width = fold.peek(d, f, size, tracking, src, unit_end);
				if line_nonempty && candidate_width > max_w + WIDTH_EPSILON {
					let source_end = if unit_start == word_start {
						word_start.wrapping_sub(1)
					} else {
						unit_start
					};
					finish_line(tl, line_start, source_start, source_end, fold.w);
					line_start = i32::try_from(tl.chars.len()).expect("text exceeds i32");
					source_start = unit_start;
					fold = LineFold::start_at(unit_start);
				} else if unit_start == word_start && line_nonempty {
					tl.chars.push(32);
				}

				let unit_width = slice_w(d, f, size, tracking, src, unit_start, unit_end);
				if unit_width > max_w + WIDTH_EPSILON {
					append_fallback_range(
						d,
						f,
						size,
						tracking,
						tl,
						src,
						unit_start,
						unit_end,
						max_w,
						&mut line_start,
						&mut source_start,
						&mut fold,
					);
				} else {
					append_range(tl, src, unit_start, unit_end);
					fold.advance_to(d, f, size, tracking, src, unit_end);
				}
				if mandatory && unit_end < word_end {
					finish_line(tl, line_start, source_start, unit_end, fold.w);
					line_start = i32::try_from(tl.chars.len()).expect("text exceeds i32");
					source_start = unit_end;
					fold = LineFold::start_at(unit_end);
				}
				unit_start = unit_end;
			}
		} else {
			let line_nonempty = i32::try_from(tl.chars.len()).expect("text exceeds i32") > line_start;
			let candidate_width = fold.peek(d, f, size, tracking, src, word_end);
			if line_nonempty && candidate_width > max_w + WIDTH_EPSILON {
				finish_line(tl, line_start, source_start, word_start.wrapping_sub(1), fold.w);
				line_start = i32::try_from(tl.chars.len()).expect("text exceeds i32");
				source_start = word_start;
				fold = LineFold::start_at(word_start);
			}

			if word_width > max_w + WIDTH_EPSILON {
				append_fallback_range(
					d,
					f,
					size,
					tracking,
					tl,
					src,
					word_start,
					word_end,
					max_w,
					&mut line_start,
					&mut source_start,
					&mut fold,
				);
			} else {
				if i32::try_from(tl.chars.len()).expect("text exceeds i32") > line_start {
					tl.chars.push(32);
				}
				append_range(tl, src, word_start, word_end);
				fold.advance_to(d, f, size, tracking, src, word_end);
			}
		}

		if word_end >= b {
			break;
		}
		word_start = word_end.wrapping_add(1);
	}

	finish_line(tl, line_start, source_start, b, fold.w);
}

fn finish_line(tl: &mut TextLayout, start: i32, src_start: i32, src_end: i32, width: f64) {
	tl.ls.push(start);
	tl.le
		.push(i32::try_from(tl.chars.len()).expect("text exceeds i32"));
	tl.src_ls.push(src_start);
	tl.src_le.push(src_end);
	tl.line_w.push(width);
}

fn replace_line_with_appended(tl: &mut TextLayout, line: usize) {
	tl.ls[line] = tl.ls.pop().expect("appended line missing");
	tl.le[line] = tl.le.pop().expect("appended line missing");
	tl.src_ls[line] = tl.src_ls.pop().expect("appended line missing");
	tl.src_le[line] = tl.src_le.pop().expect("appended line missing");
	tl.line_w[line] = tl.line_w.pop().expect("appended line missing");
}

/// Measures `text`, optionally wrapping and truncating it.
///
/// `max_lines < 0` means unlimited lines. Input text is ordinary Unicode text;
/// output lines are codepoint slices in the returned layout.
// Text measurement intentionally exposes each independent layout option; a
// builder or options allocation would add ceremony on this hot kernel path.
pub fn measure_text(
	d: &Doc,
	f: i32,
	size: f64,
	leading: f64,
	tracking: f64,
	text: &str,
	max_w: f64,
	wrap: bool,
	ellipsis: bool,
	max_lines: i32,
) -> TextLayout {
	let mut cache = ShapeCache::default();
	let src: Vec<u32> = text.chars().map(u32::from).collect();
	measure_text_cached(
		d,
		f,
		size,
		leading,
		tracking,
		&src,
		max_w,
		wrap,
		ellipsis,
		max_lines,
		&crate::edit::InlineSpans::empty(),
		&mut cache,
	)
}

/// Measures rich text: [`measure_text`] with inline span styling applied to
/// wrapping and widths; the memo-eligible shape (no ellipsis, unbounded
/// lines) also memoizes per hard line.
#[allow(
	clippy::too_many_arguments,
	reason = "text measurement exposes each independent layout option"
)]
pub fn measure_rich_text(
	d: &Doc,
	f: i32,
	size: f64,
	leading: f64,
	tracking: f64,
	text: &str,
	max_w: f64,
	wrap: bool,
	ellipsis: bool,
	max_lines: i32,
	spans: &crate::edit::InlineSpans,
) -> TextLayout {
	let mut cache = ShapeCache::default();
	let src: Vec<u32> = text.chars().map(u32::from).collect();
	measure_text_cached(
		d, f, size, leading, tracking, &src, max_w, wrap, ellipsis, max_lines, spans, &mut cache,
	)
}

/// Recomputes measured line widths for a rich layout's output lines.
///
/// Folds each line's output codepoints with span-selected fonts, matching
/// the width lazy rich shaping reports — including ellipsis lines, whose
/// synthesized tail codepoints take the clamped trailing mask.
fn rich_layout_widths(
	d: &Doc,
	base_font: i32,
	size: f64,
	tracking: f64,
	spans: &crate::edit::InlineSpans,
	layout: &mut TextLayout,
) {
	if spans.is_empty() {
		return;
	}
	for line in 0..layout.ls.len() {
		let start = usize::try_from(layout.ls[line]).expect("negative line start");
		let end = usize::try_from(layout.le[line]).expect("negative line end");
		let src_start = layout.src_ls[line];
		let src_end = layout.src_le[line];
		let mut width = 0.0;
		for (local, &cp) in layout.chars[start..end].iter().enumerate() {
			let offset =
				src_start.wrapping_add(i32::try_from(local).expect("line length exceeds i32"));
			let point = if src_end > src_start {
				offset.min(src_end - 1)
			} else {
				src_start
			};
			let font = rich_font(d, base_font, rich_mask(spans, point));
			width += char_w(d, font, size, tracking, cp);
		}
		layout.line_w[line] = width;
	}
	layout.w = layout.line_w.iter().copied().fold(0.0_f64, f64::max);
}

/// Measures hard lines through the wrap memo, splicing cached line ranges
/// and widths for unchanged content.
///
/// Only valid for layouts without ellipsis or line limits: those transforms
/// rewrite lines after wrapping and would invalidate spliced entries.
#[allow(
	clippy::too_many_arguments,
	reason = "hot measurement path keeps the public measure inputs explicit"
)]
fn measure_hard_lines_memo(
	d: &Doc,
	f: i32,
	size: f64,
	tracking: f64,
	layout: &mut TextLayout,
	src: &[u32],
	max_w: f64,
	wrap: bool,
	spans: &crate::edit::InlineSpans,
	cache: &mut ShapeCache,
) {
	let source_len = i32::try_from(src.len()).expect("text exceeds i32");
	let sweep = src.iter().filter(|&&codepoint| codepoint == 10).count() + 1;
	cache.ensure_wrap_cap(sweep);
	memo_hard_range(d, f, size, tracking, layout, src, 0, source_len, max_w, wrap, spans, cache);

	// Over-width lines only mark truncation here: without ellipsis no line is
	// cut, matching the tail of the full measurement path.
	for line in 0..layout.line_w.len() {
		if layout.line_w[line] > max_w + WIDTH_EPSILON {
			layout.truncated = true;
		}
	}
	layout.w = layout.line_w.iter().copied().fold(0.0_f64, f64::max);
	layout.h = layout.line_h * f64::from(line_count(layout)).max(1.0);
}

/// Appends memoized hard-line measurements for `src[range_lo..range_hi)`.
///
/// `range_hi` must sit on a hard boundary (a `\n` or the end of the text);
/// the delta splice re-measures exactly one such window.
#[allow(
	clippy::too_many_arguments,
	reason = "hot measurement path keeps the public measure inputs explicit"
)]
fn memo_hard_range(
	d: &Doc,
	f: i32,
	size: f64,
	tracking: f64,
	layout: &mut TextLayout,
	src: &[u32],
	range_lo: i32,
	range_hi: i32,
	max_w: f64,
	wrap: bool,
	spans: &crate::edit::InlineSpans,
	cache: &mut ShapeCache,
) {
	let idx = |value: i32| usize::try_from(value).expect("nonnegative offset");
	let mut sig = Vec::new();
	let mut hard_start = range_lo;
	loop {
		let mut hard_end = hard_start;
		while hard_end < range_hi && src[idx(hard_end)] != 10 {
			hard_end = hard_end.wrapping_add(1);
		}
		layout
			.hard_lines
			.push(i32::try_from(layout.ls.len()).expect("line count exceeds i32"));
		layout.hard_src.push(hard_start);

		let hard = &src[idx(hard_start)..idx(hard_end)];
		span_sig(spans, hard_start, hard_end, &mut sig);
		let key = ShapeCache::wrap_key(f, size, tracking, max_w, wrap, hard, &sig);
		if let Some(entry) = cache.wrap_get(key, hard, &sig) {
			let out_base = i32::try_from(layout.chars.len()).expect("text exceeds i32");
			layout.chars.extend_from_slice(&entry.out_chars);
			for line in 0..entry.ls.len() {
				layout.ls.push(entry.ls[line].wrapping_add(out_base));
				layout.le.push(entry.le[line].wrapping_add(out_base));
				layout
					.src_ls
					.push(entry.src_ls[line].wrapping_add(hard_start));
				layout
					.src_le
					.push(entry.src_le[line].wrapping_add(hard_start));
				layout.line_w.push(entry.line_w[line]);
			}
		} else {
			let lines_before = layout.ls.len();
			let chars_before = layout.chars.len();
			if !sig.is_empty() && wrap {
				rich_wrap_hard(d, f, size, tracking, src, hard_start, hard_end, max_w, spans, layout);
			} else if wrap {
				wrap_hard_metrics(d, f, size, tracking, layout, src, hard_start, hard_end, max_w);
			} else {
				let output_start = i32::try_from(layout.chars.len()).expect("text exceeds i32");
				for k in hard_start..hard_end {
					layout.chars.push(src[idx(k)]);
				}
				let width = if sig.is_empty() {
					slice_w(d, f, size, tracking, src, hard_start, hard_end)
				} else {
					rich_range_width(d, f, size, tracking, src, hard_start, hard_end, spans)
				};
				finish_line(layout, output_start, hard_start, hard_end, width);
			}
			let out_base = i32::try_from(chars_before).expect("text exceeds i32");
			let entry = WrapMemoEntry {
				chars:     Rc::from(hard),
				spans_sig: Rc::from(sig.as_slice()),
				out_chars: Rc::from(&layout.chars[chars_before..]),
				ls:        layout.ls[lines_before..]
					.iter()
					.map(|&value| value.wrapping_sub(out_base))
					.collect(),
				le:        layout.le[lines_before..]
					.iter()
					.map(|&value| value.wrapping_sub(out_base))
					.collect(),
				src_ls:    layout.src_ls[lines_before..]
					.iter()
					.map(|&value| value.wrapping_sub(hard_start))
					.collect(),
				src_le:    layout.src_le[lines_before..]
					.iter()
					.map(|&value| value.wrapping_sub(hard_start))
					.collect(),
				line_w:    Rc::from(&layout.line_w[lines_before..]),
			};
			cache.wrap_insert(key, entry);
		}

		if hard_end >= range_hi {
			break;
		}
		hard_start = hard_end.wrapping_add(1);
	}
}

/// One contiguous forward text splice: `removed` codepoints at `at` were
/// replaced by `inserted` codepoints. Coordinates are in the NEW text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextDelta {
	pub at:       i32,
	pub removed:  i32,
	pub inserted: i32,
}

impl TextDelta {
	/// Merges a later splice into this one, or reports the pair as
	/// non-contiguous (`false`), in which case the delta is unusable.
	pub fn merge(&mut self, next: Self) -> bool {
		let new_end = self.at.wrapping_add(self.inserted);
		let next_end = next.at.wrapping_add(next.removed);
		if next.at > new_end || next_end < self.at {
			return false;
		}
		let left_ext = 0.max(self.at.wrapping_sub(next.at));
		let right_ext = 0.max(next_end.wrapping_sub(new_end));
		let overlap = next_end.min(new_end).wrapping_sub(next.at.max(self.at));
		self.at = self.at.min(next.at);
		self.removed = left_ext.wrapping_add(self.removed).wrapping_add(right_ext);
		self.inserted = self
			.inserted
			.wrapping_sub(overlap)
			.wrapping_add(next.inserted);
		true
	}
}

/// Re-measures only the hard lines a contiguous edit touched, splicing the
/// affected window of `prev` in place.
///
/// Returns `false` when the previous layout cannot support a splice (no hard
/// tables, or an out-of-range delta); the caller falls back to a full
/// measure. `spans` is the post-edit span set; callers must prove it moved
/// positionally with the edit ([`crate::edit::InlineSpans::follows_splice`]),
/// which keeps retained prefix/suffix line signatures bit-identical. The
/// result matches a full measure of `text`: hard lines measure
/// independently, untouched lines keep their exact folds, and suffix tables
/// shift by the edit's size without being rebuilt.
#[allow(
	clippy::too_many_arguments,
	reason = "hot measurement path keeps the public measure inputs explicit"
)]
pub(crate) fn measure_text_spliced_into(
	d: &Doc,
	f: i32,
	size: f64,
	tracking: f64,
	prev: &mut TextLayout,
	text: &[u32],
	delta: TextDelta,
	max_w: f64,
	wrap: bool,
	spans: &crate::edit::InlineSpans,
	cache: &mut ShapeCache,
) -> bool {
	if prev.hard_lines.is_empty() {
		return false;
	}
	let src = text;
	let new_len = i32::try_from(src.len()).expect("text exceeds i32");
	let shift = delta.inserted.wrapping_sub(delta.removed);
	let old_len = new_len.wrapping_sub(shift);
	let old_at = delta.at;
	let old_removed_end = delta.at.wrapping_add(delta.removed);
	if old_at < 0 || old_removed_end > old_len {
		return false;
	}

	// Affected hard window in the previous layout.
	let hards = prev.hard_src.len();
	let h0 = prev.hard_src.partition_point(|&start| start <= old_at) - 1;
	let h1 = prev
		.hard_src
		.partition_point(|&start| start <= old_removed_end)
		- 1;
	// Old window end: the codepoint before the next hard line's `\n`, or the
	// end of the old text.
	let old_win_end = if h1 + 1 < hards {
		prev.hard_src[h1 + 1] - 1
	} else {
		old_len
	};
	let win_start = prev.hard_src[h0];
	let new_win_end = old_win_end.wrapping_add(shift);
	if new_win_end < win_start || new_win_end > new_len {
		return false;
	}

	let idx = |value: i32| usize::try_from(value).expect("nonnegative offset");
	let first_mid_line = idx(prev.hard_lines[h0]);
	let prefix_chars = if first_mid_line < prev.ls.len() {
		idx(prev.ls[first_mid_line])
	} else {
		prev.chars.len()
	};
	// Old suffix anchors, captured before any table mutates.
	let (old_suffix_line, old_suffix_chars, old_suffix_out_line) = if h1 + 1 < hards {
		let line = idx(prev.hard_lines[h1 + 1]);
		(Some(line), idx(prev.ls[line]), prev.hard_lines[h1 + 1])
	} else {
		(None, prev.chars.len(), 0)
	};
	let old_mid_line_end = old_suffix_line.unwrap_or(prev.ls.len());

	// Re-measure the affected hard window into a scratch layout; its output
	// offsets are window-local and rebase during the table splices below.
	let mut mid = tl_new();
	mid.font = f;
	mid.size = size;
	mid.tracking = tracking;
	memo_hard_range(
		d,
		f,
		size,
		tracking,
		&mut mid,
		src,
		win_start,
		new_win_end,
		max_w,
		wrap,
		spans,
		cache,
	);
	let mid_lines = mid.ls.len();
	let mid_chars = mid.chars.len();
	let mid_hards = mid.hard_lines.len();
	let prefix_chars_shift = i32::try_from(prefix_chars).expect("text exceeds i32");
	let out_shift = i32::try_from(prefix_chars + mid_chars).expect("text exceeds i32")
		- i32::try_from(old_suffix_chars).expect("text exceeds i32");
	let first_mid_line_shift = i32::try_from(first_mid_line).expect("line count exceeds i32");

	prev
		.chars
		.splice(prefix_chars..old_suffix_chars, mid.chars.drain(..));
	prev.ls.splice(
		first_mid_line..old_mid_line_end,
		mid.ls
			.iter()
			.map(|&value| value.wrapping_add(prefix_chars_shift)),
	);
	prev.le.splice(
		first_mid_line..old_mid_line_end,
		mid.le
			.iter()
			.map(|&value| value.wrapping_add(prefix_chars_shift)),
	);
	prev
		.src_ls
		.splice(first_mid_line..old_mid_line_end, mid.src_ls.iter().copied());
	prev
		.src_le
		.splice(first_mid_line..old_mid_line_end, mid.src_le.iter().copied());
	prev
		.line_w
		.splice(first_mid_line..old_mid_line_end, mid.line_w.iter().copied());
	prev.shaped.get_mut().splice(
		first_mid_line..old_mid_line_end,
		std::iter::repeat_n(LineShape::Unshaped, mid_lines),
	);
	// Rich pins index lines; drop pins in the replaced window and shift
	// suffix pins with the spliced line count so eviction never unshapes a
	// shifted line or indexes past a shrunk layout.
	let pin_shift = isize::try_from(first_mid_line + mid_lines).expect("line count fits isize")
		- isize::try_from(old_mid_line_end).expect("line count fits isize");
	prev.rich_pins.get_mut().retain_mut(|pin| {
		if *pin < first_mid_line {
			return true;
		}
		if *pin < old_mid_line_end {
			return false;
		}
		*pin = pin
			.checked_add_signed(pin_shift)
			.expect("shifted pin index in range");
		true
	});
	prev.hard_lines.splice(
		h0..=h1,
		mid.hard_lines
			.iter()
			.map(|&value| value.wrapping_add(first_mid_line_shift)),
	);
	prev.hard_src.splice(h0..=h1, mid.hard_src.iter().copied());

	// Suffix: shift retained offsets by the edit's output and source sizes.
	if old_suffix_line.is_some() {
		let new_suffix_line = first_mid_line + mid_lines;
		for value in &mut prev.ls[new_suffix_line..] {
			*value = value.wrapping_add(out_shift);
		}
		for value in &mut prev.le[new_suffix_line..] {
			*value = value.wrapping_add(out_shift);
		}
		for value in &mut prev.src_ls[new_suffix_line..] {
			*value = value.wrapping_add(shift);
		}
		for value in &mut prev.src_le[new_suffix_line..] {
			*value = value.wrapping_add(shift);
		}
		let line_shift = i32::try_from(first_mid_line + mid_lines).expect("line count exceeds i32")
			- old_suffix_out_line;
		let new_hard_suffix = h0 + mid_hards;
		for value in &mut prev.hard_lines[new_hard_suffix..] {
			*value = value.wrapping_add(line_shift);
		}
		for value in &mut prev.hard_src[new_hard_suffix..] {
			*value = value.wrapping_add(shift);
		}
	}

	// The window re-measured with the post-edit spans; retained lines keep
	// signatures that rebase identically under the positional shift.
	prev.spans = spans.clone();
	prev.truncated = false;
	for line in 0..prev.line_w.len() {
		if prev.line_w[line] > max_w + WIDTH_EPSILON {
			prev.truncated = true;
		}
	}
	prev.w = prev.line_w.iter().copied().fold(0.0_f64, f64::max);
	prev.h = prev.line_h * f64::from(line_count(prev)).max(1.0);
	true
}

/// Clone-and-splice form of [`measure_text_spliced_into`] for differential
/// verification; production callers splice the retained layout in place.
#[allow(
	clippy::too_many_arguments,
	reason = "hot measurement path keeps the public measure inputs explicit"
)]
pub(crate) fn measure_text_spliced(
	d: &Doc,
	f: i32,
	size: f64,
	tracking: f64,
	prev: &TextLayout,
	text: &[u32],
	delta: TextDelta,
	max_w: f64,
	wrap: bool,
	spans: &crate::edit::InlineSpans,
	cache: &mut ShapeCache,
) -> Option<TextLayout> {
	let mut next = prev.clone();
	measure_text_spliced_into(
		d, f, size, tracking, &mut next, text, delta, max_w, wrap, spans, cache,
	)
	.then_some(next)
}

pub(crate) fn measure_text_cached(
	d: &Doc,
	f: i32,
	size: f64,
	leading: f64,
	tracking: f64,
	text: &[u32],
	max_w: f64,
	wrap: bool,
	ellipsis: bool,
	max_lines: i32,
	spans: &crate::edit::InlineSpans,
	cache: &mut ShapeCache,
) -> TextLayout {
	let src = text;
	let mut layout = tl_new();
	layout.line_h = line_h(size, leading);
	layout.ascent = ascent(d, f, size, leading);

	// Split on hard newlines before applying wrapping to each hard line.
	let source_len = i32::try_from(src.len()).expect("text exceeds i32");
	layout.font = f;
	layout.size = size;
	layout.tracking = tracking;
	layout.spans = spans.clone();
	if spans.is_empty() && !src.contains(&10) {
		let width = measured_w(d, f, size, tracking, src);
		if width <= max_w + WIDTH_EPSILON {
			layout.chars = src.to_vec();
			finish_line(&mut layout, 0, 0, source_len, width);
			layout.shaped.get_mut().push(LineShape::Unshaped);
			layout.hard_lines.push(0);
			layout.hard_src.push(0);
			layout.w = layout.line_w[0];
			layout.h = layout.line_h;
			return layout;
		}
	}
	// Editor-shaped inputs (no ellipsis, unbounded lines) replay per-hard-line
	// wrap results instead of re-probing and re-shaping the whole text; only
	// hard lines whose content or styling changed are measured again.
	if !ellipsis && max_lines < 0 {
		measure_hard_lines_memo(d, f, size, tracking, &mut layout, src, max_w, wrap, spans, cache);
		*layout.shaped.get_mut() = vec![LineShape::Unshaped; layout.ls.len()];
		return layout;
	}
	let mut hard_start = 0_i32;
	loop {
		let mut hard_end = hard_start;
		while hard_end < source_len
			&& src[usize::try_from(hard_end).expect("nonnegative character index")] != 10
		{
			hard_end = hard_end.wrapping_add(1);
		}

		if wrap {
			wrap_hard_metrics(d, f, size, tracking, &mut layout, src, hard_start, hard_end, max_w);
		} else {
			let output_start = i32::try_from(layout.chars.len()).expect("text exceeds i32");
			for k in hard_start..hard_end {
				layout
					.chars
					.push(src[usize::try_from(k).expect("nonnegative character index")]);
			}
			finish_line(
				&mut layout,
				output_start,
				hard_start,
				hard_end,
				slice_w(d, f, size, tracking, src, hard_start, hard_end),
			);
		}

		if hard_end >= source_len {
			break;
		}
		hard_start = hard_end.wrapping_add(1);
	}

	if max_lines >= 0 && line_count(&layout) > max_lines {
		let keep = max_lines.max(1);
		while line_count(&layout) > keep {
			layout.ls.pop().expect("pop on empty array");
			layout.le.pop().expect("pop on empty array");
			layout.src_ls.pop().expect("pop on empty array");
			layout.src_le.pop().expect("pop on empty array");
			layout.line_w.pop().expect("pop on empty array");
		}
		layout.truncated = true;
	}

	// Without ellipsis, over-width lines remain intact and are only marked as
	// truncated. With ellipsis, `cut_line` appends a replacement line. Rich
	// layouts defer cutting to the span-aware pass below.
	let plain_ellipsis = ellipsis && spans.is_empty();
	let line_total = line_count(&layout);
	for line in 0..line_total {
		let line_index = usize::try_from(line).expect("nonnegative line index");
		if layout.line_w[line_index] > max_w + WIDTH_EPSILON {
			layout.truncated = true;
			if plain_ellipsis {
				// Snapshot the line metadata before mutably borrowing the layout;
				// `cut_line` appends the replacement that is moved into this slot.
				let start = layout.ls[line_index];
				let end = layout.le[line_index];
				let source_start = layout.src_ls[line_index];
				let source_end = layout.src_le[line_index];
				cut_line(
					d,
					f,
					size,
					tracking,
					&mut layout,
					start,
					end,
					source_start,
					source_end,
					max_w,
				);
				replace_line_with_appended(&mut layout, line_index);
			}
		}
	}

	if layout.truncated && plain_ellipsis && line_count(&layout) > 0 {
		let last =
			usize::try_from(line_count(&layout).wrapping_sub(1)).expect("nonnegative line index");
		let start = layout.ls[last];
		let end = layout.le[last];
		let ends_with_ellipsis = end > start
			&& layout.chars
				[usize::try_from(end.wrapping_sub(1)).expect("nonnegative character index")]
				== ELLIPSIS;

		if !ends_with_ellipsis {
			let mut candidate = layout.chars[usize::try_from(start).expect("nonnegative line start")
				..usize::try_from(end).expect("nonnegative line end")]
				.to_vec();
			candidate.push(ELLIPSIS);
			let candidate_width = measured_w(d, f, size, tracking, &candidate);
			if candidate_width <= max_w + WIDTH_EPSILON {
				let output_start = i32::try_from(layout.chars.len()).expect("text exceeds i32");
				layout.chars.extend_from_slice(&candidate);
				layout.ls[last] = output_start;
				layout.le[last] = i32::try_from(layout.chars.len()).expect("text exceeds i32");
				layout.line_w[last] = candidate_width;
			} else {
				let source_start = layout.src_ls[last];
				let source_end = layout.src_le[last];
				cut_line(
					d,
					f,
					size,
					tracking,
					&mut layout,
					start,
					end,
					source_start,
					source_end,
					max_w,
				);
				replace_line_with_appended(&mut layout, last);
			}
		}
	}

	*layout.shaped.get_mut() = vec![LineShape::Unshaped; layout.ls.len()];
	// Rich layouts re-derive wrapping, widths, and ellipsis from their spans
	// so every caller of this API sees the styled-segment contract.
	if !spans.is_empty() && (ellipsis || max_lines >= 0) {
		if wrap {
			rewrap_rich_layout(d, f, size, tracking, text, max_w, max_lines, spans, &mut layout);
		}
		rich_layout_widths(d, f, size, tracking, spans, &mut layout);
		if ellipsis {
			rich_ellipsize_layout(d, f, size, tracking, spans, &mut layout, max_w);
		}
	}
	layout.w = layout.line_w.iter().copied().fold(0.0_f64, f64::max);
	layout.h = layout.line_h * f64::from(line_count(&layout)).max(1.0);
	layout
}
