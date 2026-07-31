//! Interaction-trace conformance (P5): drive the kernel with a scripted
//! event sequence and byte-compare the canonical output against a golden.
//!
//! Cases live in `conformance/cases/traces/<name>.json`:
//!
//! ```json
//! { "doc": "<stem under conformance/cases>",
//!   "env": {"vw":300,"vh":600,"client":"web","dark":false,"coarse":false},
//!   "params": [{"name":"hot","kind":"bool","value":1}],
//!   "steps": [ {"t":0,"event":{"type":"pointer-down","x":50,"y":16}},
//!              {"t":16},                                    // tick: frame only
//!              {"t":32,"state":{"key":"…","name":"disabled","on":true}},
//!              {"t":48,"env":{"dark":true}},
//!              {"t":64,"param":{"name":"hot","kind":"bool","value":1}},
//!              {"t":72,"scroll":{"key":"…","axis":0,"off":40}},
//!              {"t":74,"focus":{"key":"…","visible":true}},
//!              {"t":76,"hole":{"hole":0,"w":120,"h":48}},
//!              {"t":80,"hit":[10,66]} ],
//!   "expect": {"signals":[{"name":"save","text":"","item":""}], "focus_key":"…",
//!              "edits":[{"name":"name","text":"hi"}],
//!              "scroll":[{"key":"…","axis":0,"off":0}]} }
//! ```
//!
//! Every step runs `inst_frame(t)` FIRST, then applies its action and emits
//! one output line; the file ends with the kernel-formatted state summary
//! and the final frame.json. All variable lines are emitted by
//! `slab_kernel::dumpjson` (host glue is fixed ASCII), so native execution and
//! the Node-bound WASM runner produce byte-identical trace outputs against the
//! same goldens.
//!
//! Embedded `expect` blocks are validated here (signals in order, final
//! focus/edits/scroll against the summary line) on top of the byte compare.

use std::path::Path;

use crate::conformance::{client_code, compile_case, diff_window};

type TraceSignal = (String, String, String);
type TraceResult = Result<(String, Vec<TraceSignal>), String>;

fn drain_frame_signals(
	inst: &mut slab_kernel::frame::Instance,
	lines: &mut Vec<String>,
	signals: &mut Vec<TraceSignal>,
) {
	let effects = slab_kernel::frame::inst_take_signals(inst);
	if effects.sig_name.is_empty() {
		return;
	}
	for index in 0..effects.sig_name.len() {
		signals.push((
			inst.doc().strs[effects.sig_name[index] as usize].clone(),
			effects.sig_text[index].clone(),
			effects.sig_item[index].clone(),
		));
	}
	lines.push(slab_kernel::dumpjson::dump_effects(inst.doc(), &inst.st, &effects));
}

/// Maps the shared trace and SDP event names to kernel event type codes.
pub fn event_code(name: &str) -> Option<u32> {
	Some(match name {
		"pointer-move" => 0,
		"pointer-down" => 1,
		"pointer-up" => 2,
		"wheel" => 3,
		"key-down" => 4,
		"text" => 5,
		"paste" => 6,
		"copy" => 7,
		"cut" => 8,
		"composition-start" => 9,
		"composition-update" => 10,
		"composition-end" => 11,
		"blur" => 12,
		"resize" => 13,
		"close" => 14,
		"inspect" => 15,
		"activate" => 16,
		_ => return None,
	})
}

/// Packs shared trace and SDP modifier names into kernel modifier bits.
pub fn mods_of(v: &serde_json::Value) -> u32 {
	let mut m = 0;
	if let Some(list) = v.as_array() {
		for e in list {
			m |= match e.as_str().unwrap_or("") {
				"shift" => 1,
				"alt" => 2,
				"ctrl" => 4,
				"meta" => 8,
				_ => 0,
			};
		}
	}
	m
}

fn integral_number(value: &serde_json::Value) -> Option<f64> {
	let number = value.as_f64()?;
	(number.is_finite() && number.fract() == 0.0).then_some(number)
}

fn json_u32(value: &serde_json::Value) -> Option<u32> {
	let number = integral_number(value)?;
	(number >= 0.0 && number <= f64::from(u32::MAX)).then_some(number as u32)
}

