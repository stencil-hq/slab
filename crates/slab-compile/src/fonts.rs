//! Complete FONT metric tables from shared runtime-font metrics.

use slab_fonts::parse_metrics;
use slab_slir::FontE;

/// Build the authoritative FONT metric and glyph-coverage table for one face.
///
/// `class` supplies fallback metrics classification and `weight` is the
/// weight this table represents in the document, independent of the weight
/// declared inside `bytes`. The complete cmap is required because host strings
/// can introduce any face-supported codepoint after compilation.
///
/// The table carries no sfnt payload: vendored faces resolve at run time
/// through [`slab_kernel::slir::face_data`], and callers embed bytes only for
/// host-supplied compile-time faces the kernels cannot resolve themselves.
pub fn build_table(class: u8, weight: u16, bytes: &[u8]) -> FontE {
	let metrics = parse_metrics(bytes).expect("registered font parses");
	let mut cmap = Vec::new();
	let mut advances = Vec::new();
	for (&cp, (&gid, &advance)) in metrics
		.cps
		.iter()
		.zip(metrics.gids.iter().zip(metrics.advances.iter()))
	{
		cmap.push((cp, u16::try_from(gid).expect("vendored glyph ID fits u16")));
		advances.push(u16::try_from(advance).expect("vendored advance fits u16"));
	}
	FontE {
		family: 0,
		class,
		weight,
		upem: metrics.upem,
		ascent: metrics.ascent,
		descent: metrics.descent,
		line_gap: metrics.line_gap,
		default_advance: metrics.default_advance,
		underline_position: metrics.underline_position,
		underline_thickness: metrics.underline_thickness,
		data: Vec::new(),
		cmap,
		advances,
	}
}
