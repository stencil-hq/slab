//! SLIR write -> read -> write byte-identity, and slir-dump determinism,
//! over the full conformance corpus.

use std::path::PathBuf;

use slab_compile::{Options, compile};

fn cases_dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/cases")
}

fn compile_case(path: &PathBuf) -> slab_slir::Slir {
	let src = std::fs::read_to_string(path).unwrap();
	let opts = Options {
		embed_assets: true,
		base_dir: path.parent().unwrap().to_path_buf(),
		..Options::default()
	};
	let (slir, diags) = compile(&src, &opts);
	assert!(!diags.has_errors(), "{}: {:?}", path.display(), diags.0);
	slir.unwrap()
}

#[test]
fn roundtrip_byte_identity_over_corpus() {
	let mut cases: Vec<_> = std::fs::read_dir(cases_dir())
		.unwrap()
		.filter_map(|e| {
			let p = e.unwrap().path();
			(p.extension().is_some_and(|x| x == "slab")).then_some(p)
		})
		.collect();
	cases.sort();
	assert!(!cases.is_empty(), "no conformance cases found");
	for path in cases {
		let slir = compile_case(&path);
		let bytes = slab_slir::write(&slir);
		let back = slab_slir::read(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
		let bytes2 = slab_slir::write(&back);
		assert_eq!(bytes, bytes2, "{}: write->read->write is not byte-identical", path.display());
		assert_eq!(back, slir, "{}: decoded model differs", path.display());
	}
}

#[test]
fn dump_is_deterministic() {
	let path = cases_dir().join("l4-golden.slab");
	let a = slab_slir::dump(&compile_case(&path));
	let b = slab_slir::dump(&compile_case(&path));
	assert_eq!(a, b);
}

#[test]
fn reader_rejects_unsupported_envelope_versions() {
	let path = cases_dir().join("l1-box-basics.slab");
	let slir = compile_case(&path);
	let mut bytes = slab_slir::write(&slir);
	bytes[4..6].copy_from_slice(&(slab_slir::MAJOR + 1).to_le_bytes());
	assert!(slab_slir::read(&bytes).is_err());

	bytes[4..6].copy_from_slice(&slab_slir::MAJOR.to_le_bytes());
	bytes[6..8].copy_from_slice(&(slab_slir::MINOR + 1).to_le_bytes());
	assert!(slab_slir::read(&bytes).is_err());
}

#[test]
fn static_svg_freezes_text_keyframes_and_reports_degradation() {
	let path = cases_dir().join("14-spinner.slab");
	let slir = compile_case(&path);
	let opts = slab_compile::render::RenderOpts {
		kind:             slab_compile::render::RenderKind::Svg,
		client:           Some("svg".into()),
		theme:            None,
		width:            160.0,
		height:           32.0,
		scale:            1.0,
		t:                375.0,
		dur:              2.0,
		fps:              20.0,
		states:           vec![],
		env:              vec![],
		sets:             vec![],
		plain:            false,
		registered_fonts: vec![],
	};
	let out = slab_compile::render::render(&slir, &opts, path.parent().unwrap()).unwrap();
	let svg = String::from_utf8(out.bytes).unwrap();
	assert!(svg.contains(">idle</text>"));
	assert!(!svg.contains(">⠙</text>"));
	assert_eq!(out.notes, [
		"note cap-anim-content: 'text-keyframes' is degraded by the svg renderer"
	]);
}

#[test]
fn theme_api_rejects_unknown_names_and_restyles_frames() {
	let src = r"
tokens { color { bg #ffffff } }
theme dusk { color { bg #121826 } }
rect id=panel w=20 h=20 bg=color.bg
";
	let opts = Options { embed_assets: false, base_dir: PathBuf::new(), ..Options::default() };
	let (slir, diags) = compile(src, &opts);
	assert!(!diags.has_errors(), "{:?}", diags.0);
	let slir = slir.unwrap();
	assert_eq!(
		slir
			.themes
			.iter()
			.map(|&name| slir.str_at(name))
			.collect::<Vec<_>>(),
		["dusk"]
	);

	let bytes = slab_slir::write(&slir);
	let (mut inst, _) = slab_slir::instance(&bytes).unwrap();
	slab_kernel::frame::inst_set_env(&mut inst, 20.0, 20.0, 0, false, false);
	let base = slab_kernel::frame::inst_frame(&mut inst, 0.0);
	let base_bg = base.ops.iter().find_map(|op| match op {
		slab_kernel::flatten::FrameOp::Rect(rect) => Some(rect.bg),
		_ => None,
	});

	assert_eq!(slab_kernel::frame::inst_theme(&inst), "");
	assert!(!slab_kernel::frame::inst_set_theme(&mut inst, "unknown"));
	assert_eq!(slab_kernel::frame::inst_theme(&inst), "");
	assert!(slab_kernel::frame::inst_set_theme(&mut inst, "dusk"));
	assert_eq!(slab_kernel::frame::inst_theme(&inst), "dusk");
	let dusk = slab_kernel::frame::inst_frame(&mut inst, 0.0);
	let dusk_bg = dusk.ops.iter().find_map(|op| match op {
		slab_kernel::flatten::FrameOp::Rect(rect) => Some(rect.bg),
		_ => None,
	});
	assert_ne!(base_bg, dusk_bg);
	assert!(slab_kernel::frame::inst_set_theme(&mut inst, ""));
	assert_eq!(slab_kernel::frame::inst_theme(&inst), "");
}

#[test]
fn active_theme_tokens_survive_deferred_patches_and_component_substitution() {
	let src = r"
tokens {
  color { direct #110000; state #220000; default #330000; arg #440000 }
  space { unit 8; ratio 25% }
}
theme dusk {
  color { direct #aa0000; state #bb0000; default #cc0000; arg #dd0000 }
  space { unit 12; ratio 50% }
}
params { on bool = true }
def Chip(tone=color.default) { rect w=10 h=10 bg=tone }
col {
  rect w=10 h=10 bg=color.direct
  rect w=10 h=10 { when on { bg=color.state } }
  Chip
  Chip tone=color.arg
}
";
	let opts = Options { embed_assets: false, base_dir: PathBuf::new(), ..Options::default() };
	let (slir, diagnostics) = compile(src, &opts);
	assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
	let bytes = slab_slir::write(&slir.unwrap());
	let (mut instance, _) = slab_slir::instance(&bytes).unwrap();
	slab_kernel::frame::inst_set_env(&mut instance, 100.0, 100.0, 0, false, false);

	let colors = |frame: &slab_kernel::flatten::Frame| {
		frame
			.ops
			.iter()
			.filter_map(|op| match op {
				slab_kernel::flatten::FrameOp::Rect(rect) => Some(rect.bg),
				_ => None,
			})
			.collect::<Vec<_>>()
	};
	let word = |red| u32::from_le_bytes([red, 0, 0, 255]);

	let base = slab_kernel::frame::inst_frame(&mut instance, 0.0);
	assert_eq!(colors(&base), [word(0x11), word(0x22), word(0x33), word(0x44)]);
	assert_eq!(
		slab_kernel::frame::inst_get_token(&instance, "space.unit"),
		Some(slab_kernel::frame::TokenValue::Number(8.0))
	);
	assert_eq!(
		slab_kernel::frame::inst_get_token(&instance, "space.ratio"),
		Some(slab_kernel::frame::TokenValue::Text("25%"))
	);
	assert_eq!(
		slab_kernel::frame::inst_get_token(&instance, "color.state"),
		Some(slab_kernel::frame::TokenValue::Color(word(0x22)))
	);

	assert!(slab_kernel::frame::inst_set_theme(&mut instance, "dusk"));
	let dusk = slab_kernel::frame::inst_frame(&mut instance, 0.0);
	assert_eq!(colors(&dusk), [word(0xaa), word(0xbb), word(0xcc), word(0xdd)]);
	assert_eq!(
		slab_kernel::frame::inst_get_token(&instance, "space.unit"),
		Some(slab_kernel::frame::TokenValue::Number(12.0))
	);
	assert_eq!(
		slab_kernel::frame::inst_get_token(&instance, "space.ratio"),
		Some(slab_kernel::frame::TokenValue::Text("50%"))
	);
	assert_eq!(
		slab_kernel::frame::inst_get_token(&instance, "color.state"),
		Some(slab_kernel::frame::TokenValue::Color(word(0xbb)))
	);
	assert_eq!(slab_kernel::frame::inst_get_token(&instance, "missing"), None);

	assert!(slab_kernel::frame::inst_set_theme(&mut instance, ""));
	let restored = slab_kernel::frame::inst_frame(&mut instance, 0.0);
	assert_eq!(colors(&restored), colors(&base));
}

#[test]
fn list_each_submit_multiline_roundtrip_contract() {
	let src = r#"
params {
  items list(Row) = [
    Row(label="Alpha", hot=true),
    Row(label="Bravo")
  ]
}

def Row(label="", hot=false) export {
  row key=row {
    when hot { opacity=0.5 }
    text label key=label field=edit submit=send multiline
  }
}

col {
  each param.items key=list
}
"#;
	let opts = Options { embed_assets: false, base_dir: PathBuf::new(), ..Options::default() };
	let (slir, diags) = compile(src, &opts);
	assert!(!diags.has_errors(), "{:?}", diags.0);
	let slir = slir.unwrap();

	assert_eq!(slir.params.len(), 1);
	assert_eq!(slir.params[0].ty, 6);
	let default = slir.avals[slir.params[0].default as usize];
	assert_eq!(default.tag, slab_slir::aval::LIST_DEFAULT);
	assert_eq!((default.lo(), default.hi()), (0, 2));
	assert_eq!(slir.lists.len(), 2);
	assert_eq!(slir.lists[0].param, slab_slir::NONE);
	assert_eq!((slir.lists[1].param, slir.lists[1].field_off, slir.lists[1].field_len), (0, 2, 2));
	assert_eq!(slir.list_fields.len(), 4);
	assert_eq!(slir.list_items.len(), 2);
	assert_eq!(slir.list_item_values.len(), 4);

	let each = slir
		.nodes
		.kind
		.iter()
		.position(|&kind| kind == slab_slir::kind::EACH)
		.expect("each node");
	let template = slir.nodes.first_child[each] as usize;
	assert_ne!(template, slab_slir::NONE as usize);
	assert_ne!(slir.nodes.flags[template] & slab_slir::flags::DETACHED, 0);
	assert!(
		slir
			.nodes
			.flags
			.iter()
			.any(|flags| flags & slab_slir::flags::MULTILINE != 0)
	);
	assert!(
		slir
			.avals
			.iter()
			.any(|aval| aval.tag == slab_slir::aval::PROP_REF)
	);
	assert!(
		slir
			.conds
			.iter()
			.any(|cond| cond.kind == slab_slir::cond::PROP && cond.sym == 1)
	);
	assert!(slir.signals.iter().any(|signal| signal.2 == 1));
	assert!(slir.signals.iter().any(|signal| signal.2 == 2));

	let bytes = slab_slir::write(&slir);
	let decoded = slab_slir::read(&bytes).expect("list SLIR decodes");
	assert_eq!(decoded, slir);
	assert_eq!(slab_slir::write(&decoded), bytes);
}

#[test]
fn invalid_list_and_edit_contracts_are_diagnosed_atomically() {
	let src = r#"
params {
  missing list(Missing) = []
  private list(Private) = []
  items list(Row) = [Row(nope="x"), Row(label=12)]
  scalar text = ""
}

def Private(label="") { text label }
def Row(label="") export {
  col {
    hole nested
    each param.items key=nested
    text label
  }
}

col {
  each param.scalar key=wrong extra=1
  each param.items key=list
  rect multiline submit=bad
  text "x" submit=bad
}
"#;
	let opts = Options { embed_assets: false, base_dir: PathBuf::new(), ..Options::default() };
	let (slir, diags) = compile(src, &opts);
	assert!(slir.is_none());
	let codes: Vec<_> = diags.0.iter().map(|diag| diag.code).collect();
	assert!(codes.contains(&"list-def"));
	assert!(codes.contains(&"param-type"));
	assert!(codes.contains(&"each-target"));
	assert!(codes.contains(&"each-nest"));
	assert!(
		diags
			.0
			.iter()
			.any(|diag| diag.code == "attr" && diag.msg.contains("multiline"))
	);
	assert!(
		diags
			.0
			.iter()
			.any(|diag| diag.code == "attr" && diag.msg.contains("submit="))
	);
}

#[test]
fn runtime_and_compiled_paths_share_normalized_geometry() {
	let src = r#"
params { route text = "m1 2 h8 v6 q4 3 8 0 z" }
canvas w=40 h=20 {
  path "m1 2 h8 v6 q4 3 8 0 z" bg=none stroke=#123456
  path param.route bg=none stroke=#654321
}
"#;
	let opts = Options { embed_assets: false, base_dir: PathBuf::new(), ..Options::default() };
	let (slir, diags) = compile(src, &opts);
	assert!(!diags.has_errors(), "{:?}", diags.0);
	let slir = slir.unwrap();
	assert_eq!(slir.paths.len(), 1);

	let bytes = slab_slir::write(&slir);
	let (mut instance, _) = slab_slir::instance(&bytes).unwrap();
	slab_kernel::frame::inst_set_env(&mut instance, 40.0, 20.0, 0, false, false);
	let frame = slab_kernel::frame::inst_frame(&mut instance, 0.0);
	assert_eq!(frame.paths_rt.len(), 1);
	assert_eq!(frame.paths_rt[0].verbs, slir.paths[0].verbs);
	assert_eq!(frame.paths_rt[0].coords, slir.paths[0].coords);
	assert!(
		frame
			.ops
			.iter()
			.any(|op| matches!(op, slab_kernel::flatten::FrameOp::PathDraw(path) if path.path >= 0))
	);
	assert!(
		frame
			.ops
			.iter()
			.any(|op| matches!(op, slab_kernel::flatten::FrameOp::PathDraw(path) if path.path < 0))
	);

	assert!(slab_kernel::frame::inst_set_param(
		&mut instance,
		0,
		&slab_kernel::frame::ParamValue::Text("not path data".into()),
	));
	let invalid = slab_kernel::frame::inst_frame(&mut instance, 16.0);
	assert_eq!(
		invalid
			.ops
			.iter()
			.filter(|op| matches!(op, slab_kernel::flatten::FrameOp::PathDraw(_)))
			.count(),
		1
	);
	assert!(instance.st.diag_code.iter().any(|code| code == "attr"));
}

#[test]
fn icons_emit_detached_current_color_scaled_paths() {
	let src = r#"
params { glyph text = "alert" }
icon check viewbox=24 {
  path "M3 12 L8 17 L21 5 L18 3 L8 13 L6 10 Z"
}
icon alert {
  path "M12 3 L22 21 L2 21 Z" bg=none stroke=current stroke-w=2
}
row color=#2563EB {
  icon check size=12
  icon param.glyph size=18 color=#DC2626
  icon missing size=16
}
"#;
	let opts = Options { embed_assets: false, base_dir: PathBuf::new(), ..Options::default() };
	let (slir, diags) = compile(src, &opts);
	assert!(!diags.has_errors(), "{:?}", diags.0);
	let slir = slir.unwrap();
	assert_eq!(slir.icons.len(), 2);
	for icon in &slir.icons {
		assert_ne!(slir.nodes.flags[icon.node as usize] & slab_slir::flags::DETACHED, 0);
	}

	let bytes = slab_slir::write(&slir);
	let (mut instance, _) = slab_slir::instance(&bytes).unwrap();
	slab_kernel::frame::inst_set_env(&mut instance, 112.0, 32.0, 0, false, false);
	let frame = slab_kernel::frame::inst_frame(&mut instance, 0.0);
	let scales: Vec<_> = frame
		.ops
		.iter()
		.filter_map(|op| match op {
			slab_kernel::flatten::FrameOp::ScalePush(scale) => Some(scale),
			_ => None,
		})
		.collect();
	assert_eq!(scales.len(), 2);
	assert_eq!(scales[0].sx, 0.5);
	assert_eq!(scales[0].sy, 0.5);
	assert!(frame.ops.iter().any(|op| {
		matches!(
			 op,
			 slab_kernel::flatten::FrameOp::PathDraw(path)
				  if path.bg_kind == 1 && (path.bg == 4293616421 || path.bg == 0x2563EBFF)
		)
	}));
}
