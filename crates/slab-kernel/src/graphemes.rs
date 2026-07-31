//! Extended grapheme-cluster segmentation for caret, selection, and text
//! geometry.
//!
//! [`unicode_segmentation`] is the single authority for Unicode Standard Annex
//! #29 boundaries. The mark and terminal-width range tables below are separate
//! painting concerns derived from Unicode 16.0.0.

use unicode_segmentation::UnicodeSegmentation;

/// Zero-width joiner.
pub const ZWJ: u32 = 0x200du32;

/// Variation selector 15, requesting text presentation.
pub const VS15: u32 = 0xfe0eu32;

/// Variation selector 16, requesting emoji presentation.
pub const VS16: u32 = 0xfe0fu32;
const SUPPLEMENTARY_VS_LO: u32 = 0xe0100u32;
const SUPPLEMENTARY_VS_HI: u32 = 0xe01efu32;

/// Reports whether `cp` selects a standardized or ideographic glyph variant.
pub fn is_variation_selector(cp: u32) -> bool {
	(0xfe00u32..=0xfe0fu32).contains(&cp)
		|| (SUPPLEMENTARY_VS_LO..=SUPPLEMENTARY_VS_HI).contains(&cp)
}

/// Reports whether `cp` modifies neighboring glyphs without painting its own.
pub fn is_glyph_modifier(cp: u32) -> bool {
	cp == ZWJ || is_variation_selector(cp)
}

/// Reports whether `cp` requires an independently covered font glyph.
///
/// Diagnostics ignore controls and non-painting glyph modifiers. Painters use
/// [`is_glyph_modifier`] separately because ordinary controls retain the
/// document's fallback-advance policy even though they do not emit warnings.
pub fn requires_glyph(cp: u32) -> bool {
	!is_glyph_modifier(cp) && char::from_u32(cp).is_some_and(|character| !character.is_control())
}

/// First regional-indicator codepoint.
pub const RI_LO: u32 = 0x1f1e6u32;

/// Last regional-indicator codepoint.
pub const RI_HI: u32 = 0x1f1ffu32;

/// Number of inclusive ranges in [`MARK_LO`] and [`MARK_HI`].
pub const MARK_N: i32 = 321i32;

