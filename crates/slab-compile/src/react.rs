//! `gen react` — the full `gen wc` file set plus a typed React wrapper
//! (`<stem>.tsx`). The wrapper registers the custom elements via a
//! side-effect import of the generated module, declares props/detail types
//! locally, applies params as element properties through a typed interface,
//! and wires signal handlers as `CustomEvent` listeners. React itself is the
//! consumer's dependency; nothing here adds a package.
//!
//! Outputs (deterministic, byte-stable across runs): everything
//! [`crate::wc::generate`] emits, plus `<stem>.tsx`.

use std::fmt::Write as _;

use slab_syntax::{ast::ParamType, diag::Diagnostics};

use crate::{
	Options,
	wc::{
		DocSpec, WcFile, WcOptions, collect_ts_list_types, doc_specs, files_of, js_string, pascal,
		ts_field_type, ts_list_type, ts_type,
	},
};

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

/// Emit the shared signal detail interfaces (mirrors the wc `.d.ts` shapes).
fn emit_signal_types(m: &mut String) {
	m.push_str(
		"\n/** Metadata captured for every emitted signal. */\nexport interface SignalMeta {\n\x20  \
		 readonly x: number;\n\x20  readonly y: number;\n\x20  readonly dx: number;\n\x20  readonly \
		 dy: number;\n\x20  readonly drag_dx: number;\n\x20  readonly drag_dy: number;\n\x20  \
		 readonly mods: number;\n\x20  readonly button: number;\n\x20  readonly clicks: \
		 number;\n\x20  readonly key: string;\n\x20  readonly src_key: string;\n\x20  readonly \
		 src_item: string;\n\x20  readonly cancelled: boolean;\n\x20  readonly dropped: \
		 boolean;\n\x20  /** Deepest hit-target canonical key on pointer-derived signals. */\n\x20  \
		 readonly hit_key?: string;\n\x20  /** Pressed key name on keyboard-driven activation. \
		 */\n\x20  readonly pressed_key?: string;\n}\n/** Detail carried by non-text signal \
		 CustomEvents. */\nexport interface SignalDetail {\n\x20  readonly item: string;\n\x20  \
		 readonly meta: SignalMeta;\n}\n/** Detail carried by Change, Submit, and Resize \
		 CustomEvents. */\nexport interface TextSignalDetail extends SignalDetail {\n\x20  readonly \
		 text: string;\n}\n",
	);
	m.push_str(
		"\n/** One layout or runtime diagnostic from the current frame. */\nexport interface \
		 FrameDiagnostic {\n\x20  readonly code: string;\n\x20  readonly line: number;\n\x20  \
		 readonly msg: string;\n}\n/** Detail carried by the `slab-diagnostics` CustomEvent. \
		 */\nexport interface SlabDiagnosticsDetail {\n\x20  readonly diagnostics: readonly \
		 FrameDiagnostic[];\n}\n",
	);
}

/// Emit the structural list item interfaces of one doc, deduped globally.
fn emit_list_item_types(
	m: &mut String,
	doc: &DocSpec,
	emitted: &mut std::collections::HashSet<String>,
) {
	for list in &doc.lists {
		let mut names = Vec::new();
		let mut order = Vec::new();
		collect_ts_list_types(doc, list.row as usize, pascal(&list.name), &mut names, &mut order);
		for row in order {
			let item_name = ts_list_type(&names, doc, row);
			if !emitted.insert(item_name.clone()) {
				continue;
			}
			let _ = writeln!(m, "\nexport interface {item_name} {{");
			m.push_str("   /** Stable per-item identity; defaults to the array index. */\n");
			m.push_str("   key?: string | number;\n");
			for field in &doc.list_rows[row] {
				let field_ty = if field.sub == 0 {
					ts_field_type(field)
				} else {
					format!("{}[]", ts_list_type(&names, doc, field.sub as usize - 1))
				};
				let _ = writeln!(m, "   {}?: {field_ty};", js_string(&field.name));
			}
			m.push_str("}\n");
		}
	}
}

fn emit_scene_keys(m: &mut String, doc: &DocSpec, comp: &str) {
	let _ = writeln!(
		m,
		"\n/** Canonical full scene keys for authored `#id` nodes. */\nexport const {comp}Keys = {{"
	);
	for (name, key) in &doc.keys {
		let _ = writeln!(m, "   {}: {},", js_string(name), js_string(key));
	}
	let _ = writeln!(
		m,
		"}} as const;\nexport type {comp}SceneKey = (typeof {comp}Keys)[keyof typeof {comp}Keys];"
	);
}
/// Emit the typed element interface the wrapper assigns properties through.
fn emit_element_interface(m: &mut String, doc: &DocSpec, comp: &str) {
	let _ = write!(
		m,
		"\n/** Imperative web-component surface for `<{}>`. */\nexport interface {comp}Element \
		 extends HTMLElement {{\n",
		doc.tag
	);
	m.push_str("   theme: string;\n");
	m.push_str("   getToken(path: string): string | number | undefined;\n");
	m.push_str("   setFieldText(key: string, text: string): boolean;\n");
	m.push_str("   fieldText(key: string): string | undefined;\n");
	m.push_str("   setFocus(key: string, visible?: boolean): boolean;\n");
	m.push_str("   clearFocus(): boolean;\n");
	m.push_str("   focusItem(each: string, index: number): boolean;\n");
	m.push_str("   focusNote(): string;\n");
	m.push_str("   focusedKey(): string | null;\n");
	m.push_str("   inEditField(): boolean;\n");
	m.push_str("   whenSettled(): Promise<void>;\n");
	m.push_str(
		"   readonly lastFrame: { readonly diagnostics: readonly FrameDiagnostic[] } | null;\n",
	);
	m.push_str(
		"   addEventListener(type: 'slab-diagnostics', listener: (event: \
		 CustomEvent<SlabDiagnosticsDetail>) => void, options?: boolean | AddEventListenerOptions): \
		 void;\n",
	);
	for (n, ty) in &doc.params {
		if n == "theme" {
			continue;
		}
		let _ = writeln!(m, "   '{n}': {};", prop_type(doc, n, ty));
	}
	m.push_str("}\n");
}

