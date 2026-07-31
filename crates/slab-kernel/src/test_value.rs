//! AVAL decoding and `f64` bit decoding against hand-built document fixtures.

use crate::{
	frame,
	params::ParamStore,
	rt,
	slir::{self, Doc},
	style, value,
};

/// Builds the compact value pools shared by the AVAL decoding cases.
pub fn fixture() -> Doc {
	let mut doc = slir::doc_new();

	// 0: Num 42.5; 1: SizeFill 2; 2: Tuple [3, 4) with length 2; 3: Color.
	doc.aval_tag.push(slir::T_NUM);
	doc.aval_lo.push(0);
	doc.aval_hi.push(0);
	doc.aval_num.push(42.5);
	doc.aval_tag.push(slir::T_SIZE_FILL);
	doc.aval_lo.push(0);
	doc.aval_hi.push(0);
	doc.aval_num.push(2.0);
	doc.aval_tag.push(slir::T_TUPLE);
	doc.aval_lo.push(1);
	doc.aval_hi.push(2);
	doc.aval_num.push(0.0);
	doc.aval_tag.push(slir::T_COLOR);
	doc.aval_lo.push(0x80ff7f33);
	doc.aval_hi.push(0);
	doc.aval_num.push(0.0);
	doc.f64s.extend([9.0, 16.0, 25.0]);
	doc
}

/// Verifies every fixture value shape, tuple bounds, and missing-value
/// fallback.
pub fn test_value_decode() {
	let doc = fixture();

	let number = value::decode(&doc, 0);
	assert_eq!(number.tag, slir::T_NUM, "num tag");
	assert_eq!(number.num, 42.5, "num payload");
	assert_eq!(value::num_of(&number, 0.0), 42.5, "num_of");

	let fill = value::decode(&doc, 1);
	assert_eq!(fill.tag, slir::T_SIZE_FILL, "fill tag");
	assert_eq!(fill.num, 2.0, "fill weight");

	let tuple = value::decode(&doc, 2);
	assert_eq!(tuple.tag, slir::T_TUPLE, "tuple tag");
	assert!(tuple.off == 1 && tuple.ln == 2, "tuple slice");
	assert_eq!(value::tuple_at(&doc, &tuple, 0), 16.0, "tuple[0]");
	assert_eq!(value::tuple_at(&doc, &tuple, 1), 25.0, "tuple[1]");
	assert_eq!(value::tuple_at(&doc, &tuple, 2), 0.0, "tuple oob -> 0");

	let color = value::decode(&doc, 3);
	assert_eq!(color.tag, slir::T_COLOR, "color tag");
	assert_eq!(color.h, 0x80ff7f33, "color handle");

	let missing = value::decode(&doc, -1);
	assert_eq!(missing.tag, value::V_MISSING, "missing");
	assert_eq!(value::num_of(&missing, 7.0), 7.0, "missing fallback");
}

/// Verifies allocation-free public token lookup and named-theme/base fallback.
pub fn test_active_theme_token_lookup() {
	let mut doc = slir::doc_new();
	doc.ok = true;
	doc.strs = vec![
		String::new(),
		"dusk".to_string(),
		"space.unit".to_string(),
		"8".to_string(),
		"12".to_string(),
	];
	doc.aval_tag.extend([slir::T_NUM, slir::T_NUM]);
	doc.aval_lo.extend([0, 0]);
	doc.aval_hi.extend([0, 0]);
	doc.aval_num.extend([8.0, 12.0]);
	doc.theme_name.push(1);
	doc.token_name.push(2);
	doc.token_base.push(0);
	doc.token_base_repr.push(3);
	doc.token_theme_off.push(0);
	doc.token_theme_len.push(1);
	doc.token_theme_name.push(1);
	doc.token_theme_val.push(1);
	doc.token_theme_repr.push(4);

	let mut instance = crate::frame::inst_shell();
	instance.doc = doc;
	crate::frame::inst_init(&mut instance);
	assert_eq!(
		crate::frame::inst_get_token(&instance, "space.unit"),
		Some(crate::frame::TokenValue::Number(8.0))
	);
	assert!(crate::frame::inst_set_theme(&mut instance, "dusk"));
	assert_eq!(
		crate::frame::inst_get_token(&instance, "space.unit"),
		Some(crate::frame::TokenValue::Number(12.0))
	);
	assert!(crate::frame::inst_set_theme(&mut instance, ""));
	assert_eq!(
		crate::frame::inst_get_token(&instance, "space.unit"),
		Some(crate::frame::TokenValue::Number(8.0))
	);
	assert_eq!(crate::frame::inst_get_token(&instance, "missing"), None);
}