/// Lower bounds of the Unicode 16.0.0 combining-mark ranges.
pub const MARK_LO: [u32; 321] = [
	0x300u32, 0x483u32, 0x591u32, 0x5bfu32, 0x5c1u32, 0x5c4u32, 0x5c7u32, 0x610u32, 0x64bu32,
	0x670u32, 0x6d6u32, 0x6dfu32, 0x6e7u32, 0x6eau32, 0x711u32, 0x730u32, 0x7a6u32, 0x7ebu32,
	0x7fdu32, 0x816u32, 0x81bu32, 0x825u32, 0x829u32, 0x859u32, 0x897u32, 0x8cau32, 0x8e3u32,
	0x93au32, 0x93eu32, 0x951u32, 0x962u32, 0x981u32, 0x9bcu32, 0x9beu32, 0x9c7u32, 0x9cbu32,
	0x9d7u32, 0x9e2u32, 0x9feu32, 0xa01u32, 0xa3cu32, 0xa3eu32, 0xa47u32, 0xa4bu32, 0xa51u32,
	0xa70u32, 0xa75u32, 0xa81u32, 0xabcu32, 0xabeu32, 0xac7u32, 0xacbu32, 0xae2u32, 0xafau32,
	0xb01u32, 0xb3cu32, 0xb3eu32, 0xb47u32, 0xb4bu32, 0xb55u32, 0xb62u32, 0xb82u32, 0xbbeu32,
	0xbc6u32, 0xbcau32, 0xbd7u32, 0xc00u32, 0xc3cu32, 0xc3eu32, 0xc46u32, 0xc4au32, 0xc55u32,
	0xc62u32, 0xc81u32, 0xcbcu32, 0xcbeu32, 0xcc6u32, 0xccau32, 0xcd5u32, 0xce2u32, 0xcf3u32,
	0xd00u32, 0xd3bu32, 0xd3eu32, 0xd46u32, 0xd4au32, 0xd57u32, 0xd62u32, 0xd81u32, 0xdcau32,
	0xdcfu32, 0xdd6u32, 0xdd8u32, 0xdf2u32, 0xe31u32, 0xe34u32, 0xe47u32, 0xeb1u32, 0xeb4u32,
	0xec8u32, 0xf18u32, 0xf35u32, 0xf37u32, 0xf39u32, 0xf3eu32, 0xf71u32, 0xf86u32, 0xf8du32,
	0xf99u32, 0xfc6u32, 0x102bu32, 0x1056u32, 0x105eu32, 0x1062u32, 0x1067u32, 0x1071u32, 0x1082u32,
	0x108fu32, 0x109au32, 0x135du32, 0x1712u32, 0x1732u32, 0x1752u32, 0x1772u32, 0x17b4u32,
	0x17ddu32, 0x180bu32, 0x180fu32, 0x1885u32, 0x18a9u32, 0x1920u32, 0x1930u32, 0x1a17u32,
	0x1a55u32, 0x1a60u32, 0x1a7fu32, 0x1ab0u32, 0x1b00u32, 0x1b34u32, 0x1b6bu32, 0x1b80u32,
	0x1ba1u32, 0x1be6u32, 0x1c24u32, 0x1cd0u32, 0x1cd4u32, 0x1cedu32, 0x1cf4u32, 0x1cf7u32,
	0x1dc0u32, 0x20d0u32, 0x2cefu32, 0x2d7fu32, 0x2de0u32, 0x302au32, 0x3099u32, 0xa66fu32,
	0xa674u32, 0xa69eu32, 0xa6f0u32, 0xa802u32, 0xa806u32, 0xa80bu32, 0xa823u32, 0xa82cu32,
	0xa880u32, 0xa8b4u32, 0xa8e0u32, 0xa8ffu32, 0xa926u32, 0xa947u32, 0xa980u32, 0xa9b3u32,
	0xa9e5u32, 0xaa29u32, 0xaa43u32, 0xaa4cu32, 0xaa7bu32, 0xaab0u32, 0xaab2u32, 0xaab7u32,
	0xaabeu32, 0xaac1u32, 0xaaebu32, 0xaaf5u32, 0xabe3u32, 0xabecu32, 0xfb1eu32, 0xfe00u32,
	0xfe20u32, 0x101fdu32, 0x102e0u32, 0x10376u32, 0x10a01u32, 0x10a05u32, 0x10a0cu32, 0x10a38u32,
	0x10a3fu32, 0x10ae5u32, 0x10d24u32, 0x10d69u32, 0x10eabu32, 0x10efcu32, 0x10f46u32, 0x10f82u32,
	0x11000u32, 0x11038u32, 0x11070u32, 0x11073u32, 0x1107fu32, 0x110b0u32, 0x110c2u32, 0x11100u32,
	0x11127u32, 0x11145u32, 0x11173u32, 0x11180u32, 0x111b3u32, 0x111c9u32, 0x111ceu32, 0x1122cu32,
	0x1123eu32, 0x11241u32, 0x112dfu32, 0x11300u32, 0x1133bu32, 0x1133eu32, 0x11347u32, 0x1134bu32,
	0x11357u32, 0x11362u32, 0x11366u32, 0x11370u32, 0x113b8u32, 0x113c2u32, 0x113c5u32, 0x113c7u32,
	0x113ccu32, 0x113d2u32, 0x113e1u32, 0x11435u32, 0x1145eu32, 0x114b0u32, 0x115afu32, 0x115b8u32,
	0x115dcu32, 0x11630u32, 0x116abu32, 0x1171du32, 0x1182cu32, 0x11930u32, 0x11937u32, 0x1193bu32,
	0x11940u32, 0x11942u32, 0x119d1u32, 0x119dau32, 0x119e4u32, 0x11a01u32, 0x11a33u32, 0x11a3bu32,
	0x11a47u32, 0x11a51u32, 0x11a8au32, 0x11c2fu32, 0x11c38u32, 0x11c92u32, 0x11ca9u32, 0x11d31u32,
	0x11d3au32, 0x11d3cu32, 0x11d3fu32, 0x11d47u32, 0x11d8au32, 0x11d90u32, 0x11d93u32, 0x11ef3u32,
	0x11f00u32, 0x11f03u32, 0x11f34u32, 0x11f3eu32, 0x11f5au32, 0x13440u32, 0x13447u32, 0x1611eu32,
	0x16af0u32, 0x16b30u32, 0x16f4fu32, 0x16f51u32, 0x16f8fu32, 0x16fe4u32, 0x16ff0u32, 0x1bc9du32,
	0x1cf00u32, 0x1cf30u32, 0x1d165u32, 0x1d16du32, 0x1d17bu32, 0x1d185u32, 0x1d1aau32, 0x1d242u32,
	0x1da00u32, 0x1da3bu32, 0x1da75u32, 0x1da84u32, 0x1da9bu32, 0x1daa1u32, 0x1e000u32, 0x1e008u32,
	0x1e01bu32, 0x1e023u32, 0x1e026u32, 0x1e08fu32, 0x1e130u32, 0x1e2aeu32, 0x1e2ecu32, 0x1e4ecu32,
	0x1e5eeu32, 0x1e8d0u32, 0x1e944u32, 0xe0100u32,
];

