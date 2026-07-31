//! Paragraph span joining: adjacent spans contribute exactly the whitespace
//! their content contains (zero-gap butt joins, multi-space preservation),
//! and structural `when` branches on bool list fields select per item.

use slab_compile::{Options, compile};
use slab_kernel::{flatten::FrameOp, frame, textm};

fn compile_instance(source: &str, width: f64, height: f64) -> frame::Instance {
	let (slir, diagnostics) =
		compile(source, &Options { embed_assets: false, ..Options::default() });
	assert!(!diagnostics.has_errors(), "{:#?}", diagnostics.0);
	let bytes = slab_slir::write(&slir.expect("valid source"));
	let (mut instance, _) = slab_slir::instance(&bytes).expect("decode fixture");
	frame::inst_set_env(&mut instance, width, height, 0, false, false);
	instance
}

fn painted_strings(instance: &mut frame::Instance) -> Vec<String> {
	let fr = frame::inst_frame(instance, 0.0);
	fr.ops
		.iter()
		.filter_map(|op| match op {
			FrameOp::Text(t) => Some(fr.strings[t.str_ref as usize].clone()),
			_ => None,
		})
		.collect()
}

#[test]
fn item_bool_field_when_selects_branch() {
	let source = r#"
def Item(no="", split=true, hunk=false, empty=false, selected=false, interactive=true) export {
  stack key=entry w=param.content_width h=20 act=diff-row role="row" label="Diff row" {
    when !interactive { inert }
    when empty { opacity=1 }
    when hover { bg=#FFFFFF08 }
    when selected { bg=#26BBff26 }
    when hunk {
      row key=hunkrow w=fill h=20 { text "HUNK" }
    }
    when !hunk {
      stack w=fill h=20 {
        when split {
          row key=splitrow w=fill h=20 { text "SPLIT" }
        }
        when !split {
          row key=unirow w=fill h=20 { text "UNIFIED" }
        }
      }
    }
    rect w=2 h=20 bg=#26BBFFFF self=start opacity=0 {
      when selected { opacity=1 }
    }
    rect w=0 h=0 bg=#00000000 inert
  }
}
params {
  content_width num = 400
  rows list(Item) = []
}
col w=420 h=100 scroll=both clip {
  each param.rows key=rows virtual item-extent=20 overscan=10
}
"#;
	let mut instance = compile_instance(source, 420.0, 100.0);
	assert!(frame::inst_set_list_len(&mut instance, 1, "", 1));
	let mut pv = frame::ParamValue::Bool(true);
	assert!(frame::inst_set_list_field(&mut instance, 1, "", 0, "split", &pv));
	let strings = painted_strings(&mut instance);
	assert_eq!(strings, vec!["SPLIT".to_string()], "split=true");

	pv = frame::ParamValue::Bool(false);
	assert!(frame::inst_set_list_field(&mut instance, 1, "", 0, "split", &pv));
	let strings = painted_strings(&mut instance);
	assert_eq!(strings, vec!["UNIFIED".to_string()], "split=false");
}

#[test]
fn spans_join_without_synthetic_gaps() {
	let source = r##"
col w=600 h=200 {
  para w=hug h=20 nowrap {
    span "#" color=#FF0000
    span "[cfg(unix)]" color=#00FF00
  }
  para w=hug h=20 nowrap {
    span "a" color=#FF0000
    span " b" color=#00FF00
  }
  para w=hug h=20 nowrap {
    span "a" color=#FF0000
    span "  b" color=#00FF00
  }
  para w=hug h=20 nowrap {
    span "ab" color=#FF0000
    span "cd" color=#FF0000
  }
  para w=hug h=20 nowrap {
    span "ab" color=#FF0000
    span " cd" color=#FF0000
  }
}
"##;
	let mut instance = compile_instance(source, 600.0, 200.0);
	let fr = frame::inst_frame(&mut instance, 0.0);
	let texts: Vec<(f64, f64, String)> = fr
		.ops
		.iter()
		.filter_map(|op| match op {
			FrameOp::Text(t) => Some((t.x, t.measured_w, fr.strings[t.str_ref as usize].clone())),
			_ => None,
		})
		.collect();

	// Adjacent spans without whitespace butt together with zero gap.
	let hash = texts.iter().find(|t| t.2 == "#").expect("# op");
	let rest = texts.iter().find(|t| t.2 == "[cfg(unix)]").expect("rest");
	assert!(
		(rest.0 - (hash.0 + hash.1)).abs() < 0.01,
		"expected zero gap, got {}",
		rest.0 - (hash.0 + hash.1)
	);

	// One and two source spaces produce proportional gaps.
	let a_ops: Vec<&(f64, f64, String)> = texts.iter().filter(|t| t.2 == "a").collect();
	let b_ops: Vec<&(f64, f64, String)> = texts.iter().filter(|t| t.2 == "b").collect();
	let gap1 = b_ops[0].0 - (a_ops[0].0 + a_ops[0].1);
	let gap2 = b_ops[1].0 - (a_ops[1].0 + a_ops[1].1);
	assert!(gap1 > 0.5, "single space gap missing: {gap1}");
	assert!(
		2.0f64.mul_add(-gap1, gap2).abs() < 0.01,
		"two spaces should double the gap: gap1={gap1} gap2={gap2}"
	);

	// Same-style spans merge into one segment; source spacing decides the join.
	assert!(texts.iter().any(|t| t.2 == "abcd"), "no-space merge");
	assert!(texts.iter().any(|t| t.2 == "ab cd"), "one-space merge");
}

#[test]
fn strike_resolves_for_text_spans_patches_params_and_list_props() {
	let source = r#"
def Item(done=false) export {
  para w=100 { span text="item" strike=done }
}
params {
  crossed bool = true
  rows list(Item) = []
}
col w=300 h=180 strike=param.crossed {
  text "bare" strike
  text "inherited"
  text "cleared" strike=false
  text "patched" {
    when crossed { strike=false }
  }
  para w=200 {
    span text="span-on" strike=true
    span text="span-off" strike=false
  }
  para w=200 { span text="span" strike=true }
  each param.rows key=rows
}
"#;
	let mut instance = compile_instance(source, 300.0, 180.0);
	assert!(frame::inst_set_list_len(&mut instance, 1, "", 1));
	assert!(frame::inst_set_list_field(
		&mut instance,
		1,
		"",
		0,
		"done",
		&frame::ParamValue::Bool(true),
	));
	let fr = frame::inst_frame(&mut instance, 0.0);
	let runs: Vec<(&str, bool)> = fr
		.ops
		.iter()
		.filter_map(|op| match op {
			FrameOp::Text(text) => Some((fr.strings[text.str_ref as usize].as_str(), text.strike)),
			_ => None,
		})
		.collect();
	assert!(runs.contains(&("bare", true)));
	assert!(runs.contains(&("inherited", true)));
	assert!(runs.contains(&("cleared", false)));
	assert!(runs.contains(&("patched", false)));
	assert!(runs.contains(&("span", true)));
	assert!(runs.contains(&("span-on", true)));
	assert!(runs.contains(&("span-off", false)));
	assert!(runs.contains(&("item", true)));
}

#[test]
fn italic_and_underline_inherit_and_split_paragraph_runs() {
	let mut instance = compile_instance(
		r#"
col w=300 h=140 italic underline {
  text "inherited"
  text "cleared" italic=false underline=false
  para w=260 {
    span text="styled"
    span text=" plain" italic=false underline=false
  }
}
"#,
		300.0,
		140.0,
	);
	let rendered = frame::inst_frame(&mut instance, 0.0);
	let runs: Vec<_> = rendered
		.ops
		.iter()
		.filter_map(|operation| {
			let FrameOp::Text(text) = operation else {
				return None;
			};
			Some((
				rendered.strings[text.str_ref as usize].as_str(),
				text.italic,
				text.underline,
				text.underline_offset,
				text.underline_thickness,
			))
		})
		.collect();
	for name in ["inherited", "styled"] {
		let run = runs.iter().find(|run| run.0 == name).expect("styled run");
		assert!(run.1 && run.2, "{name} inherits both decorations");
		assert!(run.3 > 0.0, "{name} has a font-derived underline offset");
		assert!(run.4 > 0.0, "{name} has a font-derived underline thickness");
	}
	for name in ["cleared", "plain"] {
		let run = runs.iter().find(|run| run.0 == name).expect("cleared run");
		assert!(!run.1 && !run.2, "{name} clears both decorations");
	}
}

#[test]
fn span_background_paints_only_behind_its_measured_run() {
	let mut instance = compile_instance(
		r#"para w=240 h=30 {
  span text="code" bg=#223344FF
  span text=" prose"
}"#,
		240.0,
		30.0,
	);
	let rendered = frame::inst_frame(&mut instance, 0.0);
	let code = rendered
		.ops
		.iter()
		.find_map(|operation| {
			let FrameOp::Text(text) = operation else {
				return None;
			};
			(rendered.strings[text.str_ref as usize] == "code").then_some(text)
		})
		.expect("code text run");
	let background = rendered
		.ops
		.iter()
		.find_map(|operation| {
			let FrameOp::Rect(rect) = operation else {
				return None;
			};
			(rect.bg_kind == 1 && rect.bg == 0xff44_3322).then_some(rect)
		})
		.unwrap_or_else(|| panic!("span background missing from {:#?}", rendered.ops));
	assert!((background.x - code.x).abs() < 0.001);
	assert!((background.w - code.measured_w).abs() < 0.001);
	assert_eq!(
		rendered
			.ops
			.iter()
			.filter(|operation| matches!(operation, FrameOp::Rect(rect) if rect.bg_kind != 0))
			.count(),
		1,
		"unstyled sibling emits no background"
	);
}

