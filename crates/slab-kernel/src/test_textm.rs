//! Text wrapping edge cases against a synthetic font table: 1000 units per em,
//! mapped letters 500 units wide, spaces and NBSP 250, ellipsis 800, and a
//! default advance of 600. At size 10, these become 5, 2.5, 8, and 6 units.

use crate::{
    slir::{self, Doc},
    textm::{self, TextLayout},
};

/// Builds the synthetic font document used by the text measurement checks.
pub fn font_doc() -> Doc {
    let mut doc = slir::doc_new();
    doc.font_family.push(0);
    doc.font_class.push(0);
    doc.font_weight.push(400);
    doc.font_upem.push(1000);
    doc.font_ascent.push(800);
    doc.font_descent.push(0_i32.wrapping_sub(200));
    doc.font_line_gap.push(0);
    doc.font_default_adv.push(600);
    doc.font_cmap_off.push(0);

    // The cmap is sorted by code point: space, a through c, x, NBSP, ellipsis.
    let code_points = [32, 97, 98, 99, 120, 160, 0x2026];
    let advances = [250, 500, 500, 500, 500, 250, 800];
    doc.font_cmap_len
        .push(i32::try_from(code_points.len()).expect("synthetic cmap fits in i32"));
    for (index, (&code_point, &advance)) in code_points.iter().zip(&advances).enumerate() {
        doc.font_cmap_cp.push(code_point);
        doc.font_cmap_gid
            .push(u32::try_from(index + 1).expect("synthetic glyph identifier fits in u32"));
        doc.font_adv.push(advance);
    }
    doc
}

/// Measures text with the synthetic font and the standard test metrics.
pub fn measure(
    doc: &Doc,
    text: &str,
    max_width: f64,
    wrap: bool,
    ellipsis: bool,
    max_lines: i32,
) -> TextLayout {
    textm::measure_text(
        doc, 0, 10.0, 1.4, 0.0, text, max_width, wrap, ellipsis, max_lines,
    )
}

/// Returns the string represented by one measured line.
pub fn line_str(layout: &TextLayout, line: i32) -> String {
    let line = usize::try_from(line).expect("line index must be nonnegative");
    let start = usize::try_from(layout.ls[line]).expect("line start must be nonnegative");
    let end = usize::try_from(layout.le[line]).expect("line end must be nonnegative");
    crate::rt::str_from_chars(&layout.chars[start..end])
}

/// Checks ordinary word wrapping and the resulting block metrics.
pub fn test_wrap_basic() {
    let doc = font_doc();
    // "aaa bbb ccc": words are 15 units and spaces are 2.5; a 33-unit line
    // fits two words exactly within 32.5 units.
    let layout = measure(&doc, "aaa bbb ccc", 33.0, true, false, -1);
    assert_eq!(layout.ls.len(), 2, "two lines");
    assert_eq!(line_str(&layout, 0), "aaa bbb", "first line words");
    assert_eq!(line_str(&layout, 1), "ccc", "second line word");
    assert_eq!(layout.line_w[0], 32.5, "line 0 width");
    assert_eq!(layout.line_w[1], 15.0, "line 1 width");
    assert_eq!(layout.w, 32.5, "block width");
    assert_eq!(layout.h, 28.0, "two lines at 14u");
    assert!(!layout.truncated, "not truncated");
}

/// Checks that a nonbreaking space does not create a wrapping opportunity.
pub fn test_wrap_nbsp_glue() {
    let doc = font_doc();
    // NBSP is not a break point: "aa\u{00A0}bb" is one 22.5-unit word.
    let layout = measure(&doc, "aa bb cc", 25.0, true, false, -1);
    assert_eq!(layout.ls.len(), 2, "nbsp glues");
    assert_eq!(line_str(&layout, 0), "aa bb", "glued word intact");
    assert_eq!(layout.line_w[0], 22.5, "glued width");
}

/// Checks that an explicit newline starts a new measured line.
pub fn test_hard_newline() {
    let doc = font_doc();
    let layout = measure(&doc, "aa\nbb", 1000.0, true, false, -1);
    assert_eq!(layout.ls.len(), 2, "newline splits");
    assert_eq!(line_str(&layout, 0), "aa", "first hard line");
    assert_eq!(line_str(&layout, 1), "bb", "second hard line");
}