/// Inclusive upper bounds corresponding to [`MARK_LO`].
pub const MARK_HI: [u32; 321] = [
	0x36fu32, 0x489u32, 0x5bdu32, 0x5bfu32, 0x5c2u32, 0x5c5u32, 0x5c7u32, 0x61au32, 0x65fu32,
	0x670u32, 0x6dcu32, 0x6e4u32, 0x6e8u32, 0x6edu32, 0x711u32, 0x74au32, 0x7b0u32, 0x7f3u32,
	0x7fdu32, 0x819u32, 0x823u32, 0x827u32, 0x82du32, 0x85bu32, 0x89fu32, 0x8e1u32, 0x903u32,
	0x93cu32, 0x94fu32, 0x957u32, 0x963u32, 0x983u32, 0x9bcu32, 0x9c4u32, 0x9c8u32, 0x9cdu32,
	0x9d7u32, 0x9e3u32, 0x9feu32, 0xa03u32, 0xa3cu32, 0xa42u32, 0xa48u32, 0xa4du32, 0xa51u32,
	0xa71u32, 0xa75u32, 0xa83u32, 0xabcu32, 0xac5u32, 0xac9u32, 0xacdu32, 0xae3u32, 0xaffu32,
	0xb03u32, 0xb3cu32, 0xb44u32, 0xb48u32, 0xb4du32, 0xb57u32, 0xb63u32, 0xb82u32, 0xbc2u32,
	0xbc8u32, 0xbcdu32, 0xbd7u32, 0xc04u32, 0xc3cu32, 0xc44u32, 0xc48u32, 0xc4du32, 0xc56u32,
	0xc63u32, 0xc83u32, 0xcbcu32, 0xcc4u32, 0xcc8u32, 0xccdu32, 0xcd6u32, 0xce3u32, 0xcf3u32,
	0xd03u32, 0xd3cu32, 0xd44u32, 0xd48u32, 0xd4du32, 0xd57u32, 0xd63u32, 0xd83u32, 0xdcau32,
	0xdd4u32, 0xdd6u32, 0xddfu32, 0xdf3u32, 0xe31u32, 0xe3au32, 0xe4eu32, 0xeb1u32, 0xebcu32,
	0xeceu32, 0xf19u32, 0xf35u32, 0xf37u32, 0xf39u32, 0xf3fu32, 0xf84u32, 0xf87u32, 0xf97u32,
	0xfbcu32, 0xfc6u32, 0x103eu32, 0x1059u32, 0x1060u32, 0x1064u32, 0x106du32, 0x1074u32, 0x108du32,
	0x108fu32, 0x109du32, 0x135fu32, 0x1715u32, 0x1734u32, 0x1753u32, 0x1773u32, 0x17d3u32,
	0x17ddu32, 0x180du32, 0x180fu32, 0x1886u32, 0x18a9u32, 0x192bu32, 0x193bu32, 0x1a1bu32,
	0x1a5eu32, 0x1a7cu32, 0x1a7fu32, 0x1aceu32, 0x1b04u32, 0x1b44u32, 0x1b73u32, 0x1b82u32,
	0x1badu32, 0x1bf3u32, 0x1c37u32, 0x1cd2u32, 0x1ce8u32, 0x1cedu32, 0x1cf4u32, 0x1cf9u32,
	0x1dffu32, 0x20f0u32, 0x2cf1u32, 0x2d7fu32, 0x2dffu32, 0x302fu32, 0x309au32, 0xa672u32,
	0xa67du32, 0xa69fu32, 0xa6f1u32, 0xa802u32, 0xa806u32, 0xa80bu32, 0xa827u32, 0xa82cu32,
	0xa881u32, 0xa8c5u32, 0xa8f1u32, 0xa8ffu32, 0xa92du32, 0xa953u32, 0xa983u32, 0xa9c0u32,
	0xa9e5u32, 0xaa36u32, 0xaa43u32, 0xaa4du32, 0xaa7du32, 0xaab0u32, 0xaab4u32, 0xaab8u32,
	0xaabfu32, 0xaac1u32, 0xaaefu32, 0xaaf6u32, 0xabeau32, 0xabedu32, 0xfb1eu32, 0xfe0fu32,
	0xfe2fu32, 0x101fdu32, 0x102e0u32, 0x1037au32, 0x10a03u32, 0x10a06u32, 0x10a0fu32, 0x10a3au32,
	0x10a3fu32, 0x10ae6u32, 0x10d27u32, 0x10d6du32, 0x10eacu32, 0x10effu32, 0x10f50u32, 0x10f85u32,
	0x11002u32, 0x11046u32, 0x11070u32, 0x11074u32, 0x11082u32, 0x110bau32, 0x110c2u32, 0x11102u32,
	0x11134u32, 0x11146u32, 0x11173u32, 0x11182u32, 0x111c0u32, 0x111ccu32, 0x111cfu32, 0x11237u32,
	0x1123eu32, 0x11241u32, 0x112eau32, 0x11303u32, 0x1133cu32, 0x11344u32, 0x11348u32, 0x1134du32,
	0x11357u32, 0x11363u32, 0x1136cu32, 0x11374u32, 0x113c0u32, 0x113c2u32, 0x113c5u32, 0x113cau32,
	0x113d0u32, 0x113d2u32, 0x113e2u32, 0x11446u32, 0x1145eu32, 0x114c3u32, 0x115b5u32, 0x115c0u32,
	0x115ddu32, 0x11640u32, 0x116b7u32, 0x1172bu32, 0x1183au32, 0x11935u32, 0x11938u32, 0x1193eu32,
	0x11940u32, 0x11943u32, 0x119d7u32, 0x119e0u32, 0x119e4u32, 0x11a0au32, 0x11a39u32, 0x11a3eu32,
	0x11a47u32, 0x11a5bu32, 0x11a99u32, 0x11c36u32, 0x11c3fu32, 0x11ca7u32, 0x11cb6u32, 0x11d36u32,
	0x11d3au32, 0x11d3du32, 0x11d45u32, 0x11d47u32, 0x11d8eu32, 0x11d91u32, 0x11d97u32, 0x11ef6u32,
	0x11f01u32, 0x11f03u32, 0x11f3au32, 0x11f42u32, 0x11f5au32, 0x13440u32, 0x13455u32, 0x1612fu32,
	0x16af4u32, 0x16b36u32, 0x16f4fu32, 0x16f87u32, 0x16f92u32, 0x16fe4u32, 0x16ff1u32, 0x1bc9eu32,
	0x1cf2du32, 0x1cf46u32, 0x1d169u32, 0x1d172u32, 0x1d182u32, 0x1d18bu32, 0x1d1adu32, 0x1d244u32,
	0x1da36u32, 0x1da6cu32, 0x1da75u32, 0x1da84u32, 0x1da9fu32, 0x1daafu32, 0x1e006u32, 0x1e018u32,
	0x1e021u32, 0x1e024u32, 0x1e02au32, 0x1e08fu32, 0x1e136u32, 0x1e2aeu32, 0x1e2efu32, 0x1e4efu32,
	0x1e5efu32, 0x1e8d6u32, 0x1e94au32, 0xe01efu32,
];

