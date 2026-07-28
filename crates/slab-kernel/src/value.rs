//! Decoding and canonical rendering for attribute values.
//!
//! AVAL stores its value variants in parallel pools. [`V`] is the flat,
//! tagged view consumed by style resolution and other kernel subsystems;
//! consumers distinguish variants through [`V::tag`]. SLIR tags are reused,
//! with [`V_MISSING`] representing an attribute that is not present.

use crate::slir::{
	Doc, T_NUM, T_PCT, T_SHADOW_LIST, T_SIZE_FILL, T_SIZE_FIXED, T_SIZE_PCT, T_TUPLE, T_TUPLE_DYN,
};

/// Tag used when an attribute has no AVAL entry.
///
/// Deliberately outside the SLIR tag range: reusing a live tag (this was
/// `16`, which `T_PROP_REF` later claimed) makes every missing attribute on
/// a synthetic list node read as a field-0 property reference.
pub const V_MISSING: u32 = u32::MAX;

/// A decoded, tagged AVAL value.
#[derive(Clone, Copy, Debug)]
pub struct V {
	/// SLIR value tag, or [`V_MISSING`].
	pub tag: u32,
	/// Payload for numbers, percentages, and fixed, fill, or percentage sizes.
	pub num: f64,
	/// Handle payload, such as a string reference, color, pool index, or enum
	/// symbol.
	pub h:   u32,
	/// Tuple offset in `Doc::f64s`, dynamic-tuple offset in `Doc::tup_dyn_*`,
	/// or shadow-list offset in the shadow pool.
	pub off: i32,
	/// Number of tuple or shadow-list entries.
	pub ln:  i32,
}

/// Returns the sentinel value for an attribute that is not present.
pub const fn missing() -> V {
	V { tag: V_MISSING, num: 0.0, h: 0, off: 0, ln: 0 }
}

/// Decodes AVAL entry `ix`; a negative or out-of-range index is missing.
pub fn decode(d: &Doc, ix: i32) -> V {
	let Ok(index) = usize::try_from(ix) else {
		return missing();
	};
	if index >= d.aval_tag.len() {
		return missing();
	}

	let tag = d.aval_tag[index];
	if matches!(tag, T_TUPLE | T_SHADOW_LIST | T_TUPLE_DYN) {
		return V {
			tag,
			num: 0.0,
			h: 0,
			off: i32::from_ne_bytes(d.aval_lo[index].to_ne_bytes()),
			ln: i32::from_ne_bytes(d.aval_hi[index].to_ne_bytes()),
		};
	}

	V { tag, num: d.aval_num[index], h: d.aval_lo[index], off: 0, ln: 0 }
}

fn token_theme_entry(d: &Doc, theme_index: u32, token_index: usize) -> Option<usize> {
	let theme_row = usize::try_from(theme_index.checked_sub(1)?).ok()?;
	let theme_name = *d.theme_name.get(theme_row)?;
	let start = usize::try_from(*d.token_theme_off.get(token_index)?).ok()?;
	let length = usize::try_from(*d.token_theme_len.get(token_index)?).ok()?;
	let end = start.checked_add(length)?.min(d.token_theme_name.len());

	// The compiler writes complete tables in theme declaration order. Keep a
	// checked fallback for hand-built/forward-compatible documents.
	let direct = start.checked_add(theme_row)?;
	if direct < end && d.token_theme_name[direct] == theme_name {
		return Some(direct);
	}
	(start..end).find(|&index| d.token_theme_name[index] == theme_name)
}

/// Returns the concrete AVAL index for one token row in the active theme.
///
/// `theme_index` is zero for authored base and otherwise one plus the index in
/// `Doc::theme_name`. Missing overrides fall back to the authored base.
pub fn token_aval(d: &Doc, theme_index: u32, token: u32) -> i32 {
	let Ok(token_index) = usize::try_from(token) else {
		return -1;
	};
	let Some(&base) = d.token_base.get(token_index) else {
		return -1;
	};
	if theme_index == 0 {
		return i32::from_ne_bytes(base.to_ne_bytes());
	}
	if let Some(index) = token_theme_entry(d, theme_index, token_index) {
		return i32::from_ne_bytes(d.token_theme_val[index].to_ne_bytes());
	}
	i32::from_ne_bytes(base.to_ne_bytes())
}