fn json_i32(value: &serde_json::Value) -> Option<i32> {
	let number = integral_number(value)?;
	(number >= f64::from(i32::MIN) && number <= f64::from(i32::MAX)).then_some(number as i32)
}

/// Decodes optional preedit clause pairs. Malformed metadata degrades
/// atomically to the kernel's whole-preedit fallback.
fn clauses_of(value: &serde_json::Value) -> Vec<(i32, i32)> {
	let Some(entries) = value.as_array() else {
		return Vec::new();
	};
	entries
		.iter()
		.map(|entry| {
			let pair = entry.as_array()?;
			(pair.len() == 2).then_some((json_i32(&pair[0])?, json_i32(&pair[1])?))
		})
		.collect::<Option<Vec<_>>>()
		.unwrap_or_default()
}

fn json_u8(value: &serde_json::Value) -> Option<u8> {
	let number = integral_number(value)?;
	(number >= 0.0 && number <= f64::from(u8::MAX)).then_some(number as u8)
}

/// Builds a kernel event from the shared trace and SDP JSON shape.
pub fn build_event(v: &serde_json::Value) -> Result<slab_kernel::dispatch::Event, String> {
	let ty = v["type"].as_str().unwrap_or("");
	let etype = event_code(ty).ok_or_else(|| format!("unknown event type '{ty}'"))?;
	let u32_field = |name: &str| match v.get(name) {
		Some(value) => json_u32(value).ok_or_else(|| format!("event '{name}' must be a u32")),
		None => Ok(0),
	};
	Ok(slab_kernel::dispatch::Event {
		etype,
		x: v["x"].as_f64().unwrap_or(0.0),
		y: v["y"].as_f64().unwrap_or(0.0),
		dx: v["dx"].as_f64().unwrap_or(0.0),
		dy: v["dy"].as_f64().unwrap_or(0.0),
		button: u32_field("button")?,
		clicks: u32_field("clicks")?,
		key: v["key"].as_str().unwrap_or("").to_string(),
		text: v["text"].as_str().unwrap_or("").to_string(),
		clauses: if etype == 10 {
			clauses_of(&v["clauses"])
		} else {
			Vec::new()
		},
		mods: mods_of(&v["mods"]),
	})
}

/// Builds the kernel's typed scalar payload from a trace/SDP spelling.
pub fn typed_value_parts(
	kind_name: &str,
	raw: &serde_json::Value,
) -> Result<slab_kernel::frame::ParamValue, String> {
	use slab_kernel::frame::ParamValue;
	match kind_name {
		"text" => Ok(ParamValue::Text(
			raw.as_str()
				.ok_or_else(|| "text value must be a string".to_string())?
				.to_string(),
		)),
		"num" => Ok(ParamValue::Num(
			raw.as_f64()
				.ok_or_else(|| "numeric value must be a number".to_string())?,
		)),
		"pct" => Ok(ParamValue::Pct(
			raw.as_f64()
				.ok_or_else(|| "numeric value must be a number".to_string())?,
		)),
		"color" => Ok(ParamValue::Color(
			json_u32(raw).ok_or_else(|| "color value must be a u32".to_string())?,
		)),
		"bool" => Ok(ParamValue::Bool(
			raw.as_bool()
				.or_else(|| raw.as_f64().map(|value| value != 0.0))
				.ok_or_else(|| "bool value must be a number or boolean".to_string())?,
		)),
		"enum" => Ok(ParamValue::Enum(
			raw.as_str()
				.ok_or_else(|| "enum value must be a string".to_string())?
				.to_string(),
		)),
		other => Err(format!("unknown param kind '{other}'")),
	}
}

fn typed_value(v: &serde_json::Value) -> Result<slab_kernel::frame::ParamValue, String> {
	typed_value_parts(
		v["kind"].as_str().unwrap_or(""),
		v.get("value").unwrap_or(&serde_json::Value::Null),
	)
}

fn param_value(v: &serde_json::Value) -> Result<(String, slab_kernel::frame::ParamValue), String> {
	let name = v["name"].as_str().unwrap_or("").to_string();
	Ok((name, typed_value(v)?))
}

fn param_index(doc: &slab_kernel::slir::Doc, name: &str) -> Option<u32> {
	(0..doc.parm_name.len())
		.find(|&p| doc.strs[doc.parm_name[p] as usize] == name)
		.map(|p| p as u32)
}

