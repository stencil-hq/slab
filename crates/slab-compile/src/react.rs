//! React wrapper generator built on the custom-element generator.
//!
//! `gen react` emits the full `gen wc` file set plus a typed `<stem>.tsx`
//! wrapper. The wrapper registers custom elements through a side-effect import
//! of the generated module, declares props/detail types locally, applies params
//! as element properties through a typed interface, and wires signal handlers
//! as `CustomEvent` listeners. React itself is the consumer's dependency; this
//! module adds no package.
//!
//! Outputs (deterministic, byte-stable across runs): everything
//! [`crate::wc::generate`] emits, plus `<stem>.tsx`.

use serde_json::json;
use slab_syntax::{ast::ParamType, diag::Diagnostics};

use crate::{
	Options,
	wc::{
		DocSpec, WcFile, WcOptions, collect_ts_list_types, doc_specs, files_of, js_string, pascal,
		ts_field_type, ts_list_type, ts_type,
	},
};

const TEMPLATE: &str = include_str!("../templates/react.tmpl");

/// Component name of a doc: `SlabLogRowElement` → `LogRow`. Keeps the `Slab`
/// prefix when stripping it would not leave a valid identifier (a stem like
/// `10-settings` pascals to the digit-leading `10Settings`).
fn component_name(doc: &DocSpec) -> &str {
	let base = doc.class.strip_suffix("Element").unwrap_or(&doc.class);
	let trimmed = base.strip_prefix("Slab").unwrap_or(base);
	if trimmed
		.chars()
		.next()
		.is_some_and(|c| c.is_ascii_alphabetic())
	{
		trimmed
	} else {
		base
	}
}

/// `pressed` → `onPressed` (React handler prop).
fn handler_prop(signal: &str) -> String {
	format!("on{}", pascal(signal))
}

/// TSX type of one param prop; list params use the structural item types.
fn prop_type(doc: &DocSpec, name: &str, ty: &ParamType) -> String {
	if doc.lists.iter().any(|list| list.name == name) {
		format!("{}Item[]", pascal(name))
	} else {
		ts_type(ty).to_string()
	}
}

/// Detail type name of a signal `CustomEvent`.
const fn detail_type(has_text: bool) -> &'static str {
	if has_text {
		"TextSignalDetail"
	} else {
		"SignalDetail"
	}
}

fn emit_tsx(docs: &[DocSpec], stem: &str) -> String {
	let mut signal_names = Vec::<&str>::new();
	for (name, _) in docs.iter().flat_map(|doc| &doc.signals) {
		if !signal_names.contains(&name.as_str()) {
			signal_names.push(name);
		}
	}
	let signal_union = signal_names
		.iter()
		.map(|s| js_string(s))
		.collect::<Vec<_>>()
		.join(" | ");

	let mut emitted_interfaces = std::collections::HashSet::new();

	let docs_json: Vec<serde_json::Value> = docs
		.iter()
		.map(|doc| {
			let comp = component_name(doc).to_string();

			let keys_json: Vec<serde_json::Value> = doc
				.keys
				.iter()
				.map(|(name, key)| json!({ "name": name, "key": key }))
				.collect();

			let mut list_interfaces: Vec<serde_json::Value> = Vec::new();
			for list in &doc.lists {
				let mut names = Vec::new();
				let mut order = Vec::new();
				collect_ts_list_types(
					doc,
					list.row as usize,
					pascal(&list.name),
					&mut names,
					&mut order,
				);
				for row in order {
					let item_name = ts_list_type(&names, doc, row);
					if !emitted_interfaces.insert(item_name.clone()) {
						continue;
					}
					let fields_json: Vec<serde_json::Value> = doc.list_rows[row]
						.iter()
						.map(|field| {
							let field_ty = if field.sub == 0 {
								ts_field_type(field)
							} else {
								format!("{}[]", ts_list_type(&names, doc, field.sub as usize - 1))
							};
							json!({
								"name": field.name,
								"ts_type": field_ty,
							})
						})
						.collect();
					list_interfaces.push(json!({
						"name": item_name,
						"fields": fields_json,
					}));
				}
			}

			let params_json: Vec<serde_json::Value> = doc
				.params
				.iter()
				.map(|(n, ty)| {
					json!({
						"name": n,
						"prop_type": prop_type(doc, n, ty),
					})
				})
				.collect();

			let signals_json: Vec<serde_json::Value> = doc
				.signals
				.iter()
				.map(|(signal, has_text)| {
					json!({
						"name": signal,
						"handler_prop": handler_prop(signal),
						"detail_type": detail_type(*has_text),
					})
				})
				.collect();

			let mut deps = vec!["props.theme".to_string()];
			for (n, _) in &doc.params {
				if n != "theme" {
					deps.push(format!("props['{n}']"));
				}
			}

			json!({
				"tag": doc.tag,
				"comp": comp,
				"keys": keys_json,
				"list_interfaces": list_interfaces,
				"params": params_json,
				"signals": signals_json,
				"deps": deps,
			})
		})
		.collect();

	let ctx = json!({
		"stem": stem,
		"signal_names": signal_names,
		"signal_union": signal_union,
		"docs": docs_json,
	});

	crate::tmpl::render(TEMPLATE, &ctx).expect("react.tmpl render error")
}

