//! Bounds-checked string and byte conversion helpers used throughout the
//! kernel.

/// Returns whether two strings contain the same UTF-8 text.
pub fn str_eq(a: &str, b: &str) -> bool {
	a == b
}

/// Concatenates two strings into one allocation.
pub fn str_concat(a: &str, b: &str) -> String {
	let mut result = String::with_capacity(a.len() + b.len());
	result.push_str(a);
	result.push_str(b);
	result
}

/// Returns the number of Unicode scalar values in `s`.
///
/// Panics if the count cannot be represented as an [`i32`].
pub fn str_len(s: &str) -> i32 {
	i32::try_from(s.chars().count()).expect("string has too many codepoints")
}

/// Copies the half-open range of Unicode scalar values `start..end`.
///
/// Panics if either offset is negative, the range is reversed, or either
/// endpoint lies beyond the string.
pub fn str_slice(s: &str, start: i32, end: i32) -> String {
	let start = usize::try_from(start).expect("negative string slice start");
	let end = usize::try_from(end).expect("negative string slice end");
	assert!(start <= end, "string slice start exceeds end");

	let mut boundaries = s
		.char_indices()
		.map(|(byte_offset, _)| byte_offset)
		.chain(std::iter::once(s.len()));
	let start_byte = boundaries.nth(start).expect("string slice out of bounds");
	let end_byte = if start == end {
		start_byte
	} else {
		boundaries
			.nth(end - start - 1)
			.expect("string slice out of bounds")
	};

	s.get(start_byte..end_byte)
		.expect("string slice out of bounds")
		.to_owned()
}

fn codepoint_offset(s: &str, byte: usize) -> i32 {
	i32::try_from(s[..byte].chars().count()).expect("string has too many codepoints")
}

/// Returns the first occurrence of `needle` as a Unicode scalar offset.
///
/// Returns `-1` when `needle` is absent.
pub fn str_find(s: &str, needle: &str) -> i32 {
	s.find(needle).map_or(-1, |byte| codepoint_offset(s, byte))
}

/// Returns the last occurrence of `needle` as a Unicode scalar offset.
///
/// Returns `-1` when `needle` is absent.
pub fn str_rfind(s: &str, needle: &str) -> i32 {
	s.rfind(needle).map_or(-1, |byte| codepoint_offset(s, byte))
}

/// Builds a string from Unicode scalar values.
///
/// Panics if an element is a surrogate or exceeds Unicode's maximum scalar.
pub fn str_from_chars(codepoints: &[u32]) -> String {
	codepoints
		.iter()
		.map(|&codepoint| char::from_u32(codepoint).expect("invalid codepoint"))
		.collect()
}

/// Decodes `bytes[start..end]` as UTF-8, replacing malformed sequences.
///
/// Panics if the range is negative or out of bounds, or if an element cannot
/// be represented as a byte.
pub fn utf8_str(bytes: &[u32], start: i32, end: i32) -> String {
	let start = usize::try_from(start).expect("negative UTF-8 slice start");
	let end = usize::try_from(end).expect("negative UTF-8 slice end");
	let bytes = bytes
		.get(start..end)
		.expect("UTF-8 slice out of bounds")
		.iter()
		.map(|&byte| u8::try_from(byte).expect("byte out of range"))
		.collect::<Vec<_>>();

	String::from_utf8(bytes)
		.unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned())
}