/// Validated runtime-image registration data shared by trace and SDP hosts.
pub struct RuntimeImageInput {
	/// Runtime lookup name.
	pub(crate) name:   String,
	/// Declared pixel width.
	pub(crate) w:      u32,
	/// Declared pixel height.
	pub(crate) h:      u32,
	/// Kernel image format code.
	pub(crate) format: u32,
	/// Encoded PNG or straight-alpha RGBA8 bytes.
	pub(crate) data:   Vec<u8>,
}

const fn b64_digit(byte: u8) -> Option<u8> {
	match byte {
		b'A'..=b'Z' => Some(byte - b'A'),
		b'a'..=b'z' => Some(byte - b'a' + 26),
		b'0'..=b'9' => Some(byte - b'0' + 52),
		b'+' => Some(62),
		b'/' => Some(63),
		_ => None,
	}
}

/// Decodes padded RFC 4648 base64 without accepting malformed padding.
pub fn decode_b64(input: &str) -> Result<Vec<u8>, String> {
	let bytes = input.as_bytes();
	if !bytes.len().is_multiple_of(4) {
		return Err("base64 payload length must be a multiple of four".into());
	}
	let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
	for (index, chunk) in bytes.as_chunks::<4>().0.iter().enumerate() {
		let final_chunk = index + 1 == bytes.len() / 4;
		let a = b64_digit(chunk[0]).ok_or_else(|| "invalid base64 payload".to_string())?;
		let b = b64_digit(chunk[1]).ok_or_else(|| "invalid base64 payload".to_string())?;
		out.push(a << 2 | b >> 4);
		match (chunk[2], chunk[3]) {
			(b'=', b'=') if final_chunk && b & 0x0f == 0 => {},
			(c, b'=') if final_chunk => {
				let c = b64_digit(c).ok_or_else(|| "invalid base64 payload".to_string())?;
				if c & 0x03 != 0 {
					return Err("invalid base64 payload".into());
				}
				out.push(b << 4 | c >> 2);
			},
			(c, d) => {
				let c = b64_digit(c).ok_or_else(|| "invalid base64 payload".to_string())?;
				let d = b64_digit(d).ok_or_else(|| "invalid base64 payload".to_string())?;
				out.push(b << 4 | c >> 2);
				out.push(c << 6 | d);
			},
		}
	}
	Ok(out)
}

/// Validates and decodes one trace/SDP image-registration object.
pub fn runtime_image_input(
	value: &serde_json::Map<String, serde_json::Value>,
) -> Result<RuntimeImageInput, String> {
	let name = value
		.get("name")
		.and_then(serde_json::Value::as_str)
		.ok_or_else(|| "image 'name' must be a string".to_string())?
		.to_string();
	let w = value
		.get("w")
		.and_then(json_u32)
		.ok_or_else(|| "image 'w' must be a u32".to_string())?;
	let h = value
		.get("h")
		.and_then(json_u32)
		.ok_or_else(|| "image 'h' must be a u32".to_string())?;
	let format = value
		.get("format")
		.and_then(json_u32)
		.ok_or_else(|| "image 'format' must be a u32".to_string())?;
	let data = match (value.get("rgba"), value.get("png_b64")) {
		(Some(rgba), None) if format == 1 => rgba
			.as_array()
			.ok_or_else(|| "image 'rgba' must be a byte array".to_string())?
			.iter()
			.map(|byte| {
				json_u8(byte).ok_or_else(|| "image 'rgba' must contain only bytes".to_string())
			})
			.collect::<Result<Vec<_>, _>>()?,
		(None, Some(png)) if format == 0 => decode_b64(
			png.as_str()
				.ok_or_else(|| "image 'png_b64' must be a string".to_string())?,
		)?,
		(Some(_), Some(_)) => {
			return Err("image needs exactly one of 'rgba' or 'png_b64'".into());
		},
		(Some(_), None) => return Err("image 'rgba' requires format 1".into()),
		(None, Some(_)) => return Err("image 'png_b64' requires format 0".into()),
		(None, None) => return Err("image needs exactly one of 'rgba' or 'png_b64'".into()),
	};
	Ok(RuntimeImageInput { name, w, h, format, data })
}

