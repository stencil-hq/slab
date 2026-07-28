//! Grapheme-cluster boundaries for combining marks, ZWJ emoji sequences,
//! regional-indicator flag pairs, CRLF, and variation selectors.

use crate::graphemes;

/// Appends the grapheme boundaries for `text` to `out`.
pub fn bounds_of(text: &str, out: &mut Vec<i32>) {
	graphemes::boundaries(text, out);
}

/// Checks that a combining mark remains attached to its base character.
pub fn test_combining_mark_clusters() {
	// "ae\u{301}b": a | e+acute | b
	let mut boundaries = Vec::new();
	bounds_of("aéb", &mut boundaries);
	assert_eq!(boundaries.len(), 4, "aeb bounds count");
	assert_eq!(boundaries, [0, 1, 3, 4], "aeb bounds");
}

/// Checks that a family joined by ZWJ characters is one cluster.
pub fn test_zwj_family_is_one_cluster() {
	// x 👩 ZWJ 👩 ZWJ 👧 y
	let mut boundaries = Vec::new();
	bounds_of("x👩‍👩‍👧y", &mut boundaries);
	assert_eq!(boundaries.len(), 4, "family bounds count");
	assert_eq!(boundaries, [0, 1, 6, 7], "family bounds");
}

/// Checks that regional indicators form pairs rather than one long cluster.
pub fn test_flag_pairs_split_in_twos() {
	// 🇩 🇪 🇫 🇷 form two flags of exactly two regional indicators each.
	let mut boundaries = Vec::new();
	bounds_of("🇩🇪🇫🇷", &mut boundaries);
	assert_eq!(boundaries.len(), 3, "flag bounds count");
	assert_eq!(boundaries, [0, 2, 4], "flag pair split");
}

/// Checks that CRLF is treated as one grapheme cluster.
pub fn test_crlf_is_one_cluster() {
	let mut boundaries = Vec::new();
	bounds_of("a\r\nb", &mut boundaries);
	assert_eq!(boundaries.len(), 4, "crlf bounds count");
	assert_eq!(&boundaries[1..=2], [1, 3], "crlf joined");
}

/// Checks that variation selector 16 remains attached to the preceding heart.
pub fn test_variation_selector_attaches() {
	// a ❤ VS16 b
	let mut boundaries = Vec::new();
	bounds_of("a❤️b", &mut boundaries);
	assert_eq!(boundaries.len(), 4, "vs bounds count");
	assert_eq!(&boundaries[1..=2], [1, 3], "vs joined");
}

/// Checks that empty text has only its zero boundary.
pub fn test_empty_text() {
	let mut boundaries = Vec::new();
	bounds_of("", &mut boundaries);
	assert_eq!(boundaries, [0], "empty -> [0]");
}

/// Checks navigation between cluster boundaries and character classification.
pub fn test_boundary_navigation() {
	let mut boundaries = Vec::new();
	bounds_of("aéb", &mut boundaries);
	assert_eq!(graphemes::prev_boundary(&boundaries, 4), 3, "prev from end");
	assert_eq!(graphemes::prev_boundary(&boundaries, 3), 1, "prev skips mark");
	assert_eq!(graphemes::prev_boundary(&boundaries, 1), 0, "prev to start");
	assert_eq!(graphemes::next_boundary(&boundaries, 1, 4), 3, "next skips mark");
	assert_eq!(graphemes::next_boundary(&boundaries, 3, 4), 4, "next to end");
	assert!(graphemes::is_mark(0x301), "U+0301 is Mn");
	assert!(!graphemes::is_mark(97), "a is not a mark");
	assert!(graphemes::is_ri(0x1f1e6) && !graphemes::is_ri(0x1f1e5), "RI range edges");
}