/// Verifies reconstruction of representative IEEE-754 bit patterns.
pub fn test_f64_bits() {
	assert_eq!(f64::from_bits(0x0000000000000000), 0.0, "zero");
	assert_eq!(f64::from_bits(0x3ff0000000000000), 1.0, "one");
	assert_eq!(f64::from_bits(0xc004000000000000), -2.5, "-2.5");
	assert_eq!(f64::from_bits(0x3fb999999999999a), 0.1, "0.1 exact bits");
	assert_eq!(f64::from_bits(0x4059000000000000), 100.0, "100");

	// The smallest subnormal is positive and tiny.
	let subnormal = f64::from_bits(1);
	assert!(subnormal > 0.0, "subnormal positive");
	assert!(subnormal < 1e-300, "subnormal tiny");

	// Negative zero compares equal to zero.
	assert_eq!(f64::from_bits(0x8000000000000000), 0.0, "-0 == 0");
}

/// Verifies decimal formatting of representative `u32` values.
pub fn test_fmt_u32() {
	assert!(rt::str_eq(&0_u32.to_string(), "0"), "0");
	assert!(rt::str_eq(&1_234_567_890_u32.to_string(), "1234567890"), "big");
	assert!(rt::str_eq(&7_u32.to_string(), "7"), "7");
}

/// Verifies ranged UTF-8 decoding and malformed-byte replacement.
pub fn test_utf8_str() {
	let bytes = vec![120, 0xf0, 0x9f, 0x99, 0x82, 121];
	assert!(rt::str_eq(&rt::utf8_str(&bytes, 1, 5), "🙂"), "decodes selected UTF-8 range");
	assert!(rt::str_eq(&rt::utf8_str(&[0xff], 0, 1), "�"), "replaces malformed UTF-8");
}

/// Verifies that string helpers consistently use codepoint offsets.
pub fn test_string_ops() {
	let mut count = 0_i32;
	let mut sum = 0_u32;
	for codepoint in "A🙂".chars().map(u32::from) {
		count = count.wrapping_add(1);
		sum = sum.wrapping_add(codepoint);
	}

	assert!(count == 2 && sum == 65_u32.wrapping_add(0x1f642), "string loop yields codepoints");
	assert_eq!(rt::str_len("a🙂b🙂"), 4, "codepoint length");
	assert!(rt::str_eq(&rt::str_slice("a🙂b", 1, 3), "🙂b"), "codepoint slice");
	assert!(rt::str_eq("\t value".trim_start(), "value"), "trim start");
	assert!(rt::str_eq("value \t".trim_end(), "value"), "trim end");
	assert_eq!(rt::str_find("a🙂b🙂", "🙂"), 1, "find codepoint offset");
	assert_eq!(rt::str_rfind("a🙂b🙂", "🙂"), 3, "rfind codepoint offset");
}

/// Verifies wrapping signed and unsigned division and remainder behavior.
pub fn test_integer_power_of_two_arithmetic() {
	let positive = 7_i32;
	let negative = -7_i32;
	assert_eq!(positive.wrapping_div(4), 1, "positive signed power-of-two division");
	assert_eq!(positive.wrapping_rem(4), 3, "positive signed power-of-two remainder");
	assert_eq!(negative.wrapping_div(4), -1, "negative signed division truncates toward zero");
	assert_eq!(negative.wrapping_rem(4), -3, "negative signed remainder keeps its sign");
	assert!(negative.wrapping_div(1) == -7 && negative.wrapping_rem(1) == 0, "2^0 divisor");

	let min = i32::MIN;
	assert_eq!(min.wrapping_div(1_073_741_824), -2, "largest signed power-of-two divisor");
	assert_eq!(min.wrapping_rem(1_073_741_824), 0, "largest signed power-of-two remainder");

	let unsigned = 0xffff_ffff_u32;
	assert_eq!(unsigned.wrapping_div(0x8000_0000), 1, "largest unsigned power-of-two divisor");
	assert_eq!(unsigned.wrapping_rem(0x8000_0000), 0x7fff_ffff, "unsigned remainder mask");

	assert_eq!(negative.wrapping_div(3), -2, "non-power-of-two division fallback");
	assert_eq!(negative.wrapping_rem(3), -1, "non-power-of-two remainder fallback");
}