/// Drive one trace; returns (output text, collected signals).
fn run_trace(bytes: &[u8], case: &serde_json::Value) -> TraceResult {
	let (mut inst, _) = slab_slir::instance(bytes)?;
	let env = &case["env"];
	let client_name = env["client"].as_str().unwrap_or("web");
	let client =
		client_code(client_name).ok_or_else(|| format!("unknown client '{client_name}'"))?;
	slab_kernel::frame::inst_set_env(
		&mut inst,
		env["vw"].as_f64().unwrap_or(800.0),
		env["vh"].as_f64().unwrap_or(0.0),
		client,
		env["dark"].as_bool().unwrap_or(false),
		env["coarse"].as_bool().unwrap_or(false),
	);
	if let Some(params) = case["params"].as_array() {
		for p in params {
			let (name, pv) = param_value(p)?;
			let ix =
				param_index(inst.doc(), &name).ok_or_else(|| format!("unknown param '{name}'"))?;
			if !slab_kernel::frame::inst_set_param(&mut inst, ix, &pv) {
				return Err(format!("param '{name}' rejected"));
			}
		}
	}
	let steps = case["steps"].as_array().cloned().unwrap_or_default();
	let t0 = steps.first().and_then(|s| s["t"].as_f64()).unwrap_or(0.0);
	slab_kernel::frame::inst_frame(&mut inst, t0);

	let mut lines: Vec<String> = Vec::new();
	let mut signals: Vec<TraceSignal> = Vec::new();
	let mut t_last = t0;
	for step in &steps {
		let t = step["t"].as_f64().unwrap_or(t_last);
		t_last = t;
		slab_kernel::frame::inst_frame(&mut inst, t);
		drain_frame_signals(&mut inst, &mut lines, &mut signals);
		if step["event"].is_object() {
			let ev = build_event(&step["event"])?;
			let eff = slab_kernel::frame::inst_dispatch(&mut inst, &ev);
			for k in 0..eff.sig_name.len() {
				signals.push((
					inst.doc().strs[eff.sig_name[k] as usize].clone(),
					eff.sig_text[k].clone(),
					eff.sig_item[k].clone(),
				));
			}
			lines.push(slab_kernel::dumpjson::dump_effects(inst.doc(), &inst.st, &eff));
		} else if step["state"].is_object() {
			let s = &step["state"];
			let name = s["name"].as_str().unwrap_or("");
			let on = s["on"].as_bool().unwrap_or(true);
			match s["key"].as_str() {
				Some(key) => {
					if !slab_kernel::frame::inst_set_node_state(&mut inst, key, name, on) {
						return Err(format!("unknown node key '{key}'"));
					}
				},
				None => slab_kernel::frame::inst_set_state(&mut inst, name, on),
			}
			lines.push("{\"set\":\"state\"}".into());
		} else if step["env"].is_object() {
			let e = &step["env"];
			let client = match e["client"].as_str() {
				Some(c) => client_code(c).ok_or_else(|| format!("unknown client '{c}'"))?,
				None => inst.st.env.client,
			};
			let vw = e["vw"].as_f64().unwrap_or(inst.st.env.vw);
			let vh = e["vh"].as_f64().unwrap_or(inst.st.env.vh);
			let dark = e["dark"].as_bool().unwrap_or(inst.st.env.dark);
			let coarse = e["coarse"].as_bool().unwrap_or(inst.st.env.coarse);
			slab_kernel::frame::inst_set_env(&mut inst, vw, vh, client, dark, coarse);
			lines.push("{\"set\":\"env\"}".into());
		} else if step["param"].is_object() {
			let (name, pv) = param_value(&step["param"])?;
			let ix =
				param_index(inst.doc(), &name).ok_or_else(|| format!("unknown param '{name}'"))?;
			let ok = slab_kernel::frame::inst_set_param(&mut inst, ix, &pv);
			lines.push(format!("{{\"set\":\"param\",\"ok\":{ok}}}"));
		} else if let Some(image_step) = step["img"].as_object() {
			match image_step.get("op") {
				Some(op) if op.as_str() == Some("unregister") => {
					let name = image_step
						.get("name")
						.and_then(serde_json::Value::as_str)
						.unwrap_or("");
					let ok = slab_kernel::frame::inst_img_unregister(&mut inst, name);
					lines.push(format!("{{\"set\":\"img\",\"op\":\"unregister\",\"ok\":{ok}}}"));
				},
				Some(op) => {
					return Err(format!("unknown img op '{}'", op.as_str().unwrap_or("")));
				},
				None => {
					let image = runtime_image_input(image_step)?;
					let img = slab_kernel::frame::inst_img_register(
						&mut inst,
						&image.name,
						image.w,
						image.h,
						image.format,
						&image.data,
					);
					lines.push(format!("{{\"set\":\"img\",\"img\":{img}}}"));
				},
			}
		} else if step["scroll"].is_object() {
			let s = &step["scroll"];
			let key = s["key"].as_str().unwrap_or("");
			let axis = match s.get("axis") {
				Some(axis) => json_u32(axis).unwrap_or(u32::MAX),
				None => 0,
			};
			let off = s["off"].as_f64().unwrap_or(0.0);
			let changed = slab_kernel::frame::inst_set_scroll(&mut inst, key, axis, off);
			let read = slab_kernel::frame::inst_get_scroll(&inst, key, axis) == off;
			lines.push(format!("{{\"set\":\"scroll\",\"changed\":{changed},\"read\":{read}}}"));
		} else if step["list"].is_object() {
			let list = &step["list"];
			let param = list["param"]
				.as_str()
				.and_then(|name| param_index(inst.doc(), name))
				.unwrap_or(u32::MAX);
			let path = list["path"].as_str().unwrap_or("");
			let op = list["op"].as_str().unwrap_or("");
			let ok = match op {
				"len" => {
					let n = json_i32(&list["n"]).unwrap_or(i32::MIN);
					slab_kernel::frame::inst_set_list_len(&mut inst, param, path, n)
						&& slab_kernel::frame::inst_list_len(&inst, param, path) == n
				},
				"field" => {
					let index = json_i32(&list["index"]).unwrap_or(i32::MIN);
					let field = list["field"].as_str().unwrap_or("");
					let value = typed_value(list)?;
					slab_kernel::frame::inst_set_list_field(&mut inst, param, path, index, field, &value)
				},
				"key" => {
					let index = json_i32(&list["index"]).unwrap_or(i32::MIN);
					let key = list["key"].as_str().unwrap_or("");
					slab_kernel::frame::inst_set_list_key(&mut inst, param, path, index, key)
				},
				other => return Err(format!("unknown list op '{other}'")),
			};
			lines.push(format!("{{\"set\":\"list\",\"op\":\"{op}\",\"ok\":{ok}}}"));
		} else if step["divider"].is_object() {
			let divider = &step["divider"];
			let key = divider["key"].as_str().unwrap_or("");
			let extent = divider["extent"].as_f64().unwrap_or(f64::NAN);
			let changed = slab_kernel::frame::inst_set_divider(&mut inst, key, extent);
			let read = slab_kernel::frame::inst_get_divider(&inst, key) == extent;
			lines.push(format!("{{\"set\":\"divider\",\"changed\":{changed},\"read\":{read}}}"));
		} else if step["reveal"].is_object() {
			let reveal = &step["reveal"];
			let key = reveal["key"].as_str().unwrap_or("");
			let margin = reveal["margin"].as_f64().unwrap_or(0.0);
			let ok = slab_kernel::frame::inst_reveal(&mut inst, key, margin);
			lines.push(format!("{{\"set\":\"reveal\",\"ok\":{ok}}}"));
		} else if step["item_extent"].is_object() {
			let item = &step["item_extent"];
			let each = item["each"].as_str().unwrap_or("");
			let index = json_i32(&item["index"]).unwrap_or(i32::MIN);
			let extent = item["extent"].as_f64().unwrap_or(f64::NAN);
			let ok = slab_kernel::frame::inst_set_item_extent(&mut inst, each, index, extent);
			lines.push(format!("{{\"set\":\"item_extent\",\"ok\":{ok}}}"));
		} else if step["reveal_item"].is_object() {
			let reveal = &step["reveal_item"];
			let each = reveal["each"].as_str().unwrap_or("");
			let index = json_i32(&reveal["index"]).unwrap_or(i32::MIN);
			let align = json_u32(&reveal["align"]).unwrap_or(u32::MAX);
			let ok = slab_kernel::frame::inst_reveal_item(&mut inst, each, index, align);
			lines.push(format!("{{\"set\":\"reveal_item\",\"ok\":{ok}}}"));
		} else if step["window"].is_object() {
			let each = step["window"]["each"].as_str().unwrap_or("");
			let (start, end) = slab_kernel::frame::inst_each_window(&inst, each);
			lines.push(format!("{{\"window\":[{start},{end}]}}"));
		} else if step["focus"].is_object() {
			let f = &step["focus"];
			let key = f["key"].as_str().unwrap_or("");
			let visible = f["visible"].as_bool().unwrap_or(true);
			let ok = slab_kernel::frame::inst_set_focus(&mut inst, key, visible);
			lines.push(format!("{{\"set\":\"focus\",\"ok\":{ok}}}"));
		} else if step["hole"].is_object() {
			let h = &step["hole"];
			let hole = json_u32(&h["hole"]).unwrap_or(u32::MAX);
			let w = h["w"].as_f64().unwrap_or(0.0);
			let height = h["h"].as_f64().unwrap_or(0.0);
			slab_kernel::frame::inst_set_hole_size(&mut inst, hole, w, height);
			lines.push("{\"set\":\"hole\"}".into());
		} else if step["hit"].is_array() {
			let xy = step["hit"].as_array().unwrap();
			let x = xy.first().and_then(|v| v.as_f64()).unwrap_or(0.0);
			let y = xy.get(1).and_then(|v| v.as_f64()).unwrap_or(0.0);
			let nodes = slab_kernel::frame::inst_hit(&inst, x, y);
			lines.push(slab_kernel::dumpjson::dump_hit(inst.doc(), &inst.st, &nodes));
		} else {
			lines.push("{\"tick\":true}".into());
		}
	}
	let fr = slab_kernel::frame::inst_frame(&mut inst, t_last);
	drain_frame_signals(&mut inst, &mut lines, &mut signals);
	let summary = slab_kernel::dumpjson::dump_trace_summary(inst.doc(), &inst.st, &inst);
	let frame_json = slab_kernel::dumpjson::dump(inst.doc(), &inst.st, &fr);
	let mut out = String::new();
	for l in &lines {
		out.push_str(l);
		out.push('\n');
	}
	out.push_str(&summary);
	out.push('\n');
	out.push_str(&frame_json);
	out.push('\n');
	Ok((out, signals))
}