/// Returns the canonical host-text STRS reference for one active token value.
pub fn token_repr(d: &Doc, theme_index: u32, token: u32) -> Option<u32> {
	let token_index = usize::try_from(token).ok()?;
	let base = *d.token_base_repr.get(token_index)?;
	if theme_index == 0 {
		return Some(base);
	}
	let index = token_theme_entry(d, theme_index, token_index);
	index
		.and_then(|index| d.token_theme_repr.get(index).copied())
		.or(Some(base))
}

/// Decodes an AVAL and follows token references through the active theme.
pub fn decode_active(d: &Doc, theme_index: u32, ix: i32) -> V {
	let mut value = decode(d, ix);
	let mut depth = 0;
	while value.tag == crate::slir::T_TOKEN_REF && depth < 64 {
		value = decode(d, token_aval(d, theme_index, value.h));
		depth += 1;
	}
	value
}

/// Returns the payload of a numeric or size value, or `fallback` for any other
/// tag.
pub const fn num_of(v: &V, fallback: f64) -> f64 {
	if matches!(v.tag, T_NUM | T_PCT | T_SIZE_FIXED | T_SIZE_FILL | T_SIZE_PCT) {
		v.num
	} else {
		fallback
	}
}

/// Returns tuple element `k`, or `0.0` when `v` is not a tuple or `k` is out of
/// range.
pub fn tuple_at(d: &Doc, v: &V, k: i32) -> f64 {
	if v.tag != T_TUPLE || k < 0 || k >= v.ln {
		return 0.0;
	}

	let index = usize::try_from(v.off.wrapping_add(k)).expect("tuple offset must be non-negative");
	d.f64s[index]
}

/// Renders a float canonically for frame JSON and diagnostics.
///
/// Values are rounded half to even at three decimal places, trailing zeros
/// are removed, and negative zero is rendered as `"0"`. Digits are emitted
/// from integer-valued arithmetic rather than host float formatting to keep
/// frame output deterministic. Panics for NaN, infinity, and magnitudes at
/// least $10^9$, which are outside the solver's output domain.
pub fn fmt3(v: f64) -> String {
	assert!(!v.is_nan(), "fmt3: NaN");
	assert!(v.abs() < 1_000_000_000.0, "fmt3: out of range");

	let scaled = v * 1_000.0;
	let floor = scaled.floor();
	let distance = scaled - floor;
	// On an exact tie, retain an even floor and increment an odd floor.
	let round_up = distance > 0.5 || (distance == 0.5 && (floor / 2.0).floor() * 2.0 != floor);
	let rounded = if round_up { floor + 1.0 } else { floor };
	if rounded == 0.0 {
		return "0".to_owned();
	}

	let mut output = String::with_capacity(15);
	let magnitude = if rounded < 0.0 {
		output.push('-');
		-rounded
	} else {
		rounded
	};
	let whole = (magnitude / 1_000.0).floor();
	let fraction = magnitude - whole * 1_000.0;
	push_decimal_integer(&mut output, whole);

	if fraction != 0.0 {
		output.push('.');
		let hundreds = (fraction / 100.0).floor();
		let tens = ((fraction / 10.0).floor()) - hundreds * 10.0;
		let ones = fraction - (hundreds * 100.0 + tens * 10.0);
		push_decimal_digit(&mut output, hundreds);
		if tens != 0.0 || ones != 0.0 {
			push_decimal_digit(&mut output, tens);
		}
		if ones != 0.0 {
			push_decimal_digit(&mut output, ones);
		}
	}

	output
}

// Emits a non-negative, integer-valued float without float formatting.
fn push_decimal_integer(output: &mut String, mut value: f64) {
	let mut place = 1_000_000_000.0;
	let mut emitted = false;

	loop {
		let digit = (value / place).floor();
		if emitted || digit != 0.0 || place == 1.0 {
			push_decimal_digit(output, digit);
			emitted = true;
		}
		value -= digit * place;

		if place == 1.0 {
			break;
		}
		place /= 10.0;
	}
}

// Emits one integer-valued decimal digit.
fn push_decimal_digit(output: &mut String, digit: f64) {
	let character = if digit == 0.0 {
		'0'
	} else if digit == 1.0 {
		'1'
	} else if digit == 2.0 {
		'2'
	} else if digit == 3.0 {
		'3'
	} else if digit == 4.0 {
		'4'
	} else if digit == 5.0 {
		'5'
	} else if digit == 6.0 {
		'6'
	} else if digit == 7.0 {
		'7'
	} else if digit == 8.0 {
		'8'
	} else if digit == 9.0 {
		'9'
	} else {
		unreachable!("decimal digit must be an integer from zero through nine");
	};
	output.push(character);
}
