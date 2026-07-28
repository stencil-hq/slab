//! Template engine wrapper using `minijinja` for code generation.

use std::fmt::Write as _;

use minijinja::Environment;
use serde_json::Value;

/// `PascalCase` identifier (`row-clicked` -> `RowClicked`).
pub fn pascal(s: &str) -> String {
	let mut out = String::new();
	let mut up = true;
	for c in s.chars() {
		if c.is_alphanumeric() {
			if up {
				out.extend(c.to_uppercase());
				up = false;
			} else {
				out.push(c);
			}
		} else {
			up = true;
		}
	}
	if out.is_empty() {
		out.push('X');
	}
	out
}

/// `snake_case` identifier (`row-clicked` -> `row_clicked`).
pub fn snake(s: &str) -> String {
	let mut out = String::new();
	for c in s.chars() {
		if c.is_alphanumeric() {
			out.extend(c.to_lowercase());
		} else {
			out.push('_');
		}
	}
	if out.is_empty() {
		out.push('x');
	}
	out
}

/// `camelCase` identifier (`row-clicked` -> `rowClicked`).
pub fn camel(s: &str) -> String {
	let mut out = pascal(s);
	let Some(first) = out.chars().next() else {
		return out;
	};
	let lowered: String = first.to_lowercase().collect();
	out.replace_range(0..first.len_utf8(), &lowered);
	out
}

/// `kebab-case` identifier (`RowClicked` -> `row-clicked`).
pub fn kebab(s: &str) -> String {
	let mut out = String::new();
	for (i, c) in s.chars().enumerate() {
		if c.is_ascii_uppercase() {
			if i > 0 {
				out.push('-');
			}
			out.push(c.to_ascii_lowercase());
		} else if c.is_ascii_alphanumeric() || c == '-' {
			out.push(c);
		} else {
			out.push('-');
		}
	}
	out
}

/// Quoted Rust string literal (`"hello"` or `b"\x12\x34"`).
pub fn rust_string(s: &str) -> String {
	format!("{s:?}")
}

/// Quoted Go string literal.
pub fn go_string(s: &str) -> String {
	let mut out = String::with_capacity(s.len() + 2);
	out.push('"');
	for c in s.chars() {
		match c {
			'\\' => out.push_str("\\\\"),
			'"' => out.push_str("\\\""),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if (c as u32) < 0x20 || c as u32 == 0x7f => {
				let _ = write!(out, "\\x{:02x}", c as u32);
			},
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// Quoted JavaScript string literal.
pub fn js_string(s: &str) -> String {
	let mut out = String::from("\"");
	for c in s.chars() {
		match c {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if c < ' ' => {
				let _ = write!(out, "\\u{:04x}", u32::from(c));
			},
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// Creates a `minijinja::Environment` preconfigured with Slab codegen filters.
pub fn env() -> Environment<'static> {
	let mut env = Environment::new();
	env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);
	env.set_keep_trailing_newline(true);
	env.add_filter("pascal", |s: &str| pascal(s));
	env.add_filter("snake", |s: &str| snake(s));
	env.add_filter("camel", |s: &str| camel(s));
	env.add_filter("kebab", |s: &str| kebab(s));
	env.add_filter("upper", |s: &str| s.to_uppercase());
	env.add_filter("lower", |s: &str| s.to_lowercase());
	env.add_filter("rust_str", |s: &str| rust_string(s));
	env.add_filter("go_str", |s: &str| go_string(s));
	env.add_filter("js_str", |s: &str| js_string(s));
	env
}

/// Renders a template string using a `serde_json::Value` context.
pub fn render(template_src: &str, ctx: &Value) -> Result<String, String> {
	let mut env = env();
	env.add_template("main", template_src)
		.map_err(|e| format!("Template syntax error: {e}"))?;
	let tmpl = env
		.get_template("main")
		.map_err(|e| format!("Template load error: {e}"))?;
	tmpl
		.render(ctx)
		.map_err(|e| format!("Template render error: {e}"))
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn basic_minijinja_rendering() {
		let ctx = json!({
			"name": "row-clicked",
			"items": ["apple", "banana"]
		});
		assert_eq!(render("Hello {{ name | pascal }}!", &ctx).unwrap(), "Hello RowClicked!");
		assert_eq!(
			render(
				"{% for item in items %}{{ loop.index }}: {{ item }}{% if not loop.last %}, {% endif \
				 %}{% endfor %}",
				&ctx
			)
			.unwrap(),
			"1: apple, 2: banana"
		);
	}
}
