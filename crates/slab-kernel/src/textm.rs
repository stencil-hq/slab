//! Text measurement using only the SLIR font tables; host metrics never enter
//! the calculation. Wrapping follows the original `VectorMetrics` behavior:
//! spaces split words (including empty words), non-breaking spaces remain glued,
//! overlong words hard-break, and ellipsis truncation strips trailing whitespace.
//! These details are part of the conformance contract.
//!
//! Advances come from the selected font table as
//! `advance(glyph) * size / units_per_em + tracking`, with the default advance
//! used for codepoints missing from the cmap. Font weight is represented by a
//! real table rather than an artificial width multiplier.
//!
//! Ascent uses CSS half-leading over the hhea box:
//! `ascent * size / units_per_em + (line_height - (ascent - descent) * size / units_per_em) / 2`.
//! This keeps kernel baselines aligned with browser-painted glyphs.

use crate::slir::{self, Doc};

/// Unicode codepoint inserted when a line is ellipsized.
pub const ELLIPSIS: u32 = 0x2026;
/// Unicode non-breaking-space codepoint.
pub const NBSP: u32 = 0xA0;
const WIDTH_EPSILON: f64 = 1e-6;

/// A measured and optionally wrapped text layout.
///
/// Output lines are slices `chars[ls[i]..le[i]]`. `src_ls` and `src_le` hold
/// the corresponding codepoint offsets in the original string. They remain
/// source offsets when wrapping normalizes the output or ellipsis synthesizes
/// characters.
#[derive(Clone, Debug)]
pub struct TextLayout {
    /// Codepoints for all output lines.
    pub chars: Vec<u32>,
    /// Inclusive output start offset for each line.
    pub ls: Vec<i32>,
    /// Exclusive output end offset for each line.
    pub le: Vec<i32>,
    /// Inclusive original-text codepoint offset for each line.
    pub src_ls: Vec<i32>,
    /// Exclusive original-text codepoint offset for each line.
    pub src_le: Vec<i32>,
    /// Measured advance for each line.
    pub line_w: Vec<f64>,
    /// Maximum measured line width.
    pub w: f64,
    /// Total layout height.
    pub h: f64,
    /// First-baseline offset from a line's top.
    pub ascent: f64,
    /// Height of one line.
    pub line_h: f64,
    /// Whether line or width limits removed or replaced any text.
    pub truncated: bool,
}

/// Creates an empty text layout.
pub fn tl_new() -> TextLayout {
    TextLayout {
        chars: Vec::new(),
        ls: Vec::new(),
        le: Vec::new(),
        src_ls: Vec::new(),
        src_le: Vec::new(),
        line_w: Vec::new(),
        w: 0.0,
        h: 0.0,
        ascent: 0.0,
        line_h: 0.0,
        truncated: false,
    }
}

/// Returns the number of output lines.
pub fn line_count(tl: &TextLayout) -> i32 {
    i32::try_from(tl.ls.len()).expect("text line count exceeds i32")
}

/// Returns one codepoint's advance, including letter spacing after the glyph,
/// matching CSS letter-spacing semantics.
pub fn char_w(d: &Doc, f: i32, size: f64, tracking: f64, cp: u32) -> f64 {
    if f < 0 {
        return 0.6 * size + tracking;
    }

    let font = usize::try_from(f).expect("nonnegative font index");
    let upem = f64::from(d.font_upem[font]);
    if upem <= 0.0 {
        return 0.6 * size + tracking;
    }

    f64::from(slir::font_advance_units(d, f, cp)) * size / upem + tracking
}

/// Measures a codepoint slice without allocating another character buffer.
pub fn slice_w(d: &Doc, f: i32, size: f64, tracking: f64, chars: &[u32], a: i32, b: i32) -> f64 {
    let mut width = 0.0;
    for i in a..b {
        width += char_w(
            d,
            f,
            size,
            tracking,
            chars[usize::try_from(i).expect("nonnegative character index")],
        );
    }
    width
}

/// Measures a string's codepoint slice without materializing codepoints.
pub fn str_slice_w(d: &Doc, f: i32, size: f64, tracking: f64, text: &str, a: i32, b: i32) -> f64 {
    if a < 0 || b < a {
        panic!("invalid string slice");
    }

    let mut width = 0.0;
    let mut i = 0_i32;
    for cp in text.chars().map(u32::from) {
        if i >= b {
            break;
        }
        if i >= a {
            width += char_w(d, f, size, tracking, cp);
        }
        i = i.wrapping_add(1);
    }
    if i < b {
        panic!("string slice out of bounds");
    }
    width
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

/// Returns whether ellipsis truncation may strip this codepoint.
pub fn is_strippable(cp: u32) -> bool {
    matches!(cp, 32 | 9 | 10 | 13 | NBSP)
}

/// Truncates `chars[a..b]` with an ellipsis. Source offsets continue to refer
/// to the original line rather than the synthesized output buffer.
// The arguments are the complete text-metric and source-range inputs to this
// standalone kernel operation; grouping them would only move the same data.
#[allow(clippy::too_many_arguments)]
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
                    tl.chars[usize::try_from(end.wrapping_sub(1))
                        .expect("nonnegative character index")],
                )
            {
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
#[allow(clippy::too_many_arguments)]
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
            finish_line(
                tl,
                line_start,
                source_start,
                word_start.wrapping_sub(1),
                line_width,
            );
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
#[allow(clippy::too_many_arguments)]
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
    let src: Vec<u32> = text.chars().map(u32::from).collect();
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
            wrap_hard(
                d,
                f,
                size,
                tracking,
                &mut layout,
                &src,
                hard_start,
                hard_end,
                max_w,
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

    layout.w = layout.line_w.iter().copied().fold(0.0_f64, f64::max);
    layout.h = layout.line_h * f64::from(line_count(&layout)).max(1.0);
    layout
}
