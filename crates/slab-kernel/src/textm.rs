//! Text measurement using only SLIR font tables without host metrics.
//!
//! Wrapping follows the original `VectorMetrics` behavior: spaces split words
//! (including empty words), non-breaking spaces remain glued, overlong words
//! hard-break, and ellipsis truncation strips trailing whitespace. These
//! details are part of the conformance contract.
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

use std::{fmt, hash::Hasher, rc::Rc, sync::Arc};

use rustc_hash::{FxHashMap, FxHasher};

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

/// Retains parsed font faces, expensive OpenType plans, and immutable shaped
/// lines across frames.
#[derive(Default)]
pub(crate) struct ShapeCache {
	plans:      FxHashMap<ShapePlanKey, Arc<rustybuzz::ShapePlan>>,
	faces:      Vec<Option<CachedFace>>,
	lines:      FxHashMap<ShapedLineKey, CachedShapedLine>,
	lines_cold: FxHashMap<ShapedLineKey, CachedShapedLine>,
	buffer:     Option<rustybuzz::UnicodeBuffer>,
}

impl Clone for ShapeCache {
	fn clone(&self) -> Self {
		Self {
			plans:      self.plans.clone(),
			faces:      Vec::new(),
			lines:      self.lines.clone(),
			lines_cold: self.lines_cold.clone(),
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

	/// Drops retained data tied to the previously initialized document.
	pub(crate) fn clear(&mut self) {
		self.plans.clear();
		self.faces.clear();
		self.lines.clear();
		self.lines_cold.clear();
	}
}

/// One glyph emitted by OpenType shaping, positioned relative to its visual
/// line.
#[derive(Clone, Debug)]
pub struct ShapedGlyph {
	pub font:    i32,
	pub gid:     u32,
	pub cluster: i32,
	pub x:       f64,
	pub y:       f64,
}

/// One visually contiguous font and direction run.
#[derive(Clone, Debug)]
pub struct ShapedRun {
	/// Slice into [`TextLayout::chars`] in logical order.
	pub start:  i32,
	pub end:    i32,
	pub font:   i32,
	pub rtl:    bool,
	pub x:      f64,
	pub width:  f64,
	pub glyphs: Vec<ShapedGlyph>,
}

/// A shaped caret unit with visual geometry and logical source bounds.
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
#[derive(Clone, Debug)]
pub struct TextLayout {
	/// Codepoints for all output lines.
	pub chars:     Vec<u32>,
	/// Inclusive output start offset for each line.
	pub ls:        Vec<i32>,
	/// Exclusive output end offset for each line.
	pub le:        Vec<i32>,
	/// Inclusive original-text codepoint offset for each line.
	pub src_ls:    Vec<i32>,
	/// Exclusive original-text codepoint offset for each line.
	pub src_le:    Vec<i32>,
	/// Measured advance for each line.
	pub line_w:    Vec<f64>,
	/// OpenType-shaped runs and visual caret clusters, parallel to `line_w`.
	pub shaped:    Vec<Rc<ShapedLine>>,
	/// Maximum measured line width.
	pub w:         f64,
	/// Total layout height.
	pub h:         f64,
	/// First-baseline offset from a line's top.
	pub ascent:    f64,
	/// Height of one line.
	pub line_h:    f64,
	/// Whether line or width limits removed or replaced any text.
	pub truncated: bool,
}

/// A cached [`measure_text`] result with the exact inputs that produced it.
///
/// Layout stores one entry per text node and treats an entry as valid only
/// when every input matches, so style or content changes re-measure.
#[derive(Clone, Debug)]
pub struct TextCacheEntry {
	pub font:      i32,
	/// Bit patterns of the size, leading, tracking, and width budget.
	pub size:      u64,
	pub leading:   u64,
	pub tracking:  u64,
	pub max_w:     u64,
	pub wrap:      bool,
	pub ellipsis:  bool,
	pub max_lines: i32,
	/// Resolved text content at measurement time.
	pub content:   String,
	pub layout:    std::rc::Rc<TextLayout>,
}

/// Creates an empty text layout.
pub const fn tl_new() -> TextLayout {
	TextLayout {
		chars:     Vec::new(),
		ls:        Vec::new(),
		le:        Vec::new(),
		src_ls:    Vec::new(),
		src_le:    Vec::new(),
		line_w:    Vec::new(),
		shaped:    Vec::new(),
		w:         0.0,
		h:         0.0,
		ascent:    0.0,
		line_h:    0.0,
		truncated: false,
	}
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
	let upem = f64::from(d.font_upem[font]);
	if upem <= 0.0 {
		return 0.6f64.mul_add(size, tracking);
	}

	let cmap_index = slir::font_cmap_ix(d, f, cp);
	if cmap_index < 0 {
		let mut advance = f64::from(d.font_default_adv[font]);
		if d.font_class[font] == 1 && graphemes::cp_wide(cp) {
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

fn fallback_font(d: &Doc, primary: i32, chars: &[u32]) -> i32 {
	if font_covers(d, primary, chars) {
		return primary;
	}
	for font in (0..d.font_upem.len()).rev() {
		let font = i32::try_from(font).expect("font table exceeds i32");
		if font != primary && font_covers(d, font, chars) {
			return font;
		}
	}
	primary
}

fn font_assignments(d: &Doc, primary: i32, text: &str, chars: &[u32]) -> Vec<i32> {
	let mut assigned = vec![primary; chars.len()];
	if font_covers(d, primary, chars) {
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
	source_delta: i32,
	source_end: i32,
) {
	clusters.push(ShapedCluster {
		start: start.wrapping_add(source_delta).min(source_end),
		end: end.wrapping_add(source_delta).min(source_end),
		x0,
		x1,
		rtl,
	});
}

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
	output_start: i32,
	source_delta: i32,
	source_end: i32,
	cache: &mut ShapeCache,
) -> (ShapedRun, Vec<ShapedCluster>) {
	let run_start = output_start.wrapping_add(i32::try_from(start).expect("text exceeds i32"));
	let run_end = output_start.wrapping_add(i32::try_from(end).expect("text exceeds i32"));
	let mut glyphs = Vec::new();
	let mut clusters = Vec::new();
	let data = slir::font_data(d, font);
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
					cluster: cluster.wrapping_add(source_delta),
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
				push_cluster(&mut clusters, cluster, end, x0, x1, rtl, source_delta, source_end);
			}
			let run = ShapedRun {
				start: run_start,
				end: run_end,
				font,
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
		let cluster =
			output_start.wrapping_add(i32::try_from(cluster_start).expect("text exceeds i32"));
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
				cluster: cluster.wrapping_add(source_delta),
				x: cursor,
				y: 0.0,
			});
			cursor += width;
		}
		push_cluster(
			&mut clusters,
			cluster,
			output_start.wrapping_add(i32::try_from(cluster_end).expect("text exceeds i32")),
			x0,
			cursor,
			rtl,
			source_delta,
			source_end,
		);
	}
	(
		ShapedRun {
			start: run_start,
			end: run_end,
			font,
			rtl,
			x: line_x,
			width: cursor - line_x,
			glyphs,
		},
		clusters,
	)
}

/// Shapes and visually reorders one logical line.
pub fn shape_line(
	d: &Doc,
	primary_font: i32,
	size: f64,
	tracking: f64,
	chars: &[u32],
	output_start: i32,
	source_start: i32,
	source_end: i32,
) -> ShapedLine {
	shape_line_uncached(
		d,
		primary_font,
		size,
		tracking,
		chars,
		output_start,
		source_start,
		source_end,
		&mut ShapeCache::default(),
	)
}

fn shape_line_uncached(
	d: &Doc,
	primary_font: i32,
	size: f64,
	tracking: f64,
	chars: &[u32],
	output_start: i32,
	source_start: i32,
	source_end: i32,
	cache: &mut ShapeCache,
) -> ShapedLine {
	if chars.is_empty() {
		return ShapedLine::default();
	}
	if chars.iter().all(|&codepoint| codepoint <= 0x7f) && font_covers(d, primary_font, chars) {
		let (run, clusters) = shape_font_run(
			d,
			chars,
			0,
			chars.len(),
			primary_font,
			size,
			tracking,
			false,
			0.0,
			output_start,
			source_start.wrapping_sub(output_start),
			source_end,
			cache,
		);
		let width = run.width;
		return ShapedLine { runs: vec![run], clusters, width };
	}
	let text: String = chars
		.iter()
		.map(|&codepoint| char::from_u32(codepoint).unwrap_or(char::REPLACEMENT_CHARACTER))
		.collect();
	let assigned = font_assignments(d, primary_font, &text, chars);
	let mut byte_offsets: Vec<usize> = text.char_indices().map(|(offset, _)| offset).collect();
	byte_offsets.push(text.len());
	let bidi = unicode_bidi::BidiInfo::new(&text, None);
	let Some(paragraph) = bidi.paragraphs.first() else {
		return ShapedLine::default();
	};
	let (levels, visual_runs) = bidi.visual_runs(paragraph, paragraph.range.clone());
	let source_delta = source_start.wrapping_sub(output_start);
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
			let (run, mut clusters) = shape_font_run(
				d,
				chars,
				start,
				end,
				font,
				size,
				tracking,
				rtl,
				line.width,
				output_start,
				source_delta,
				source_end,
				cache,
			);
			line.width += run.width;
			line.runs.push(run);
			line.clusters.append(&mut clusters);
		}
	}
	line
}

