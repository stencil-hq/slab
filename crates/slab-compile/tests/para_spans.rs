//! Paragraph span joining: adjacent spans contribute exactly the whitespace
//! their content contains (zero-gap butt joins, multi-space preservation),
//! and structural `when` branches on bool list fields select per item.

use slab_compile::{Options, compile};
use slab_kernel::{flatten::FrameOp, frame};

fn compile_instance(source: &str, width: f64, height: f64) -> frame::Instance {
    let (slir, diagnostics) = compile(
        source,
        &Options {
            embed_assets: false,
            ..Options::default()
        },
    );
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
    let mut pv = frame::ParamValue {
        kind: 4,
        num: 1.0,
        s: String::new(),
        rgba: 0,
        sym: String::new(),
    };
    assert!(frame::inst_set_list_field(
        &mut instance,
        1,
        "",
        0,
        "split",
        &pv
    ));
    let strings = painted_strings(&mut instance);
    assert_eq!(strings, vec!["SPLIT".to_string()], "split=true");

    pv.num = 0.0;
    assert!(frame::inst_set_list_field(
        &mut instance,
        1,
        "",
        0,
        "split",
        &pv
    ));
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
        (gap2 - 2.0 * gap1).abs() < 0.01,
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
        &frame::ParamValue {
            kind: 4,
            num: 1.0,
            s: String::new(),
            rgba: 0,
            sym: String::new(),
        },
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
fn svg_emits_strike_only_for_true_runs() {
    let (slir, diagnostics) = compile(
        r#"col w=200 h=60 {
  text "done" strike=true
  text "open" strike=false
}"#,
        &Options {
            embed_assets: false,
            ..Options::default()
        },
    );
    assert!(!diagnostics.has_errors(), "{:#?}", diagnostics.0);
    let slir = slir.expect("valid source");
    let bytes = slab_slir::write(&slir);
    let (mut instance, _) = slab_slir::instance(&bytes).expect("decode fixture");
    frame::inst_set_env(&mut instance, 200.0, 60.0, 0, false, false);
    let rendered = frame::inst_frame(&mut instance, 0.0);
    let svg =
        slab_compile::svg::render_svg(&slir, &[], &[], &[], &rendered, std::path::Path::new("."));
    assert_eq!(svg.matches("text-decoration=\"line-through\"").count(), 1);
    assert!(svg.contains(">done</text>"));
    assert!(svg.contains(">open</text>"));
}