/// Check the embedded expect block against collected signals + the summary
/// line (second to last) of the output.
fn check_expect(
	case: &serde_json::Value,
	output: &str,
	signals: &[(String, String, String)],
) -> Result<(), String> {
	let expect = &case["expect"];
	if !expect.is_object() {
		return Ok(());
	}
	if let Some(want) = expect["signals"].as_array() {
		if want.len() != signals.len() {
			return Err(format!(
				"expected {} signals, got {} ({signals:?})",
				want.len(),
				signals.len()
			));
		}
		for (k, w) in want.iter().enumerate() {
			let name = w["name"].as_str().unwrap_or("");
			let text = w["text"].as_str().unwrap_or("");
			let item = w["item"].as_str().unwrap_or("");
			if signals[k].0 != name || signals[k].1 != text || signals[k].2 != item {
				return Err(format!(
					"signal {k}: expected {name:?}/{text:?}/{item:?}, got {:?}/{:?}/{:?}",
					signals[k].0, signals[k].1, signals[k].2
				));
			}
		}
	}
	let mut it = output.lines().rev();
	let _frame = it.next();
	let summary: serde_json::Value = it
		.next()
		.and_then(|l| serde_json::from_str(l).ok())
		.ok_or("missing summary line")?;
	if let Some(fk) = expect.get("focus_key")
		&& &summary["focus"] != fk
	{
		return Err(format!("focus: expected {fk}, got {}", summary["focus"]));
	}
	if let Some(edits) = expect["edits"].as_array() {
		for e in edits {
			let name = e["name"].as_str().unwrap_or("");
			let text = e["text"].as_str().unwrap_or("");
			let found = summary["edits"]
				.as_array()
				.is_some_and(|list| list.iter().any(|s| s["name"] == name && s["text"] == text));
			if !found {
				return Err(format!("edit {name:?}: expected {text:?}, summary {}", summary["edits"]));
			}
		}
	}
	if let Some(scroll) = expect["scroll"].as_array() {
		for s in scroll {
			let key = s["key"].as_str().unwrap_or("");
			let axis = json_u32(&s["axis"]).unwrap_or(0);
			let off = s["off"].as_f64().unwrap_or(0.0);
			let found = summary["scroll"].as_array().is_some_and(|list| {
				list.iter().any(|e| {
					e["key"] == key
						&& json_u32(&e["axis"]) == Some(axis)
						&& (e["off"].as_f64().unwrap_or(f64::NAN) - off).abs() < 1e-6
				})
			});
			if !found {
				return Err(format!(
					"scroll {key:?} axis {axis}: expected {off}, summary {}",
					summary["scroll"]
				));
			}
		}
	}
	Ok(())
}