#[test]
fn authored_weight_is_preserved_without_four_weight_snapping() {
	let source = r#"text "heavy" family="Inter" weight=800"#;
	let (slir, diagnostics) =
		compile(source, &Options { embed_assets: false, ..Options::default() });
	assert!(!diagnostics.has_errors(), "{:#?}", diagnostics.0);
	let slir = slir.expect("valid source");
	assert!(slir.fonts.iter().any(|font| font.weight == 800));
	let bytes = slab_slir::write(&slir);
	let (mut instance, _) = slab_slir::instance(&bytes).expect("decode fixture");
	frame::inst_set_env(&mut instance, 200.0, 40.0, 0, false, false);
	let rendered = frame::inst_frame(&mut instance, 0.0);
	let run = rendered
		.ops
		.iter()
		.find_map(|operation| {
			let FrameOp::Text(text) = operation else {
				return None;
			};
			Some(text)
		})
		.expect("text run");
	assert_eq!(run.weight, 800);
	assert_eq!(instance.doc().font_weight[run.font as usize], 800);
}

#[test]
fn nowrap_paragraph_ellipsizes_once_across_styled_spans() {
	let mut instance = compile_instance(
		r#"col w=320 h=80 {
  para w=90 nowrap ellipsis {
    span text="Alpha " color=#FF0000
    span text="Beta Gamma Delta" color=#00FF00
  }
  para w=300 nowrap ellipsis {
    span text="Alpha " color=#FF0000
    span text="Beta Gamma Delta" color=#00FF00
  }
}"#,
		320.0,
		80.0,
	);
	let frame = frame::inst_frame(&mut instance, 0.0);
	let runs: Vec<_> = frame
		.ops
		.iter()
		.filter_map(|op| match op {
			FrameOp::Text(text) => Some((
				text.node,
				frame.strings[text.str_ref as usize].clone(),
				text.y_baseline,
				text.color,
			)),
			_ => None,
		})
		.collect();
	let narrow_node = runs
		.iter()
		.find(|(_, text, ..)| text.ends_with('…'))
		.map(|(node, ..)| *node)
		.expect("narrow paragraph emits an ellipsis");
	let narrow: Vec<_> = runs.iter().filter(|run| run.0 == narrow_node).collect();
	assert!(
		narrow.iter().all(|run| run.2 == narrow[0].2),
		"nowrap paragraph must have one baseline"
	);
	assert_eq!(narrow.iter().map(|run| run.1.as_str()).collect::<String>(), "AlphaBeta…");

	let wide_node = runs
		.iter()
		.find(|(_, text, ..)| text == "Beta Gamma Delta")
		.map(|(node, ..)| *node)
		.expect("wide paragraph retains the complete second run");
	let wide: Vec<_> = runs.iter().filter(|run| run.0 == wide_node).collect();
	assert!(
		wide.iter().all(|run| run.2 == wide[0].2),
		"wide nowrap paragraph must also remain one line"
	);
	assert_eq!(wide.iter().map(|run| run.1.as_str()).collect::<String>(), "AlphaBeta Gamma Delta");
	assert_eq!(
		narrow.last().expect("narrow paragraph run").3,
		wide.last().expect("wide paragraph run").3,
		"ellipsis inherits the last retained run color"
	);
}