/// Returns immutable geometry normalized to zero-based output and source
/// ranges.
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
	let end = i32::try_from(chars.len()).expect("text exceeds i32");
	let line =
		Rc::new(shape_line_uncached(d, primary_font, size, tracking, chars, 0, 0, end, cache));
	cache.insert_line(key, chars, Rc::clone(&line));
	line
}

pub(crate) fn shape_line_cached(
	d: &Doc,
	primary_font: i32,
	size: f64,
	tracking: f64,
	chars: &[u32],
	output_start: i32,
	source_start: i32,
	source_end: i32,
	cache: &mut ShapeCache,
) -> Rc<ShapedLine> {
	let shared = shape_line_shared_cached(d, primary_font, size, tracking, chars, cache);
	if output_start == 0
		&& source_start == 0
		&& source_end == i32::try_from(chars.len()).expect("text exceeds i32")
	{
		return shared;
	}
	let mut line = shared.as_ref().clone();
	for run in &mut line.runs {
		run.start = run.start.wrapping_add(output_start);
		run.end = run.end.wrapping_add(output_start);
		for glyph in &mut run.glyphs {
			glyph.cluster = glyph.cluster.wrapping_add(source_start);
		}
	}
	for cluster in &mut line.clusters {
		cluster.start = cluster.start.wrapping_add(source_start).min(source_end);
		cluster.end = cluster.end.wrapping_add(source_start).min(source_end);
	}
	Rc::new(line)
}

