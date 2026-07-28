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

use crate::{
	graphemes,
	slir::{self, Doc},
};

/// Unicode codepoint inserted when a line is ellipsized.
pub const ELLIPSIS: u32 = 0x2026;
/// Unicode non-breaking-space codepoint.
pub const NBSP: u32 = 0xa0;
const WIDTH_EPSILON: f64 = 1e-6;

/// One glyph emitted by OpenType shaping, positioned relative to its visual line.
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
	pub shaped:    Vec<ShapedLine>,
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
		&& chars
			.iter()
			.all(|&codepoint| !graphemes::requires_glyph(codepoint) || slir::font_gid(d, font, codepoint) != 0)
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
	if upem > 0
		&& let Some(mut face) = rustybuzz::Face::from_slice(data, 0)
	{
		if let Some(weight) = usize::try_from(font)
			.ok()
			.and_then(|index| d.font_weight.get(index))
			&& let Ok(variation) = format!("wght={weight}").parse::<rustybuzz::Variation>()
		{
			face.set_variations(&[variation]);
		}
		let mut buffer = rustybuzz::UnicodeBuffer::new();
		for (local, &codepoint) in chars[start..end].iter().enumerate() {
			buffer.add(
				char::from_u32(codepoint).unwrap_or(char::REPLACEMENT_CHARACTER),
				u32::try_from(
					run_start.wrapping_add(i32::try_from(local).expect("text exceeds i32")),
				)
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
		let shaped = rustybuzz::shape(&face, &[], buffer);
		let infos = shaped.glyph_infos();
		let positions = shaped.glyph_positions();
		let scale = size / f64::from(upem);
		let mut cursor = line_x;
		let mut group_cluster = None;
		let mut group_x = cursor;
		let mut groups = Vec::new();
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
				x: cursor + f64::from(position.x_offset) * scale,
				y: -f64::from(position.y_offset) * scale,
			});
			cursor += f64::from(position.x_advance) * scale;
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
			push_cluster(
				&mut clusters,
				cluster,
				end,
				x0,
				x1,
				rtl,
				source_delta,
				source_end,
			);
		}
		return (
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
		);
	}

	let indices: Box<dyn Iterator<Item = usize>> = if rtl {
		Box::new((start..end).rev())
	} else {
		Box::new(start..end)
	};
	let mut cursor = line_x;
	for index in indices {
		let codepoint = chars[index];
		let cluster = output_start.wrapping_add(i32::try_from(index).expect("text exceeds i32"));
		let width = char_w(d, font, size, tracking, codepoint);
		glyphs.push(ShapedGlyph {
			font,
			gid: if graphemes::is_glyph_modifier(codepoint) {
				0
			} else {
				slir::font_gid(d, font, codepoint)
			},
			cluster: cluster.wrapping_add(source_delta),
			x: cursor,
			y: 0.0,
		});
		push_cluster(
			&mut clusters,
			cluster,
			cluster.wrapping_add(1),
			cursor,
			cursor + width,
			rtl,
			source_delta,
			source_end,
		);
		cursor += width;
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
	if chars.is_empty() {
		return ShapedLine::default();
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
			);
			line.width += run.width;
			line.runs.push(run);
			line.clusters.append(&mut clusters);
		}
	}
	line
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
	if at <= layout.src_ls[line] { 0.0 } else { shaped.width }
}

/// Finds the nearest shaped-cluster caret on one visual line.
pub fn caret_for_visual_x(layout: &TextLayout, line: usize, goal: f64) -> i32 {
	let Some(shaped) = layout.shaped.get(line) else {
		return layout.src_ls.get(line).copied().unwrap_or(0);
	};
	for cluster in &shaped.clusters {
		let midpoint = f64::midpoint(cluster.x0, cluster.x1);
		if goal < midpoint {
			return if cluster.rtl { cluster.end } else { cluster.start };
		}
		if goal <= cluster.x1 {
			return if cluster.rtl { cluster.start } else { cluster.end };
		}
	}
	layout.src_le.get(line).copied().unwrap_or(0)
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
fn shape_layout(d: &Doc, font: i32, size: f64, tracking: f64, layout: &mut TextLayout) {
	layout.shaped.clear();
	for line in 0..layout.ls.len() {
		let start = usize::try_from(layout.ls[line]).expect("negative line start");
		let end = usize::try_from(layout.le[line]).expect("negative line end");
		let shaped = shape_line(
			d,
			font,
			size,
			tracking,
			&layout.chars[start..end],
			layout.ls[line],
			layout.src_ls[line],
			layout.src_le[line],
		);
		layout.line_w[line] = shaped.width;
		layout.shaped.push(shaped);
	}
}


/// Measures a codepoint slice without allocating another character buffer.
pub fn slice_w(d: &Doc, f: i32, size: f64, tracking: f64, chars: &[u32], a: i32, b: i32) -> f64 {
	let start = usize::try_from(a).expect("negative character index");
	let end = usize::try_from(b).expect("negative character index");
	shape_line(d, f, size, tracking, &chars[start..end], a, a, b).width
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
		-f64::from(d.font_underline_position.get(index).copied().unwrap_or(-(d.font_upem[index] as i32 / 10)))
			* size
			/ upem,
		f64::from(
			d.font_underline_thickness
				.get(index)
				.copied()
				.unwrap_or((d.font_upem[index] as i32 / 20).max(1)),
		)
		.abs()
			* size
			/ upem,
	)
}