#[test]
fn strike_never_changes_text_measurement() {
	let mut instance = compile_instance(
		r#"col w=200 h=60 {
  text "same"
  text "same" strike
}"#,
		200.0,
		60.0,
	);
	let frame = frame::inst_frame(&mut instance, 0.0);
	let runs: Vec<_> = frame
		.ops
		.iter()
		.filter_map(|op| match op {
			FrameOp::Text(text) => Some((text.measured_w, text.strike)),
			_ => None,
		})
		.collect();
	assert_eq!(runs.len(), 2);
	assert_eq!(runs[0].0, runs[1].0);
	assert!(!runs[0].1);
	assert!(runs[1].1);
}

#[test]
fn svg_emits_strike_only_for_true_runs() {
	let (slir, diagnostics) = compile(
		r#"col w=200 h=60 {
  text "done" strike=true
  text "open" strike=false
}"#,
		&Options { embed_assets: false, ..Options::default() },
	);
	assert!(!diagnostics.has_errors(), "{:#?}", diagnostics.0);
	let slir = slir.expect("valid source");
	let bytes = slab_slir::write(&slir);
	let (mut instance, _) = slab_slir::instance(&bytes).expect("decode fixture");
	frame::inst_set_env(&mut instance, 200.0, 60.0, 0, false, false);
	let rendered = frame::inst_frame(&mut instance, 0.0);
	let svg = slab_compile::svg::render_svg(
		&slir,
		instance.doc(),
		&[],
		&[],
		&[],
		&rendered,
		std::path::Path::new("."),
	);
	assert_eq!(svg.matches("text-decoration=\"line-through\"").count(), 1);
	assert!(svg.contains(">done</text>"));
	assert!(svg.contains(">open</text>"));
}

