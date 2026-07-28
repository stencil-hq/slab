//! Static exporters resolve faces from the live document, never from the
//! compiled SLIR alone: the kernel shapes glyph ids against each FONT table's
//! embedded sfnt data, and host registration appends tables the SLIR never
//! carried.

use std::collections::HashMap;

use slab_compile::{Options, compile, raster::Raster, render::RegisteredFont};
use slab_kernel::{
	flatten::{Frame, FrameOp},
	frame,
	slir::Doc,
};
use slab_slir::Slir;

const SOURCE: &str = r#"col w=140 h=48 {
  text "AB" family="Test Face" size=24
}"#;

fn mono_face() -> &'static [u8] {
	slab_fonts::asset(slab_fonts::CLASS_MONO, 400).bytes
}

fn sans_face() -> &'static [u8] {
	slab_fonts::asset(slab_fonts::CLASS_SANS, 400).bytes
}

/// Compiles [`SOURCE`], optionally backing `Test Face` with real bytes, and
/// returns the SLIR beside a decoded instance sized for the PNG client.
fn fixture(fonts: HashMap<String, Vec<u8>>) -> (Slir, frame::Instance) {
	let (slir, diagnostics) =
		compile(SOURCE, &Options { embed_assets: false, fonts, ..Options::default() });
	assert!(!diagnostics.has_errors(), "{:#?}", diagnostics.0);
	let slir = slir.expect("valid source");
	let bytes = slab_slir::write(&slir);
	let (mut instance, _) = slab_slir::instance(&bytes).expect("decode fixture");
	frame::inst_set_env(&mut instance, 140.0, 48.0, 4, false, false);
	(slir, instance)
}

fn registered(bytes: &[u8]) -> RegisteredFont {
	let metrics = slab_fonts::parse_metrics(bytes).expect("bundled face parses");
	RegisteredFont::new("Test Face".to_owned(), bytes.to_vec(), metrics)
}

/// Premultiplied RGBA8 pixels of one rasterized frame.
fn pixels(slir: &Slir, doc: &Doc, frame: &Frame, fonts: &[RegisteredFont]) -> Vec<u8> {
	Raster::new(slir, doc, &[], &[], fonts, 1.0)
		.render(frame)
		.expect("raster renders the fixture")
		.data()
		.to_vec()
}

fn ink(pixels: &[u8]) -> usize {
	pixels
		.as_chunks::<4>()
		.0
		.iter()
		.filter(|pixel| pixel[3] > 0)
		.count()
}

#[test]
fn embedded_face_outranks_a_conflicting_registration() {
	let (slir, mut instance) =
		fixture(HashMap::from([("Test Face".to_owned(), mono_face().to_vec())]));
	let rendered = frame::inst_frame(&mut instance, 0.0);
	let embedded = pixels(&slir, instance.doc(), &rendered, &[]);
	let shadowed = pixels(&slir, instance.doc(), &rendered, &[registered(sans_face())]);
	assert!(ink(&embedded) > 0, "the fixture paints glyph ink");
	let (bundled_slir, mut bundled_instance) = fixture(HashMap::new());
	let bundled_frame = frame::inst_frame(&mut bundled_instance, 0.0);
	assert_ne!(
		embedded,
		pixels(&bundled_slir, bundled_instance.doc(), &bundled_frame, &[]),
		"the embedded mono face and the bundled sans fallback draw distinguishable ink"
	);
	assert_eq!(
		embedded, shadowed,
		"glyph ids were shaped against the embedded face, so a same-family registration must not \
		 change the outlines"
	);
}

#[test]
fn runtime_registered_table_paints() {
	let (slir, mut instance) = fixture(HashMap::new());
	let metrics = slab_fonts::parse_metrics(mono_face()).expect("bundled mono parses");
	frame::inst_font_register(
		&mut instance,
		"Test Face",
		u32::from(metrics.weight),
		u32::from(metrics.upem),
		i32::from(metrics.ascent),
		i32::from(metrics.descent),
		i32::from(metrics.line_gap),
		i32::from(metrics.underline_position),
		i32::from(metrics.underline_thickness),
		u32::from(metrics.default_advance),
		mono_face(),
		&metrics.cps,
		&metrics.gids,
		&metrics.advances,
	);
	let rendered = frame::inst_frame(&mut instance, 0.0);
	let font = rendered
		.ops
		.iter()
		.find_map(|op| match op {
			FrameOp::Text(text) => Some(text.font),
			_ => None,
		})
		.expect("the fixture paints one text op");
	assert!(
		usize::try_from(font).expect("resolved font index") >= slir.fonts.len(),
		"an equal runtime registration wins the weight tie, so the op must use an appended table"
	);
	assert!(
		ink(&pixels(&slir, instance.doc(), &rendered, &[])) > 0,
		"a FONT table that exists only in the live document must still paint"
	);
}