/// Number of inclusive ranges in [`WIDE_LO`] and [`WIDE_HI`].
pub const WIDE_N: i32 = 122i32;

/// Lower bounds of the Unicode 16.0.0 terminal-width-two ranges.
pub const WIDE_LO: [u32; 122] = [
	0x1100u32, 0x231au32, 0x2329u32, 0x23e9u32, 0x23f0u32, 0x23f3u32, 0x25fdu32, 0x2614u32,
	0x2630u32, 0x2648u32, 0x267fu32, 0x268au32, 0x2693u32, 0x26a1u32, 0x26aau32, 0x26bdu32,
	0x26c4u32, 0x26ceu32, 0x26d4u32, 0x26eau32, 0x26f2u32, 0x26f5u32, 0x26fau32, 0x26fdu32,
	0x2705u32, 0x270au32, 0x2728u32, 0x274cu32, 0x274eu32, 0x2753u32, 0x2757u32, 0x2795u32,
	0x27b0u32, 0x27bfu32, 0x2b1bu32, 0x2b50u32, 0x2b55u32, 0x2e80u32, 0x2e9bu32, 0x2f00u32,
	0x2ff0u32, 0x3041u32, 0x3099u32, 0x3105u32, 0x3131u32, 0x3190u32, 0x31efu32, 0x3220u32,
	0x3250u32, 0xa490u32, 0xa960u32, 0xac00u32, 0xf900u32, 0xfe10u32, 0xfe30u32, 0xfe54u32,
	0xfe68u32, 0xff01u32, 0xffe0u32, 0x16fe0u32, 0x16ff0u32, 0x17000u32, 0x18800u32, 0x18cffu32,
	0x1aff0u32, 0x1aff5u32, 0x1affdu32, 0x1b000u32, 0x1b132u32, 0x1b150u32, 0x1b155u32, 0x1b164u32,
	0x1b170u32, 0x1d300u32, 0x1d360u32, 0x1f004u32, 0x1f0cfu32, 0x1f18eu32, 0x1f191u32, 0x1f200u32,
	0x1f210u32, 0x1f240u32, 0x1f250u32, 0x1f260u32, 0x1f300u32, 0x1f32du32, 0x1f337u32, 0x1f37eu32,
	0x1f3a0u32, 0x1f3cfu32, 0x1f3e0u32, 0x1f3f4u32, 0x1f3f8u32, 0x1f440u32, 0x1f442u32, 0x1f4ffu32,
	0x1f54bu32, 0x1f550u32, 0x1f57au32, 0x1f595u32, 0x1f5a4u32, 0x1f5fbu32, 0x1f680u32, 0x1f6ccu32,
	0x1f6d0u32, 0x1f6d5u32, 0x1f6dcu32, 0x1f6ebu32, 0x1f6f4u32, 0x1f7e0u32, 0x1f7f0u32, 0x1f90cu32,
	0x1f93cu32, 0x1f947u32, 0x1fa70u32, 0x1fa80u32, 0x1fa8fu32, 0x1faceu32, 0x1fadfu32, 0x1faf0u32,
	0x20000u32, 0x30000u32,
];

