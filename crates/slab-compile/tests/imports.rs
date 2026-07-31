use std::{collections::HashMap, path::PathBuf};

use slab_compile::{Options, compile, input::apply_sets};
use slab_syntax::diag::{Diagnostics, Level};

fn virtual_options<I, K>(sources: I) -> Options
where
	I: IntoIterator<Item = (K, String)>,
	K: Into<String>,
{
	Options {
		embed_assets: false,
		base_dir:     PathBuf::from("/path/that/does/not/exist"),
		assets:       Some(HashMap::new()),
		sources:      Some(
			sources
				.into_iter()
				.map(|(path, source)| (path.into(), source))
				.collect(),
		),
		fonts:        HashMap::new(),
	}
}

fn diagnostic<'a>(diagnostics: &'a Diagnostics, code: &str) -> &'a slab_syntax::Diag {
	diagnostics
		.0
		.iter()
		.find(|diagnostic| diagnostic.code == code)
		.unwrap_or_else(|| panic!("missing {code} diagnostic: {:#?}", diagnostics.0))
}

#[test]
fn nested_missing_import_is_attributed_to_its_importer() {
	let options = virtual_options([("ui/panel.slab", "import \"../missing.slab\"\n".to_string())]);
	let (_, diagnostics) = compile("import \"./ui/panel.slab\"\n", &options);
	let missing = diagnostic(&diagnostics, "import-io");

	assert_eq!(missing.level, Level::Error);
	assert_eq!(missing.file.as_deref(), Some("ui/panel.slab"));
	assert_eq!(missing.line, 1);
	assert!(missing.msg.contains("missing.slab"), "{missing:?}");
}

#[test]
fn import_cycles_report_the_normalized_chain() {
	let options = virtual_options([
		("a.slab", "import \"dir/../b.slab\"\n".to_string()),
		("b.slab", "import \"./a.slab\"\n".to_string()),
	]);
	let (_, diagnostics) = compile("import \"a.slab\"\n", &options);
	let cycle = diagnostic(&diagnostics, "import-cycle");

	assert_eq!(cycle.file.as_deref(), Some("b.slab"));
	assert_eq!(cycle.msg, "import cycle: a.slab -> b.slab -> a.slab");
}

#[test]
fn import_depth_is_capped_at_thirty_two_modules() {
	let sources =
		(0..32).map(|index| (format!("{index}.slab"), format!("import \"{}.slab\"\n", index + 1)));
	let options = virtual_options(sources);
	let (_, diagnostics) = compile("import \"0.slab\"\n", &options);
	let depth = diagnostic(&diagnostics, "import-depth");

	assert_eq!(depth.file.as_deref(), Some("31.slab"));
	assert!(depth.msg.contains("exceeds 32"), "{depth:?}");
}

#[test]
fn imported_root_content_is_rejected_in_its_file() {
	let options = virtual_options([("module.slab", "col\n".to_string())]);
	let (_, diagnostics) = compile("import \"module.slab\"\n", &options);
	let content = diagnostic(&diagnostics, "import-content");

	assert_eq!(content.file.as_deref(), Some("module.slab"));
	assert_eq!(content.line, 1);
}

#[test]
fn root_definition_shadows_an_imported_definition() {
	let options =
		virtual_options([("module.slab", "def Card() { text \"IMPORTED_SENTINEL\" }\n".to_string())]);
	let source = r#"import "module.slab"
def Card() { text "ROOT_SENTINEL" }
Card
"#;
	let (slir, diagnostics) = compile(source, &options);

	assert!(!diagnostics.has_errors(), "{:#?}", diagnostics.0);
	let warning = diagnostic(&diagnostics, "dup-def");
	assert_eq!(warning.file, None);
	let dump = slab_slir::dump(&slir.expect("compiled document"));
	assert!(dump.contains("ROOT_SENTINEL"), "{dump}");
	assert!(!dump.contains("IMPORTED_SENTINEL"), "{dump}");
}

#[test]
fn first_cross_file_parameter_declaration_wins() {
	let options = virtual_options([(
		"module.slab",
		"params { title text = \"MODULE_DEFAULT\" }\n".to_string(),
	)]);
	let source =
		"import \"module.slab\"\nparams { title text = \"ROOT_DEFAULT\" }\ntext param.title\n";
	let (slir, diagnostics) = compile(source, &options);

	assert!(!diagnostics.has_errors(), "{:#?}", diagnostics.0);
	assert_eq!(diagnostic(&diagnostics, "dup-param").file, None);
	let slir = slir.expect("compiled document");
	let param = slir
		.params
		.iter()
		.find(|param| slir.str_at(param.name) == "title")
		.expect("title parameter");
	assert_eq!(slir.str_at(slir.avals[param.default as usize].lo()), "MODULE_DEFAULT");
}