/// Checks per-character hard breaks when a word exceeds the line width.
pub fn test_hard_break_long_word() {
    let doc = font_doc();
    // "aaaaa" is 25 units; a 12-unit maximum breaks it as aa|aa|a.
    let layout = measure(&doc, "aaaaa", 12.0, true, false, -1);
    assert_eq!(layout.ls.len(), 3, "hard-broken into 3");
    assert_eq!(line_str(&layout, 0), "aa", "chunk 1");
    assert_eq!(line_str(&layout, 1), "aa", "chunk 2");
    assert_eq!(line_str(&layout, 2), "a", "chunk 3");
}

/// Checks clipping and ellipsis insertion when wrapping is disabled.
pub fn test_nowrap_ellipsis() {
    let doc = font_doc();
    // "abcabc" is 30 units; within a 20-unit maximum, "ab" (10) plus the
    // ellipsis (8) fits, but adding "c" would require 23 units.
    let layout = measure(&doc, "abcabc", 20.0, false, true, -1);
    assert_eq!(layout.ls.len(), 1, "one line");
    assert!(layout.truncated, "truncated");
    assert_eq!(line_str(&layout, 0), "ab…", "ellipsis cut");
    assert_eq!(layout.line_w[0], 18.0, "cut width 10 + 8");
}

/// Checks that no-wrap truncation retains text when ellipsis is disabled.
pub fn test_nowrap_clipped_no_ellipsis() {
    let doc = font_doc();
    let layout = measure(&doc, "abcabc", 20.0, false, false, -1);
    assert!(layout.truncated, "flagged truncated");
    assert_eq!(line_str(&layout, 0), "abcabc", "text kept without ellipsis");
}

/// Checks that wrapping respects the maximum line count.
pub fn test_max_lines() {
    let doc = font_doc();
    let layout = measure(&doc, "aaa bbb ccc", 16.0, true, false, 2);
    assert_eq!(layout.ls.len(), 2, "clamped to 2 lines");
    assert!(layout.truncated, "truncated by max_lines");
}

/// Checks that max-line truncation appends an ellipsis when it fits.
pub fn test_max_lines_ellipsis_appends() {
    let doc = font_doc();
    // Each word occupies its own line. With one line allowed, the ellipsis
    // appends to "aaa" because 15 + 8 = 23 is at most 26.
    let layout = measure(&doc, "aaa bbb", 26.0, true, true, 1);
    assert_eq!(layout.ls.len(), 1, "one line kept");
    assert_eq!(line_str(&layout, 0), "aaa…", "ellipsis appended");
    assert_eq!(layout.line_w[0], 23.0, "appended width");
}

/// Checks fallback advance and tracking for measured glyphs.
pub fn test_default_advance() {
    let doc = font_doc();
    // 'z' (U+007A) is absent from the cmap, so the 600-unit default advance
    // becomes 6 units at size 10.
    assert_eq!(
        textm::char_w(&doc, 0, 10.0, 0.0, 122),
        6.0,
        "default advance"
    );
    assert_eq!(
        textm::char_w(&doc, 0, 10.0, 1.5, 97),
        6.5,
        "tracking after glyph"
    );
}

/// Checks line metrics, half-leading ascent, and empty-text dimensions.
pub fn test_metrics() {
    let doc = font_doc();
    assert_eq!(textm::line_h(10.0, 1.4), 14.0, "line height");
    // Ascent is 8 + (14 - 10) / 2 = 10: half-leading over the 1000-upem box.
    assert_eq!(
        textm::ascent(&doc, 0, 10.0, 1.4),
        10.0,
        "half-leading ascent"
    );
    // Empty text still occupies one line.
    let layout = measure(&doc, "", 100.0, true, false, -1);
    assert_eq!(layout.ls.len(), 1, "empty text one line");
    assert_eq!(layout.h, 14.0, "empty text height");
    assert_eq!(layout.w, 0.0, "empty text width");
}