/// Inclusive upper bounds corresponding to [`WIDE_LO`].
pub const WIDE_HI: [u32; 122] = [
	0x115fu32, 0x231bu32, 0x232au32, 0x23ecu32, 0x23f0u32, 0x23f3u32, 0x25feu32, 0x2615u32,
	0x2637u32, 0x2653u32, 0x267fu32, 0x268fu32, 0x2693u32, 0x26a1u32, 0x26abu32, 0x26beu32,
	0x26c5u32, 0x26ceu32, 0x26d4u32, 0x26eau32, 0x26f3u32, 0x26f5u32, 0x26fau32, 0x26fdu32,
	0x2705u32, 0x270bu32, 0x2728u32, 0x274cu32, 0x274eu32, 0x2755u32, 0x2757u32, 0x2797u32,
	0x27b0u32, 0x27bfu32, 0x2b1cu32, 0x2b50u32, 0x2b55u32, 0x2e99u32, 0x2ef3u32, 0x2fd5u32,
	0x303eu32, 0x3096u32, 0x30ffu32, 0x312fu32, 0x318eu32, 0x31e5u32, 0x321eu32, 0x3247u32,
	0xa48cu32, 0xa4c6u32, 0xa97cu32, 0xd7a3u32, 0xfaffu32, 0xfe19u32, 0xfe52u32, 0xfe66u32,
	0xfe6bu32, 0xff60u32, 0xffe6u32, 0x16fe4u32, 0x16ff1u32, 0x187f7u32, 0x18cd5u32, 0x18d08u32,
	0x1aff3u32, 0x1affbu32, 0x1affeu32, 0x1b122u32, 0x1b132u32, 0x1b152u32, 0x1b155u32, 0x1b167u32,
	0x1b2fbu32, 0x1d356u32, 0x1d376u32, 0x1f004u32, 0x1f0cfu32, 0x1f18eu32, 0x1f19au32, 0x1f202u32,
	0x1f23bu32, 0x1f248u32, 0x1f251u32, 0x1f265u32, 0x1f320u32, 0x1f335u32, 0x1f37cu32, 0x1f393u32,
	0x1f3cau32, 0x1f3d3u32, 0x1f3f0u32, 0x1f3f4u32, 0x1f43eu32, 0x1f440u32, 0x1f4fcu32, 0x1f53du32,
	0x1f54eu32, 0x1f567u32, 0x1f57au32, 0x1f596u32, 0x1f5a4u32, 0x1f64fu32, 0x1f6c5u32, 0x1f6ccu32,
	0x1f6d2u32, 0x1f6d7u32, 0x1f6dfu32, 0x1f6ecu32, 0x1f6fcu32, 0x1f7ebu32, 0x1f7f0u32, 0x1f93au32,
	0x1f945u32, 0x1f9ffu32, 0x1fa7cu32, 0x1fa89u32, 0x1fac6u32, 0x1fadcu32, 0x1fae9u32, 0x1faf8u32,
	0x2fffdu32, 0x3fffdu32,
];