#[test]
fn folded_parameter_name_collisions_are_errors() {
	let options =
		virtual_options([("module.slab", "params panel { open bool = true }\n".to_string())]);
	let source = "import \"module.slab\"\nparams { panel_open bool = false }\ncol\n";
	let (slir, diagnostics) = compile(source, &options);
	let collision = diagnostic(&diagnostics, "param-collide");

	assert!(slir.is_none());
	assert!(collision.msg.contains("panel.open"), "{collision:?}");
	assert!(collision.msg.contains("panel_open"), "{collision:?}");
	assert!(collision.msg.contains("module.slab:1"), "{collision:?}");
	assert!(collision.msg.contains("<root>:2"), "{collision:?}");
}

#[test]
fn grouped_parameters_flow_through_imports_conditions_and_host_sets() {
	let options = virtual_options([(
		"panel.slab",
		r"params panel {
  open bool = true
}
def Panel() {
  col {
    when panel.open { rect selected=param.panel.open }
  }
}
"
		.to_string(),
	)]);
	let (slir, diagnostics) = compile("import \"panel.slab\"\nPanel\n", &options);

	assert!(!diagnostics.has_errors(), "{:#?}", diagnostics.0);
	let slir = slir.expect("compiled document");
	assert!(
		slir
			.params
			.iter()
			.any(|param| slir.str_at(param.name) == "panel.open")
	);
	let bytes = slab_slir::write(&slir);
	let (mut instance, _) = slab_slir::instance(&bytes).expect("decoded instance");
	assert_eq!(
		slab_kernel::frame::inst_param_json(&instance, "panel.open").as_deref(),
		Some("true")
	);
	apply_sets(&mut instance, &[("panel.open".to_string(), "false".to_string())])
		.expect("dotted host set");
	assert_eq!(
		slab_kernel::frame::inst_param_json(&instance, "panel.open").as_deref(),
		Some("false")
	);
}

#[test]
fn unknown_dotted_condition_is_not_a_state_name() {
	let options = virtual_options(Vec::<(String, String)>::new());
	let (slir, diagnostics) = compile("col { when panel.missing { rect } }\n", &options);
	let reference = diagnostic(&diagnostics, "ref");

	assert!(slir.is_none());
	assert_eq!(reference.msg, "unknown param 'panel.missing' in condition");
}

#[test]
fn diamond_imports_are_included_once_at_first_occurrence() {
	let options = virtual_options([
		("shared.slab", "params { shared bool = true }\n".to_string()),
		("left.slab", "import \"shared.slab\"\n".to_string()),
		("right.slab", "import \"./shared.slab\"\n".to_string()),
	]);
	let source = "import \"left.slab\"\nimport \"right.slab\"\ncol { when shared { rect } }\n";
	let (slir, diagnostics) = compile(source, &options);

	assert!(!diagnostics.has_errors(), "{:#?}", diagnostics.0);
	assert!(
		diagnostics
			.0
			.iter()
			.all(|diagnostic| diagnostic.code != "dup-param")
	);
	assert_eq!(slir.expect("compiled document").params.len(), 1);
}

#[test]
fn imported_exports_compile_from_the_shared_closure() {
	let options = virtual_options([(
		"badge.slab",
		"def Badge(label=\"ready\") export { text label }\n".to_string(),
	)]);
	let source = "import \"badge.slab\"\nBadge\n";
	let (slir, diagnostics) = slab_compile::compile_with_exports(source, &options);

	assert!(slir.is_some(), "{:#?}", diagnostics.0);
	assert!(!diagnostics.has_errors(), "{:#?}", diagnostics.0);
}

#[test]
fn deep_duplicate_import_is_deduped_before_the_depth_guard() {
	let mut sources =
		vec![("shared.slab".to_string(), "params { shared bool = true }\n".to_string())];
	for index in 0..32 {
		let target = if index == 31 {
			"shared.slab".to_string()
		} else {
			format!("{}.slab", index + 1)
		};
		sources.push((format!("{index}.slab"), format!("import \"{target}\"\n")));
	}
	let options = virtual_options(sources);
	let source = "import \"shared.slab\"\nimport \"0.slab\"\ncol { when shared { rect } }\n";
	let (slir, diagnostics) = compile(source, &options);

	assert!(slir.is_some(), "{:#?}", diagnostics.0);
	assert!(
		diagnostics
			.0
			.iter()
			.all(|diagnostic| diagnostic.code != "import-depth"),
		"{:#?}",
		diagnostics.0
	);
}
