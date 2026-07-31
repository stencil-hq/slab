//! Shared immutable text content addressed by codepoint offsets.
//!
//! [`Text`] is the kernel's canonical text representation between the edit
//! model, field content overrides, resolved styles, and text measurement:
//! one reference-counted codepoint buffer, cloned in O(1) and mutated
//! copy-on-write by its single writer (the edit model). UTF-8 strings exist
//! only at host boundaries — events in, signals, parameters, and dumps out —
//! so keystrokes never re-scan a whole document to convert offsets.

use std::rc::Rc;

/// Reference-counted codepoint text.
///
/// Equality compares content, with a pointer fast path for shared buffers;
/// all offsets are codepoint indices, matching every caret, selection, span,
/// and layout offset in the kernel.
#[derive(Clone, Debug, Default)]
pub struct Text(Rc<Vec<u32>>);

impl From<&str> for Text {
	/// Builds text from UTF-8, one codepoint per Unicode scalar value.
	fn from(text: &str) -> Self {
		let mut cps = Vec::with_capacity(text.len());
		cps.extend(text.chars().map(u32::from));
		Self(Rc::new(cps))
	}
}

impl Text {
	/// Wraps an owned codepoint buffer without copying.
	pub fn from_cps(cps: Vec<u32>) -> Self {
		Self(Rc::new(cps))
	}

	/// Returns the codepoints.
	pub fn cps(&self) -> &[u32] {
		&self.0
	}

	/// Returns the length in codepoints.
	///
	/// Panics if the length cannot be represented as an [`i32`].
	pub fn len(&self) -> i32 {
		i32::try_from(self.0.len()).expect("text has too many codepoints")
	}

	/// Reports whether the text is empty.
	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}

	/// Returns a mutable buffer, copying only when the buffer is shared.
	pub fn make_mut(&mut self) -> &mut Vec<u32> {
		Rc::make_mut(&mut self.0)
	}

	/// Copies the half-open codepoint range `start..end`.
	///
	/// Panics if the range is reversed or out of bounds.
	pub fn slice_cps(&self, start: i32, end: i32) -> Vec<u32> {
		let start = usize::try_from(start).expect("negative text slice start");
		let end = usize::try_from(end).expect("negative text slice end");
		self
			.0
			.get(start..end)
			.expect("text slice out of bounds")
			.to_vec()
	}

	/// Returns the text as UTF-8.
	///
	/// Maximal ASCII runs bulk-copy instead of encoding per char, so
	/// editor-sized change payloads materialize at near-memcpy speed.
	pub fn to_utf8(&self) -> String {
		let cps: &[u32] = &self.0;
		let mut bytes = Vec::with_capacity(cps.len());
		let mut index = 0;
		while index < cps.len() {
			if cps[index] < 0x80 {
				let run = index;
				while index < cps.len() && cps[index] < 0x80 {
					index += 1;
				}
				#[allow(clippy::cast_possible_truncation, reason = "run values are ASCII")]
				bytes.extend(cps[run..index].iter().map(|&cp| cp as u8));
			} else {
				let ch = char::from_u32(cps[index]).expect("invalid codepoint");
				let mut buf = [0u8; 4];
				bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
				index += 1;
			}
		}
		String::from_utf8(bytes).expect("codepoints encode to valid UTF-8")
	}

	/// Replaces codepoints `start..end` with `insert`.
	///
	/// A uniquely held buffer splices in place (one tail move); a shared
	/// buffer is rebuilt in a single pass instead of copy-then-splice.
	pub fn splice(&mut self, start: i32, end: i32, insert: &[u32]) {
		let start = usize::try_from(start).expect("negative splice start");
		let end = usize::try_from(end).expect("negative splice end");
		if let Some(cps) = Rc::get_mut(&mut self.0) {
			cps.splice(start..end, insert.iter().copied());
			return;
		}
		let cps = &self.0;
		let mut next = Vec::with_capacity(cps.len() - (end - start) + insert.len());
		next.extend_from_slice(&cps[..start]);
		next.extend_from_slice(insert);
		next.extend_from_slice(&cps[end..]);
		self.0 = Rc::new(next);
	}
}

impl PartialEq for Text {
	fn eq(&self, other: &Self) -> bool {
		Rc::ptr_eq(&self.0, &other.0) || self.0 == other.0
	}
}

impl Eq for Text {}

impl PartialEq<str> for Text {
	fn eq(&self, other: &str) -> bool {
		cps_eq_str(&self.0, other)
	}
}

impl PartialEq<&str> for Text {
	fn eq(&self, other: &&str) -> bool {
		cps_eq_str(&self.0, other)
	}
}

impl PartialEq<String> for Text {
	fn eq(&self, other: &String) -> bool {
		cps_eq_str(&self.0, other)
	}
}

/// Compares a codepoint slice with UTF-8 text without allocating.
pub fn cps_eq_str(cps: &[u32], text: &str) -> bool {
	let mut chars = text.chars();
	for &cp in cps {
		if chars.next().map(u32::from) != Some(cp) {
			return false;
		}
	}
	chars.next().is_none()
}