fn is_in_ranges(cp: u32, lower: &[u32], upper: &[u32]) -> bool {
	let candidate_count = lower.partition_point(|&range_start| range_start <= cp);
	candidate_count != 0 && cp <= upper[candidate_count - 1]
}

/// Returns whether `cp` is a combining mark (general category `Mn`, `Mc`, or
/// `Me`).
pub fn is_mark(cp: u32) -> bool {
	if cp < MARK_LO[0] {
		return false;
	}
	is_in_ranges(cp, &MARK_LO, &MARK_HI)
}

/// Returns whether `cp` occupies two terminal cells according to the Unicode
/// 16.0.0 width table.
pub fn cp_wide(cp: u32) -> bool {
	if cp < WIDE_LO[0] {
		return false;
	}
	is_in_ranges(cp, &WIDE_LO, &WIDE_HI)
}

/// Returns whether one grapheme cluster occupies two terminal cells.
pub fn cluster_wide(s: &str, start: i32, end: i32) -> bool {
	let start = usize::try_from(start).expect("negative string slice start");
	let end = usize::try_from(end).expect("negative string slice end");
	assert!(start <= end, "string slice start exceeds end");

	let mut codepoints = s.chars();
	for _ in 0..start {
		codepoints.next().expect("string slice out of bounds");
	}

	let mut remaining = end - start;
	if remaining == 0 {
		return false;
	}

	let base = u32::from(codepoints.next().expect("string slice out of bounds"));
	remaining -= 1;
	let mut regional_count = i32::from(is_ri(base));
	let mut text_presentation = base == VS15;
	let mut emoji_presentation = base == VS16;

	for _ in 0..remaining {
		let cp = u32::from(codepoints.next().expect("string slice out of bounds"));
		if is_ri(cp) {
			regional_count = regional_count.wrapping_add(1);
		}
		if cp == VS15 {
			text_presentation = true;
		} else if cp == VS16 {
			emoji_presentation = true;
		}
	}

	!text_presentation && (cp_wide(base) || emoji_presentation || regional_count == 2)
}

/// Returns whether `cp` is a regional-indicator symbol
/// (`U+1F1E6..=U+1F1FF`).
pub fn is_ri(cp: u32) -> bool {
	(RI_LO..=RI_HI).contains(&cp)
}

/// Writes the cluster boundaries of `text` as ascending codepoint offsets.
///
/// The result always starts with zero and, for non-empty text, ends with its
/// codepoint length. Carets may sit exactly at these offsets.
pub fn boundaries(text: &str, out: &mut Vec<i32>) {
	out.clear();
	out.push(0);

	let mut offset = 0i32;
	for cluster in text.graphemes(true) {
		for _ in cluster.chars() {
			offset = offset.wrapping_add(1);
		}
		out.push(offset);
	}
}

