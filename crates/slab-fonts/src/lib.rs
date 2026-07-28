//! Vendored fallback fonts and authoritative coverage/metrics extracted from
//! registered files.

use std::collections::BTreeMap;

/// Fallback class for proportional faces.
pub const CLASS_SANS: u8 = 0;

/// Fallback class for monospaced faces.
pub const CLASS_MONO: u8 = 1;

/// A bundled fallback face used when a runtime family is unavailable.
pub struct FontAsset {
	/// Fallback class used for layout metrics.
	pub class:  u8,
	/// Weight represented by this face.
	pub weight: u16,
	/// CSS family name exposed by this face.
	pub family: &'static str,
	/// Complete sfnt bytes used by native and export paint.
	pub bytes:  &'static [u8],
}

macro_rules! font {
	($class:expr, $weight:expr, $family:literal, $file:literal) => {
		FontAsset {
			class:  $class,
			weight: $weight,
			family: $family,
			bytes:  include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/", $file)),
		}
	};
}

/// Built-in fallback faces shipped by native and compiler exports.
pub static FONT_ASSETS: [FontAsset; 8] = [
	font!(CLASS_SANS, 400, "Inter", "Inter-Regular.ttf"),
	font!(CLASS_SANS, 500, "Inter", "Inter-Medium.ttf"),
	font!(CLASS_SANS, 600, "Inter", "Inter-SemiBold.ttf"),
	font!(CLASS_SANS, 700, "Inter", "Inter-Bold.ttf"),
	font!(CLASS_MONO, 400, "JetBrains Mono", "JetBrainsMono-Regular.ttf"),
	font!(CLASS_MONO, 500, "JetBrains Mono", "JetBrainsMono-Medium.ttf"),
	font!(CLASS_MONO, 600, "JetBrains Mono", "JetBrainsMono-SemiBold.ttf"),
	font!(CLASS_MONO, 700, "JetBrains Mono", "JetBrainsMono-Bold.ttf"),
];

/// `mono` (or any family whose name contains "mono") selects mono fallback
/// metrics; all other names use sans fallback metrics.
pub fn classify_family(family: &str) -> u8 {
	if family.to_ascii_lowercase().contains("mono") {
		CLASS_MONO
	} else {
		CLASS_SANS
	}
}

/// Snap an authored weight to the nearest vendored weight; ties round up.
pub fn snap_weight(w: f64) -> u16 {
	let mut best = 400u16;
	let mut best_d = f64::INFINITY;
	for &cand in &[400u16, 500, 600, 700] {
		let d = (w - f64::from(cand)).abs();
		if d < best_d || (d == best_d && cand > best) {
			best = cand;
			best_d = d;
		}
	}
	best
}

/// Finds a bundled fallback face by class and snapped weight.
pub fn asset(class: u8, weight: u16) -> &'static FontAsset {
	FONT_ASSETS
		.iter()
		.find(|a| a.class == class && a.weight == weight)
		.expect("weight already snapped to a vendored weight")
}

/// Metrics and complete Unicode cmap extracted from a font file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredMetrics {
	pub weight:          u16,
	pub upem:            u16,
	pub ascent:          i16,
	pub descent:         i16,
	pub line_gap:        i16,
	pub default_advance: u16,
	/// Sorted Unicode codepoints.
	pub cps:             Vec<u32>,
	/// Glyph IDs parallel to `cps`.
	pub gids:            Vec<u32>,
	/// Horizontal advances parallel to `cps`.
	pub advances:        Vec<u32>,
}

/// Parse the layout data needed to register a runtime font.
///
/// The fallback advance matches compiler metric tables: `.notdef` when it has
/// a width, otherwise space, otherwise half an em.
pub fn parse_metrics(bytes: &[u8]) -> Option<RegisteredMetrics> {
	let face = ttf_parser::Face::parse(bytes, 0).ok()?;
	let upem = face.units_per_em();
	let notdef = face.glyph_hor_advance(ttf_parser::GlyphId(0)).unwrap_or(0);
	let space = face
		.glyph_index(' ')
		.and_then(|gid| face.glyph_hor_advance(gid))
		.unwrap_or(upem / 2);
	let default_advance = if notdef != 0 { notdef } else { space };

	let mut cmap = BTreeMap::new();
	let tables = face.tables();
	let subtables = tables.cmap?.subtables;
	for subtable in subtables {
		if !subtable.is_unicode() {
			continue;
		}
		subtable.codepoints(|cp| {
			if let Some(ch) = char::from_u32(cp)
				&& let Some(gid) = face.glyph_index(ch)
				&& gid.0 != 0
			{
				cmap
					.entry(cp)
					.or_insert_with(|| (gid.0, face.glyph_hor_advance(gid).unwrap_or(default_advance)));
			}
		});
	}

	let mut cps = Vec::with_capacity(cmap.len());
	let mut gids = Vec::with_capacity(cmap.len());
	let mut advances = Vec::with_capacity(cmap.len());
	for (cp, (gid, advance)) in cmap {
		cps.push(cp);
		gids.push(u32::from(gid));
		advances.push(u32::from(advance));
	}

	Some(RegisteredMetrics {
		weight: face.weight().to_number(),
		upem,
		ascent: face.ascender(),
		descent: face.descender(),
		line_gap: face.line_gap(),
		default_advance,
		cps,
		gids,
		advances,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn registered_metrics_cover_the_vendored_cmap() {
		let metrics = parse_metrics(asset(CLASS_SANS, 400).bytes).expect("Inter parses");
		assert_eq!(metrics.weight, 400);
		assert_eq!(metrics.cps.len(), metrics.gids.len());
		assert_eq!(metrics.cps.len(), metrics.advances.len());
		assert!(metrics.cps.binary_search(&u32::from('A')).is_ok());
		assert!(metrics.default_advance > 0);
	}
}