#[test]
fn runtime_glyph_diagnostics_are_once_per_family_and_codepoint() {
	let mut instance = compile_instance(
		r#"
params { value text = "✕" }
col {
  text param.value family="sans"
  text param.value family="sans"
}
"#,
		200.0,
		80.0,
	);
	let first = frame::inst_frame(&mut instance, 0.0);
	let missing: Vec<_> = first
		.diagnostics
		.iter()
		.filter(|diagnostic| diagnostic.code == "glyph-missing")
		.collect();
	assert_eq!(missing.len(), 1, "{missing:?}");
	assert!(missing[0].msg.contains("'✕'"), "{missing:?}");
	assert!(missing[0].msg.contains("U+2715"), "{missing:?}");
	assert!(missing[0].msg.contains("family 'sans'"), "{missing:?}");

	let second = frame::inst_frame(&mut instance, 0.0);
	assert!(
		second
			.diagnostics
			.iter()
			.all(|diagnostic| diagnostic.code != "glyph-missing"),
		"{:?}",
		second.diagnostics
	);
}

#[test]
fn kernel_shapes_kerning_and_emits_visual_bidi_runs() {
	let mut instance =
		compile_instance(r#"col w=300 h=60 { text "AV office אבג" nowrap }"#, 300.0, 60.0);
	let rendered = frame::inst_frame(&mut instance, 0.0);
	let runs: Vec<_> = rendered
		.ops
		.iter()
		.filter_map(|operation| match operation {
			FrameOp::Text(text) => Some((text, &rendered.strings[text.str_ref as usize])),
			_ => None,
		})
		.collect();
	let (latin, _) = runs
		.iter()
		.find(|(_, content)| content.contains("AV"))
		.expect("Latin shaped run");
	let nominal_width: f64 = runs
		.iter()
		.find(|(_, content)| content.contains("AV"))
		.expect("Latin shaped run")
		.1
		.chars()
		.map(|codepoint| {
			textm::char_w(instance.doc(), latin.font, latin.size, latin.tracking, u32::from(codepoint))
		})
		.sum();
	assert!(
		(latin.measured_w - nominal_width).abs() <= 0.01,
		"shaped runs rebase into FRAME.md-normative advance space (kerning deltas fold back to \
		 per-codepoint advances so measurement, wrapping, and paint agree)"
	);
	let (rtl, _) = runs
		.iter()
		.find(|(text, content)| text.rtl && content.contains('א'))
		.expect("RTL shaped run");
	let start = usize::try_from(rtl.glyph_off).expect("glyph offset");
	let end = start + usize::try_from(rtl.glyph_len).expect("glyph count");
	let clusters: Vec<_> = rendered.glyphs[start..end]
		.iter()
		.map(|glyph| glyph.cluster)
		.collect();
	assert!(
		clusters.windows(2).all(|pair| pair[0] >= pair[1]),
		"RTL glyph clusters must be in visual order: {clusters:?}"
	);
}