/// Returns the largest boundary strictly below `at`, or zero when none exists.
///
/// This is the backspace and left-arrow target.
pub fn prev_boundary(bounds: &[i32], at: i32) -> i32 {
	bounds
		.iter()
		.copied()
		.filter(|&boundary| boundary < at && boundary > 0)
		.max()
		.unwrap_or(0)
}

/// Returns the smallest boundary strictly above `at`, or `n` when none exists.
///
/// This is the delete and right-arrow target.
pub fn next_boundary(bounds: &[i32], at: i32, n: i32) -> i32 {
	bounds
		.iter()
		.copied()
		.filter(|&boundary| boundary > at && boundary < n)
		.min()
		.unwrap_or(n)
}

const LF: u32 = 10;
const CR: u32 = 13;

/// Encodes `cps[start..end]` for windowed segmentation.
fn window_str(cps: &[u32], start: i32, end: i32) -> String {
	let start = usize::try_from(start).expect("negative window start");
	let end = usize::try_from(end).expect("negative window end");
	cps[start..end]
		.iter()
		.map(|&cp| char::from_u32(cp).expect("invalid codepoint"))
		.collect()
}

/// Returns the codepoint index just after the last LF strictly before `at`.
///
/// UAX #29 breaks after every control (GB4), so this is always a true
/// grapheme boundary: segmentation restarted here matches the full text.
fn line_start(cps: &[u32], at: i32) -> i32 {
	let mut index = at;
	while index > 0 && cps[usize::try_from(index - 1).expect("negative index")] != LF {
		index -= 1;
	}
	index
}

/// Returns the largest grapheme boundary strictly below `at`, or zero.
///
/// Equivalent to [`prev_boundary`] over [`boundaries`] of the full text,
/// but segments only the caret's hard line: clusters never cross a LF, so
/// the window between the previous newline and `at` reproduces the full
/// boundary table locally. This is the backspace and left-arrow target.
pub fn prev_boundary_in(cps: &[u32], at: i32) -> i32 {
	let len = i32::try_from(cps.len()).expect("text has too many codepoints");
	let at = at.clamp(0, len);
	if at <= 0 {
		return 0;
	}
	let before = cps[usize::try_from(at - 1).expect("negative index")];
	if before == LF {
		// The cluster ending at `at` is the newline itself: "\r\n" is one
		// cluster (GB3); any other LF stands alone (GB4/GB5).
		if at >= 2 && cps[usize::try_from(at - 2).expect("negative index")] == CR {
			return at - 2;
		}
		return at - 1;
	}
	let start = line_start(cps, at);
	let window = window_str(cps, start, at);
	let mut bounds = Vec::new();
	boundaries(&window, &mut bounds);
	start.wrapping_add(
		bounds
			.iter()
			.copied()
			.filter(|&boundary| boundary < at.wrapping_sub(start))
			.max()
			.unwrap_or(0),
	)
}

/// Returns the smallest grapheme boundary strictly above `at`, or the text
/// length.
///
/// Windowed like [`prev_boundary_in`]; this is the delete and right-arrow
/// target.
pub fn next_boundary_in(cps: &[u32], at: i32) -> i32 {
	let len = i32::try_from(cps.len()).expect("text has too many codepoints");
	let at = at.clamp(0, len);
	if at >= len {
		return len;
	}
	let start = line_start(cps, at);
	// End the window just past the next LF so a trailing "\r\n" pair stays
	// whole; the position after a LF is always a true boundary (GB4).
	let mut end = at;
	while end < len && cps[usize::try_from(end).expect("negative index")] != LF {
		end += 1;
	}
	if end < len {
		end += 1;
	}
	let window = window_str(cps, start, end);
	let mut bounds = Vec::new();
	boundaries(&window, &mut bounds);
	let fallback = end.wrapping_sub(start);
	start.wrapping_add(
		bounds
			.iter()
			.copied()
			.filter(|&boundary| boundary > at.wrapping_sub(start))
			.min()
			.unwrap_or(fallback),
	)
}

/// Writes the cluster boundaries of a codepoint slice, like [`boundaries`].
pub fn boundaries_cps(cps: &[u32], out: &mut Vec<i32>) {
	let text = window_str(cps, 0, i32::try_from(cps.len()).expect("text has too many codepoints"));
	boundaries(&text, out);
}