/// Returns whether ellipsis truncation may strip this codepoint.
pub const fn is_strippable(cp: u32) -> bool {
	matches!(cp, 32 | 9 | 10 | 13 | NBSP)
}

/// Truncates `chars[a..b]` with an ellipsis. Source offsets continue to refer
/// to the original line rather than the synthesized output buffer.
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
	let ellipsis_width = char_w(d, f, size, tracking, ELLIPSIS);
	let mut width = 0.0;
	let mut i = a;

	while i < b {
		let char_width = char_w(
			d,
			f,
			size,
			tracking,
			tl.chars[usize::try_from(i).expect("nonnegative character index")],
		);
		if width + char_width + ellipsis_width > max_w + WIDTH_EPSILON {
			let mut end = i;
			while end > a
				&& is_strippable(
					tl.chars[usize::try_from(end.wrapping_sub(1)).expect("nonnegative character index")],
				) {
				end = end.wrapping_sub(1);
			}

			let output_start = i32::try_from(tl.chars.len()).expect("text exceeds i32");
			for k in a..end {
				let cp = tl.chars[usize::try_from(k).expect("nonnegative character index")];
				tl.chars.push(cp);
			}
			tl.chars.push(ELLIPSIS);
			tl.ls.push(output_start);
			tl.le
				.push(i32::try_from(tl.chars.len()).expect("text exceeds i32"));
			tl.src_ls.push(src_a);
			tl.src_le
				.push(src_a.wrapping_add(end.wrapping_sub(a)).min(src_b));
			tl.line_w
				.push(slice_w(d, f, size, tracking, &tl.chars, a, end) + ellipsis_width);
			return;
		}
		width += char_width;
		i = i.wrapping_add(1);
	}

	tl.ls.push(a);
	tl.le.push(b);
	tl.src_ls.push(src_a);
	tl.src_le.push(src_b);
	tl.line_w.push(width);
}

/// Greedily wraps one hard line at spaces, hard-breaking words wider than
/// `max_w`. Empty words are preserved by the space-splitting behavior.
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
	let space_width = char_w(d, f, size, tracking, 32);
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

		let word_width = slice_w(d, f, size, tracking, src, word_start, word_end);
		let line_nonempty = i32::try_from(tl.chars.len()).expect("text exceeds i32") > line_start;
		let mut added_width = if line_nonempty {
			word_width + space_width
		} else {
			word_width
		};

		if line_nonempty && line_width + added_width > max_w + WIDTH_EPSILON {
			finish_line(tl, line_start, source_start, word_start.wrapping_sub(1), line_width);
			line_start = i32::try_from(tl.chars.len()).expect("text exceeds i32");
			source_start = word_start;
			line_width = 0.0;
			added_width = word_width;
		}

		if word_width > max_w + WIDTH_EPSILON {
			for k in word_start..word_end {
				let cp = src[usize::try_from(k).expect("nonnegative character index")];
				let char_width = char_w(d, f, size, tracking, cp);
				let line_nonempty =
					i32::try_from(tl.chars.len()).expect("text exceeds i32") > line_start;
				if line_nonempty && line_width + char_width > max_w + WIDTH_EPSILON {
					finish_line(tl, line_start, source_start, k, line_width);
					line_start = i32::try_from(tl.chars.len()).expect("text exceeds i32");
					source_start = k;
					line_width = 0.0;
				}
				tl.chars.push(cp);
				line_width += char_width;
			}
		} else {
			if i32::try_from(tl.chars.len()).expect("text exceeds i32") > line_start {
				tl.chars.push(32);
			}
			for k in word_start..word_end {
				tl.chars
					.push(src[usize::try_from(k).expect("nonnegative character index")]);
			}
			line_width += added_width;
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
	let mut src: Vec<u32> = Vec::with_capacity(text.len());
	src.extend(text.chars().map(u32::from));
	let mut layout = tl_new();
	layout.line_h = line_h(size, leading);
	layout.ascent = ascent(d, f, size, leading);

	// Split on hard newlines before applying wrapping to each hard line.
	let source_len = i32::try_from(src.len()).expect("text exceeds i32");
	let mut hard_start = 0_i32;
	loop {
		let mut hard_end = hard_start;
		while hard_end < source_len
			&& src[usize::try_from(hard_end).expect("nonnegative character index")] != 10
		{
			hard_end = hard_end.wrapping_add(1);
		}

		if wrap {
			wrap_hard(d, f, size, tracking, &mut layout, &src, hard_start, hard_end, max_w);
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
				slice_w(d, f, size, tracking, &src, hard_start, hard_end),
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
			let ellipsis_width = char_w(d, f, size, tracking, ELLIPSIS);
			if layout.line_w[last] + ellipsis_width <= max_w + WIDTH_EPSILON {
				let output_start = i32::try_from(layout.chars.len()).expect("text exceeds i32");
				for k in start..end {
					let cp = layout.chars[usize::try_from(k).expect("nonnegative character index")];
					layout.chars.push(cp);
				}
				layout.chars.push(ELLIPSIS);
				layout.ls[last] = output_start;
				layout.le[last] = i32::try_from(layout.chars.len()).expect("text exceeds i32");
				layout.line_w[last] += ellipsis_width;
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

	shape_layout(d, f, size, tracking, &mut layout);
	layout.w = layout.line_w.iter().copied().fold(0.0_f64, f64::max);
	layout.h = layout.line_h * f64::from(line_count(&layout)).max(1.0);
	layout
}