/// Returns the visual caret coordinate for a source position on one line.
pub fn caret_x(layout: &TextLayout, line: usize, at: i32) -> f64 {
	let Some(shaped) = layout.shaped.get(line) else {
		return 0.0;
	};
	for cluster in &shaped.clusters {
		if at == cluster.start {
			return if cluster.rtl { cluster.x1 } else { cluster.x0 };
		}
		if at == cluster.end {
			return if cluster.rtl { cluster.x0 } else { cluster.x1 };
		}
		if at > cluster.start && at < cluster.end {
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
pub fn caret_for_visual_x(layout: &TextLayout, line: usize, goal: f64) -> i32 {
	let Some(shaped) = layout.shaped.get(line) else {
		return layout.src_ls.get(line).copied().unwrap_or(0);
	};
	for cluster in &shaped.clusters {
		let midpoint = f64::midpoint(cluster.x0, cluster.x1);
		if goal < midpoint {
			return if cluster.rtl {
				cluster.end
			} else {
				cluster.start
			};
		}
		if goal <= cluster.x1 {
			return if cluster.rtl {
				cluster.start
			} else {
				cluster.end
			};
		}
	}
	shaped.clusters.last().map_or_else(
		|| layout.src_ls.get(line).copied().unwrap_or(0),
		|cluster| {
			if cluster.rtl {
				cluster.start
			} else {
				cluster.end
			}
		},
	)
}

/// Returns coalesced visual bands for a logical source selection on one line.
pub fn selection_bands(layout: &TextLayout, line: usize, lo: i32, hi: i32) -> Vec<(f64, f64)> {
	let Some(shaped) = layout.shaped.get(line) else {
		return Vec::new();
	};
	let mut spans: Vec<(f64, f64)> = shaped
		.clusters
		.iter()
		.filter(|cluster| cluster.end > lo && cluster.start < hi)
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
fn shape_layout(
	d: &Doc,
	font: i32,
	size: f64,
	tracking: f64,
	layout: &mut TextLayout,
	cache: &mut ShapeCache,
) {
	layout.shaped.clear();
	for line in 0..layout.ls.len() {
		let start = usize::try_from(layout.ls[line]).expect("negative line start");
		let end = usize::try_from(layout.le[line]).expect("negative line end");
		let shaped = shape_line_cached(
			d,
			font,
			size,
			tracking,
			&layout.chars[start..end],
			layout.ls[line],
			layout.src_ls[line],
			layout.src_le[line],
			cache,
		);
		layout.line_w[line] = shaped.width;
		layout.shaped.push(shaped);
	}
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
	let mut cache = ShapeCache::default();
	slice_w_cached(d, f, size, tracking, chars, a, b, &mut cache)
}

/// Measures a codepoint slice while reusing retained shaped geometry.
pub(crate) fn slice_w_cached(
	d: &Doc,
	f: i32,
	size: f64,
	tracking: f64,
	chars: &[u32],
	a: i32,
	b: i32,
	cache: &mut ShapeCache,
) -> f64 {
	let start = usize::try_from(a).expect("negative character index");
	let end = usize::try_from(b).expect("negative character index");
	shape_line_shared_cached(d, f, size, tracking, &chars[start..end], cache).width
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
	let upem = f64::from(d.font_upem[font]);
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
	let mut cache = ShapeCache::default();
	cut_line_cached(d, f, size, tracking, tl, a, b, src_a, src_b, max_w, &mut cache);
}

fn cut_line_cached(
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
	cache: &mut ShapeCache,
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
		let width = slice_w_cached(
			d,
			f,
			size,
			tracking,
			&candidate,
			0,
			i32::try_from(candidate.len()).expect("candidate exceeds i32"),
			cache,
		);
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
		.push(slice_w_cached(d, f, size, tracking, &tl.chars, output_start, output_end, cache));
}

/// Greedily wraps one hard line at spaces and hard-breaks oversized words at
/// grapheme-cluster boundaries.
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
	let mut cache = ShapeCache::default();
	wrap_hard_cached(d, f, size, tracking, tl, src, a, b, max_w, &mut cache);
}

fn wrap_hard_cached(
	d: &Doc,
	f: i32,
	size: f64,
	tracking: f64,
	tl: &mut TextLayout,
	src: &[u32],
	a: i32,
	b: i32,
	max_w: f64,
	cache: &mut ShapeCache,
) {
	let mut line_start = i32::try_from(tl.chars.len()).expect("text exceeds i32");
	let mut source_start = a;
	let mut line_width = 0.0;
	let mut word_start = a;

	loop {
		let mut word_end = word_start;
		while word_end < b
			&& src[usize::try_from(word_end).expect("nonnegative character index")] != 32
		{
			word_end = word_end.wrapping_add(1);
		}

		let word_width = slice_w_cached(d, f, size, tracking, src, word_start, word_end, cache);
		let line_nonempty = i32::try_from(tl.chars.len()).expect("text exceeds i32") > line_start;
		let candidate_width =
			slice_w_cached(d, f, size, tracking, src, source_start, word_end, cache);
		if line_nonempty && candidate_width > max_w + WIDTH_EPSILON {
			finish_line(tl, line_start, source_start, word_start.wrapping_sub(1), line_width);
			line_start = i32::try_from(tl.chars.len()).expect("text exceeds i32");
			source_start = word_start;
			line_width = 0.0;
		}

		if word_width > max_w + WIDTH_EPSILON {
			let word: String = src[usize::try_from(word_start).expect("nonnegative word start")
				..usize::try_from(word_end).expect("nonnegative word end")]
				.iter()
				.map(|&codepoint| char::from_u32(codepoint).expect("valid codepoint"))
				.collect();
			let mut boundaries = Vec::new();
			graphemes::boundaries(&word, &mut boundaries);
			for pair in boundaries.windows(2) {
				let cluster_start = word_start.wrapping_add(pair[0]);
				let cluster_end = word_start.wrapping_add(pair[1]);
				let candidate_width =
					slice_w_cached(d, f, size, tracking, src, source_start, cluster_end, cache);
				let line_nonempty =
					i32::try_from(tl.chars.len()).expect("text exceeds i32") > line_start;
				if line_nonempty && candidate_width > max_w + WIDTH_EPSILON {
					finish_line(tl, line_start, source_start, cluster_start, line_width);
					line_start = i32::try_from(tl.chars.len()).expect("text exceeds i32");
					source_start = cluster_start;
				}
				for k in cluster_start..cluster_end {
					tl.chars
						.push(src[usize::try_from(k).expect("nonnegative character index")]);
				}
				line_width =
					slice_w_cached(d, f, size, tracking, src, source_start, cluster_end, cache);
			}
		} else {
			if i32::try_from(tl.chars.len()).expect("text exceeds i32") > line_start {
				tl.chars.push(32);
			}
			for k in word_start..word_end {
				tl.chars
					.push(src[usize::try_from(k).expect("nonnegative character index")]);
			}
			let output_end = i32::try_from(tl.chars.len()).expect("text exceeds i32");
			line_width =
				slice_w_cached(d, f, size, tracking, &tl.chars, line_start, output_end, cache);
		}

		if word_end >= b {
			break;
		}
		word_start = word_end.wrapping_add(1);
	}

	finish_line(tl, line_start, source_start, b, line_width);
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
	measure_text_cached(
		d, f, size, leading, tracking, text, max_w, wrap, ellipsis, max_lines, &mut cache,
	)
}

pub(crate) fn measure_text_cached(
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
	cache: &mut ShapeCache,
) -> TextLayout {
	let mut src: Vec<u32> = Vec::with_capacity(text.len());
	src.extend(text.chars().map(u32::from));
	let mut layout = tl_new();
	layout.line_h = line_h(size, leading);
	layout.ascent = ascent(d, f, size, leading);

	// Split on hard newlines before applying wrapping to each hard line.
	let source_len = i32::try_from(src.len()).expect("text exceeds i32");
	if !src.contains(&10) {
		let shaped = shape_line_cached(d, f, size, tracking, &src, 0, 0, source_len, cache);
		if shaped.width <= max_w + WIDTH_EPSILON {
			layout.chars = src;
			finish_line(&mut layout, 0, 0, source_len, shaped.width);
			layout.shaped.push(shaped);
			layout.w = layout.line_w[0];
			layout.h = layout.line_h;
			return layout;
		}
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
			wrap_hard_cached(
				d,
				f,
				size,
				tracking,
				&mut layout,
				&src,
				hard_start,
				hard_end,
				max_w,
				cache,
			);
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
				slice_w_cached(d, f, size, tracking, &src, hard_start, hard_end, cache),
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
	// truncated. With ellipsis, `cut_line` appends a replacement line.
	let line_total = line_count(&layout);
	for line in 0..line_total {
		let line_index = usize::try_from(line).expect("nonnegative line index");
		if layout.line_w[line_index] > max_w + WIDTH_EPSILON {
			layout.truncated = true;
			if ellipsis {
				// Snapshot the line metadata before mutably borrowing the layout;
				// `cut_line` appends the replacement that is moved into this slot.
				let start = layout.ls[line_index];
				let end = layout.le[line_index];
				let source_start = layout.src_ls[line_index];
				let source_end = layout.src_le[line_index];
				cut_line_cached(
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
					cache,
				);
				replace_line_with_appended(&mut layout, line_index);
			}
		}
	}

	if layout.truncated && ellipsis && line_count(&layout) > 0 {
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
			let candidate_width = slice_w_cached(
				d,
				f,
				size,
				tracking,
				&candidate,
				0,
				i32::try_from(candidate.len()).expect("candidate exceeds i32"),
				cache,
			);
			if candidate_width <= max_w + WIDTH_EPSILON {
				let output_start = i32::try_from(layout.chars.len()).expect("text exceeds i32");
				layout.chars.extend_from_slice(&candidate);
				layout.ls[last] = output_start;
				layout.le[last] = i32::try_from(layout.chars.len()).expect("text exceeds i32");
				layout.line_w[last] = candidate_width;
			} else {
				let source_start = layout.src_ls[last];
				let source_end = layout.src_le[last];
				cut_line_cached(
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
					cache,
				);
				replace_line_with_appended(&mut layout, last);
			}
		}
	}

	shape_layout(d, f, size, tracking, &mut layout, cache);
	layout.w = layout.line_w.iter().copied().fold(0.0_f64, f64::max);
	layout.h = layout.line_h * f64::from(line_count(&layout)).max(1.0);
	layout
}