/// Run every trace case; returns (pass, fail). Compiled `.slir` for each
/// referenced doc is emitted under `<emit_dir>/traces/` for the TS runner.
pub fn run_traces(root: &Path, emit_dir: &Path, update: bool) -> (usize, usize) {
	let dir = root.join("conformance/cases/traces");
	let mut names: Vec<String> = match std::fs::read_dir(&dir) {
		Ok(rd) => rd
			.filter_map(|e| e.ok())
			.filter_map(|e| {
				let n = e.file_name().into_string().ok()?;
				n.strip_suffix(".json").map(str::to_string)
			})
			.collect(),
		Err(_) => Vec::new(),
	};
	names.sort();
	let trace_slir = emit_dir.join("traces");
	if let Err(e) = std::fs::create_dir_all(&trace_slir) {
		eprintln!("FAIL traces: {}: {e}", trace_slir.display());
		return (0, names.len());
	}
	let expected_dir = root.join("conformance/expected/traces");
	if update {
		let _ = std::fs::create_dir_all(&expected_dir);
	}
	let mut pass = 0usize;
	let mut fail = 0usize;
	for name in &names {
		let case: serde_json::Value = match std::fs::read_to_string(dir.join(format!("{name}.json")))
			.map_err(|e| e.to_string())
			.and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
		{
			Ok(v) => v,
			Err(e) => {
				eprintln!("FAIL trace {name}: {e}");
				fail += 1;
				continue;
			},
		};
		let doc = case["doc"].as_str().unwrap_or("");
		let src = root.join("conformance/cases").join(format!("{doc}.slab"));
		let bytes = match compile_case(&src) {
			Ok(b) => b,
			Err(e) => {
				eprintln!("FAIL trace {name}: compile\n{e}");
				fail += 1;
				continue;
			},
		};
		if let Err(e) = std::fs::write(trace_slir.join(format!("{doc}.slir")), &bytes) {
			eprintln!("FAIL trace {name}: write slir: {e}");
			fail += 1;
			continue;
		}
		let (output, signals) = match run_trace(&bytes, &case) {
			Ok(r) => r,
			Err(e) => {
				eprintln!("FAIL trace {name}: {e}");
				fail += 1;
				continue;
			},
		};
		if let Err(e) = check_expect(&case, &output, &signals) {
			eprintln!("FAIL trace {name}: expect: {e}");
			fail += 1;
			continue;
		}
		let golden = expected_dir.join(format!("{name}.trace.txt"));
		if update {
			if let Err(e) = std::fs::write(&golden, &output) {
				eprintln!("FAIL trace {name}: write golden: {e}");
				fail += 1;
				continue;
			}
			eprintln!("update trace {name}: wrote {} bytes", output.len());
			pass += 1;
			continue;
		}
		match std::fs::read_to_string(&golden) {
			Ok(want) if want == output => {
				eprintln!("ok trace {name}");
				pass += 1;
			},
			Ok(want) => {
				eprintln!("FAIL trace {name}: trace mismatch");
				eprintln!("{}", diff_window(&output, &want));
				fail += 1;
			},
			Err(e) => {
				eprintln!("FAIL trace {name}: {}: {e} (run with --update)", golden.display());
				fail += 1;
			},
		}
	}
	(pass, fail)
}
