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

/// Decodes optional clause pairs. Any malformed payload degrades atomically
/// to no metadata so the kernel applies its whole-preedit fallback.
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

#[cfg(test)]
mod tests {
	use super::build_event;

	#[test]
	fn composition_clauses_reach_edit_state() {
		let source = serde_json::json!({
			"type": "composition-update",
			"text": "にほんご",
			"clauses": [[0, 2], [2, 4]]
		});
		let event = build_event(&source).expect("valid composition event");
		let mut state = slab_kernel::edit::es_new(0, "");
		slab_kernel::edit::composition_update_clauses(&mut state, &event.text, &event.clauses);
		assert_eq!(state.compose_clauses, [(0, 2), (2, 4)]);
	}

	#[test]
	fn malformed_composition_clauses_degrade_atomically() {
		for clauses in [
			serde_json::json!([[0, 2], ["bad", 4]]),
			serde_json::json!([[0, 2, 4]]),
			serde_json::json!({"start": 0, "end": 4}),
		] {
			let source = serde_json::json!({
				"type": "composition-update",
				"text": "にほんご",
				"clauses": clauses
			});
			assert!(
				build_event(&source)
					.expect("malformed clauses do not reject event")
					.clauses
					.is_empty()
			);
		}
	}
}