/// Verifies that dynamic-tuple members read literals from the pool and
/// track the current num/pct param values between solves.
pub fn test_tuple_dyn_members_track_params() {
	let mut doc = slir::doc_new();
	doc.strs.extend([String::new(), "px".into()]);
	doc.parm_name.push(1);
	doc.parm_type.push(slir::PARAM_NUM);
	doc.parm_default
		.push(u32::from_ne_bytes((-1i32).to_ne_bytes()));
	doc.parm_enum_off.push(0);
	doc.parm_enum_len.push(0);
	doc.parm_site_off.push(0);
	doc.parm_site_len.push(0);
	// AVAL 0: TupleDyn [Lit 5, Param 0].
	doc.aval_tag.push(slir::T_TUPLE_DYN);
	doc.aval_lo.push(0);
	doc.aval_hi.push(2);
	doc.aval_num.push(0.0);
	doc.tup_dyn_tag.extend([0, 1]);
	doc.tup_dyn_num.extend([5.0, 0.0]);
	doc.tup_dyn_param.extend([0, 0]);

	let mut st = style::st_new();
	style::init_params(&doc, &mut st);
	assert!(st.params.set_number(0, 30.0));

	let v = value::decode(&doc, 0);
	assert_eq!(v.tag, slir::T_TUPLE_DYN, "tuple-dyn tag survives decode");
	assert!(v.off == 0 && v.ln == 2, "tuple-dyn slice");
	assert!(style::is_tuple_v(v.tag), "tuple-dyn counts as a tuple");
	assert_eq!(style::tup_at(&doc, &st, &v, 0), 5.0, "literal member");
	assert_eq!(style::tup_at(&doc, &st, &v, 1), 30.0, "param member");
	assert!(st.params.set_number(0, 42.0));
	assert_eq!(style::tup_at(&doc, &st, &v, 1), 42.0, "param member tracks the current value");
	assert_eq!(style::tup_at(&doc, &st, &v, 2), 0.0, "oob member -> 0");
}

/// Verifies packed Boolean parameters on both sides of a word boundary,
/// including retained defaults, no-op writes, and isolation from numeric
/// parameters.
pub fn test_boolean_params_cross_word_boundary() {
	let mut doc = slir::doc_new();
	doc.ok = true;
	doc.strs.push(String::new());

	let false_default =
		u32::try_from(doc.aval_tag.len()).expect("fixture attribute count fits in u32");
	doc.aval_tag.push(slir::T_NUM);
	doc.aval_lo.push(0);
	doc.aval_hi.push(0);
	doc.aval_num.push(0.0);
	let true_default =
		u32::try_from(doc.aval_tag.len()).expect("fixture attribute count fits in u32");
	doc.aval_tag.push(slir::T_NUM);
	doc.aval_lo.push(0);
	doc.aval_hi.push(0);
	doc.aval_num.push(1.0);
	let number_default =
		u32::try_from(doc.aval_tag.len()).expect("fixture attribute count fits in u32");
	doc.aval_tag.push(slir::T_NUM);
	doc.aval_lo.push(0);
	doc.aval_hi.push(0);
	doc.aval_num.push(12.5);

	const NUMBER_PARAM: usize = 17;
	for param in 0..66 {
		let name = u32::try_from(doc.strs.len()).expect("fixture string count fits in u32");
		doc.strs.push(if param == NUMBER_PARAM {
			"scale".into()
		} else {
			format!("flag-{param}")
		});
		doc.parm_name.push(name);
		doc.parm_type.push(if param == NUMBER_PARAM {
			slir::PARAM_NUM
		} else {
			slir::PARAM_BOOL
		});
		doc.parm_default.push(if param == NUMBER_PARAM {
			number_default
		} else if param % 3 == 0 {
			true_default
		} else {
			false_default
		});
		doc.parm_enum_off.push(0);
		doc.parm_enum_len.push(0);
		doc.parm_site_off.push(0);
		doc.parm_site_len.push(0);
	}

	let mut params = ParamStore::default();
	params.init(&doc);
	for param in [0, 62, 63, 64, 65] {
		assert_eq!(
			frame::ParamValue::Bool(params.boolean(param)),
			frame::ParamValue::Bool(param % 3 == 0),
			"default for boundary-adjacent parameter {param}"
		);
	}
	assert_eq!(frame::ParamValue::Num(params.number(NUMBER_PARAM)), frame::ParamValue::Num(12.5));

	assert!(params.set_boolean(63, false), "bit 63 changes");
	assert!(!params.set_boolean(63, false), "equal bit-63 write is a no-op");
	assert!(params.set_boolean(64, true), "bit 64 changes");
	assert!(!params.set_boolean(64, true), "equal bit-64 write is a no-op");
	assert_eq!(frame::ParamValue::Bool(params.boolean(62)), frame::ParamValue::Bool(false));
	assert_eq!(frame::ParamValue::Bool(params.boolean(63)), frame::ParamValue::Bool(false));
	assert_eq!(frame::ParamValue::Bool(params.boolean(64)), frame::ParamValue::Bool(true));
	assert_eq!(frame::ParamValue::Bool(params.boolean(65)), frame::ParamValue::Bool(false));
	assert_eq!(
		frame::ParamValue::Num(params.number(NUMBER_PARAM)),
		frame::ParamValue::Num(12.5),
		"Boolean writes do not disturb the numeric lane"
	);
	assert!(!params.set_number(NUMBER_PARAM, 12.5), "equal numeric write is a no-op");
}