/// Generate the full `gen wc` file set plus the typed React wrapper
/// `<stem>.tsx`. `None` on compile failure.
pub fn generate(
	src: &str,
	copts: &Options,
	w: &WcOptions,
	stem: &str,
) -> (Option<Vec<WcFile>>, Diagnostics) {
	let (docs, diags) = doc_specs(src, copts, w, stem);
	let Some(docs) = docs else {
		return (None, diags);
	};
	let mut files = files_of(&docs, w, stem);
	files.push(WcFile {
		name:  format!("{stem}.tsx"),
		bytes: emit_tsx(&docs, stem).into_bytes(),
		text:  true,
	});
	(Some(files), diags)
}

#[cfg(test)]
mod tests {
	use super::generate;
	use crate::{Options, wc::WcOptions};

	const SOURCE: &str = r#"
params {
  title   text = "Settings"
  volume  num = 40
  ratio   pct = 62%
  accent  color = #112233
  compact bool = false
}
col#root {
  box press=pressed
  divider w=6 resize=resized
  text param.title
}
"#;

	fn tsx_of(files: &[crate::wc::WcFile], name: &str) -> String {
		files
			.iter()
			.find(|file| file.name == name)
			.map(|file| String::from_utf8(file.bytes.clone()).expect("UTF-8 text file"))
			.expect("generated file present")
	}

	#[test]
	fn wrapper_declares_typed_props_handlers_and_property_assignment() {
		let options = WcOptions { tag: None, separate_ir: false };
		let (files, diagnostics) = generate(
			SOURCE,
			&Options { embed_assets: false, ..Options::default() },
			&options,
			"settings",
		);
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let files = files.expect("settings document should compile");
		let tsx = tsx_of(&files, "settings.tsx");

		// Side-effect import registers the custom elements.
		assert!(tsx.contains("// GENERATED by `slab gen react` — do not edit."));
		assert!(tsx.contains("import './settings.js';"));

		// Typed props interface with one optional prop per param.
		assert!(tsx.contains("export interface SettingsProps {"));
		assert!(tsx.contains("'title'?: string;"));
		assert!(tsx.contains("'volume'?: number;"));
		assert!(tsx.contains("'ratio'?: number | string;"));
		assert!(tsx.contains("'accent'?: string | number;"));
		assert!(tsx.contains("'compact'?: boolean;"));

		// Text-bearing and plain signals get distinctly typed handler props.
		assert!(tsx.contains("onPressed?: (detail: SignalDetail) => void;"));
		assert!(tsx.contains("onResized?: (detail: TextSignalDetail) => void;"));
		assert!(tsx.contains("el.addEventListener('pressed', listener);"));
		assert!(tsx.contains("export type SignalName = \"pressed\" | \"resized\";"));

		// Params land as properties through the typed element interface,
		// never via setAttribute.
		assert!(tsx.contains("interface SettingsElement extends HTMLElement {"));
		assert!(tsx.contains("el['volume'] = props['volume'];"));
		assert!(!tsx.contains("setAttribute"));

		// The component is exported with the tag pre-bound.
		assert!(tsx.contains("export const Settings = React.forwardRef<SettingsElement"));
		assert!(tsx.contains("React.useImperativeHandle(forwardedRef"));
		assert!(tsx.contains("focusedKey(): string | null"));
		assert!(tsx.contains("inEditField(): boolean"));
		assert!(tsx.contains("clearFocus(): boolean"));
		assert!(tsx.contains("focusItem(each: string, index: number): boolean"));
		assert!(tsx.contains("focusNote(): string"));
		assert!(tsx.contains("whenSettled(): Promise<void>"));
		assert!(tsx.contains("readonly lastFrame:"));
		assert!(tsx.contains("type: 'slab-diagnostics'"));
		assert!(tsx.contains("React.createElement('slab-settings', {"));
		assert!(tsx.contains("export const SettingsKeys = {"));
		assert!(tsx.contains("\"root\": \"#root\""));
		assert!(tsx.contains("export type SettingsSceneKey"));
	}

	#[test]
	fn wrapper_generation_is_deterministic_and_keeps_wc_files() {
		let options = WcOptions { tag: None, separate_ir: false };
		let copts = Options { embed_assets: false, ..Options::default() };
		let (first, diagnostics) = generate(SOURCE, &copts, &options, "settings");
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let first = first.expect("settings document should compile");
		let (second, _) = generate(SOURCE, &copts, &options, "settings");
		let second = second.expect("settings document should compile");

		assert_eq!(first.len(), second.len());
		for (a, b) in first.iter().zip(&second) {
			assert_eq!(a.name, b.name);
			assert_eq!(a.bytes, b.bytes, "{} differs between runs", a.name);
		}

		for expected in [
			"settings.js",
			"settings.d.ts",
			"slab-runtime.js",
			"wasm/slab_kernel_bg.wasm",
			"settings.tsx",
		] {
			assert!(first.iter().any(|file| file.name == expected), "missing {expected}");
		}
	}
	#[test]
	fn grouped_parameters_use_quoted_dotted_property_keys() {
		let source = r"
params editor { font_size num = 14 }
col { rect w=param.editor.font_size }
";
		let options = WcOptions { tag: None, separate_ir: false };
		let (files, diagnostics) = generate(
			source,
			&Options { embed_assets: false, ..Options::default() },
			&options,
			"editor",
		);
		assert!(!diagnostics.has_errors(), "{:?}", diagnostics.0);
		let files = files.expect("grouped parameter React wrapper");
		let tsx = tsx_of(&files, "editor.tsx");

		assert!(tsx.contains("'editor.font_size'?: number;"));
		assert!(tsx.contains("'editor.font_size': number;"));
		assert!(tsx.contains("el['editor.font_size'] = props['editor.font_size'];"));
		assert!(tsx.contains("[props.theme, props['editor.font_size']]"));
	}
}