/// Emit the props interface of one wrapper component.
fn emit_props_interface(m: &mut String, doc: &DocSpec, comp: &str) {
	let _ = write!(m, "\n/** Props accepted by the `{comp}` wrapper. */\n");
	let _ = writeln!(m, "export interface {comp}Props {{");
	m.push_str("   /** Forwarded `id` attribute. */\n");
	m.push_str("   id?: string;\n");
	m.push_str("   /** Forwarded class list. */\n");
	m.push_str("   className?: string;\n");
	m.push_str("   /** Forwarded inline style. */\n");
	m.push_str("   style?: React.CSSProperties;\n");
	m.push_str("   /** Active theme name; empty selects authored values. */\n");
	m.push_str("   theme?: string;\n");
	for (n, ty) in &doc.params {
		if n == "theme" {
			continue;
		}
		let _ = writeln!(m, "   '{n}'?: {};", prop_type(doc, n, ty));
	}
	for (signal, has_text) in &doc.signals {
		let _ = writeln!(
			m,
			"   /** Handler for the `{signal}` CustomEvent. */\n   {}?: (detail: {}) => void;",
			handler_prop(signal),
			detail_type(*has_text)
		);
	}
	m.push_str("}\n");
}

/// Emit one wrapper function component.
fn emit_component(m: &mut String, doc: &DocSpec, comp: &str) {
	let _ = write!(
		m,
		"\n/** Typed React wrapper for `<{}>`; its ref exposes the element's imperative API. \
		 */\nexport const {comp} = React.forwardRef<{comp}Element, {comp}Props>(function \
		 {comp}(props, forwardedRef): React.JSX.Element {{\n\x20  const ref = \
		 React.useRef<{comp}Element | null>(null);\n\x20  React.useImperativeHandle(forwardedRef, \
		 () => ref.current as {comp}Element, []);\n",
		doc.tag
	);

	// Params flow to the element as properties (never attributes): the wc
	// module defines accessor pairs per param, and `theme` is an accessor on
	// the SlabElement base, so property assignment is the setParam path.
	m.push_str("   React.useEffect(() => {\n");
	m.push_str("      const el = ref.current;\n");
	m.push_str("      if (!el) {\n         return;\n      }\n");
	m.push_str(
		"      if (props.theme !== undefined) {\n         el.theme = props.theme;\n      }\n",
	);
	let mut deps = vec!["props.theme".to_string()];
	for (n, _) in &doc.params {
		if n == "theme" {
			continue;
		}
		let _ = write!(
			m,
			"      if (props['{n}'] !== undefined) {{\n         el['{n}'] = props['{n}'];\n      }}\n",
		);
		deps.push(format!("props['{n}']"));
	}
	let _ = writeln!(m, "   }}, [{}]);", deps.join(", "));

	for (signal, has_text) in &doc.signals {
		let prop = handler_prop(signal);
		let detail = detail_type(*has_text);
		let _ = write!(
			m,
			"   React.useEffect(() => {{\n\x20     const el = ref.current;\n\x20     const handler = \
			 props.{prop};\n\x20     if (!el || !handler) {{\n\x20        return undefined;\n\x20     \
			 }}\n\x20     const listener = (event: Event) =>\n\x20        handler((event as \
			 CustomEvent<{detail}>).detail);\n\x20     el.addEventListener('{signal}', \
			 listener);\n\x20     return () => el.removeEventListener('{signal}', listener);\n\x20  \
			 }}, [props.{prop}]);\n",
		);
	}

	// `class` (not `className`) reaches custom elements as the class
	// attribute across React versions.
	let _ = write!(
		m,
		"   return React.createElement('{}', {{\n\x20     ref,\n\x20     id: props.id,\n\x20     \
		 class: props.className,\n\x20     style: props.style,\n\x20  }});\n}});\n",
		doc.tag
	);
}

/// Emit the whole `<stem>.tsx` wrapper module.
fn emit_tsx(docs: &[DocSpec], stem: &str) -> String {
	let mut m = String::new();
	m.push_str("// GENERATED by `slab gen react` — do not edit.\n");
	m.push_str("import * as React from 'react';\n");
	let _ = writeln!(m, "import './{stem}.js';");
	emit_signal_types(&mut m);
	let mut signal_names = Vec::<&str>::new();
	for (name, _) in docs.iter().flat_map(|doc| &doc.signals) {
		if !signal_names.contains(&name.as_str()) {
			signal_names.push(name);
		}
	}
	if !signal_names.is_empty() {
		let union = signal_names
			.into_iter()
			.map(js_string)
			.collect::<Vec<_>>()
			.join(" | ");
		let _ = writeln!(m, "\nexport type SignalName = {union};");
	}
	let mut emitted_interfaces = std::collections::HashSet::new();
	for doc in docs {
		let comp = component_name(doc).to_string();
		emit_scene_keys(&mut m, doc, &comp);
		emit_list_item_types(&mut m, doc, &mut emitted_interfaces);
		emit_element_interface(&mut m, doc, &comp);
		emit_props_interface(&mut m, doc, &comp);
		emit_component(&mut m, doc, &comp);
	}
	m
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
