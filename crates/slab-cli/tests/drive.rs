//! End-to-end SDP smoke tests over the spawned CLI's stdio and TCP transports.

use std::{
	io::{BufRead, BufReader, Write},
	net::TcpStream,
	path::PathBuf,
	process::{Command, Stdio},
};

use serde_json::{Value, json};

fn settings_example() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/10-settings.slab")
}

fn a11y_example() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../conformance/cases/a11y-dynamic.slab")
}

fn exchange(
	writer: &mut impl Write,
	reader: &mut impl BufRead,
	id: u64,
	method: &str,
	params: Value,
) -> Value {
	writeln!(writer, "{}", json!({"id": id, "method": method, "params": params}))
		.expect("write SDP request");
	writer.flush().expect("flush SDP request");

	let mut line = String::new();
	assert_ne!(
		reader.read_line(&mut line).expect("read SDP response"),
		0,
		"SDP closed before responding to {method}"
	);
	let response: Value = serde_json::from_str(&line).expect("parse SDP response");
	assert_eq!(response["id"], json!(id), "{response}");
	response
}

fn result(response: Value) -> Value {
	assert!(response.get("error").is_none(), "{response}");
	response["result"].clone()
}

fn center(rect: &Value) -> (f64, f64) {
	(
		rect["x"].as_f64().expect("rect x") + rect["w"].as_f64().expect("rect w") / 2.0,
		rect["y"].as_f64().expect("rect y") + rect["h"].as_f64().expect("rect h") / 2.0,
	)
}

fn final_key_segment(key: &str, expected: &str) -> bool {
	key.rsplit('/')
		.next()
		.is_some_and(|segment| segment.trim_start_matches('#') == expected)
}

fn decode_b64(encoded: &str) -> Vec<u8> {
	fn digit(byte: u8) -> u32 {
		match byte {
			b'A'..=b'Z' => u32::from(byte - b'A'),
			b'a'..=b'z' => u32::from(byte - b'a') + 26,
			b'0'..=b'9' => u32::from(byte - b'0') + 52,
			b'+' => 62,
			b'/' => 63,
			_ => panic!("invalid base64 digit"),
		}
	}

	assert_eq!(encoded.len() % 4, 0, "base64 must be padded");
	let bytes = encoded.as_bytes();
	let padding = usize::from(bytes.ends_with(b"=")) + usize::from(bytes.ends_with(b"=="));
	let mut decoded = Vec::with_capacity(bytes.len() / 4 * 3 - padding);
	for chunk in bytes.chunks_exact(4) {
		let packed = digit(chunk[0]) << 18
			| digit(chunk[1]) << 12
			| if chunk[2] == b'=' {
				0
			} else {
				digit(chunk[2]) << 6
			} | if chunk[3] == b'=' { 0 } else { digit(chunk[3]) };
		decoded.push(u8::try_from(packed >> 16 & 0xff).expect("base64 byte"));
		if chunk[2] != b'=' {
			decoded.push(u8::try_from(packed >> 8 & 0xff).expect("base64 byte"));
		}
		if chunk[3] != b'=' {
			decoded.push(u8::try_from(packed & 0xff).expect("base64 byte"));
		}
	}
	decoded
}

#[test]
fn drive_stdio_smoke() {
	let mut child = Command::new(env!("CARGO_BIN_EXE_slab"))
		.arg("drive")
		.arg(settings_example())
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.spawn()
		.expect("spawn slab drive");
	let mut writer = child.stdin.take().expect("drive stdin");
	let mut reader = BufReader::new(child.stdout.take().expect("drive stdout"));
	let mut id = 0_u64;
	let mut request = |method: &str, params: Value| {
		id += 1;
		exchange(&mut writer, &mut reader, id, method, params)
	};

	let info = result(request("protocol.info", json!({})));
	assert_eq!(info["name"], json!("sdp"));
	assert!(
		info["methods"]
			.as_array()
			.expect("method list")
			.contains(&json!("input.click"))
	);

	let found = result(request("scene.find", json!({"text": "Save"})));
	let matches = found["matches"].as_array().expect("scene.find matches");
	assert_eq!(matches.len(), 1, "{matches:?}");
	let (save_x, save_y) = center(&matches[0]["rect"]);
	let clicked = result(request("input.click", json!({"x": save_x, "y": save_y})));
	assert!(
		clicked["effects"]["signals"]
			.as_array()
			.expect("click signals")
			.iter()
			.any(|s| s["name"] == "save" && s["text"] == "" && s["item"] == ""),
		"{clicked}"
	);

	let tree = result(request("scene.tree", json!({})));
	let nodes = tree["nodes"].as_array().expect("scene nodes");
	let field = nodes
		.iter()
		.find(|node| {
			node["key"]
				.as_str()
				.is_some_and(|key| final_key_segment(key, "field"))
		})
		.expect("field scene node");
	let (field_x, field_y) = center(field);
	let focused = result(request("input.click", json!({"x": field_x, "y": field_y})));
	assert!(
		focused["effects"]["focus"]
			.as_str()
			.is_some_and(|key| final_key_segment(key, "field")),
		"{focused}"
	);

	let typed = result(request("input.text", json!({"text": "hi"})));
	assert!(
		typed["effects"]["signals"]
			.as_array()
			.expect("text signals")
			.iter()
			.any(|signal| signal["name"] == "draft" && signal["text"] == "hi"),
		"{typed}"
	);
	let summary = result(request("frame.summary", json!({})));
	assert!(
		summary["edits"]
			.as_array()
			.expect("summary edits")
			.iter()
			.any(|edit| edit["name"] == "draft" && edit["text"] == "hi"),
		"{summary}"
	);

	let _ = result(request("input.key", json!({"key": "Tab"})));
	let focus = result(request("focus.get", json!({})));
	assert_ne!(focus["key"], json!(""));
	assert_eq!(focus["visible"], json!(true));

	let registered = result(request(
		"img.register",
		json!({
			 "name": "smoke",
			 "w": 1,
			 "h": 1,
			 "format": 1,
			 "rgba": [17, 34, 51, 255],
		}),
	));
	let image = registered["img"].as_i64().expect("runtime image index");
	let info = result(request("img.info", json!({"img": image})));
	assert_eq!(info, json!({"w": 1, "h": 1, "format": 1, "generation": 1}));
	let data = result(request("img.data", json!({"img": image})));
	assert_eq!(decode_b64(data["data"].as_str().expect("runtime image bytes")), [17, 34, 51, 255]);
	assert_eq!(result(request("img.unregister", json!({"name": "smoke"}))), json!({"ok": true}));

	let png = result(request("render.png", json!({})));
	let png_bytes = decode_b64(png["data"].as_str().expect("inline PNG"));
	assert!(png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
	assert_eq!(png["width_px"], json!(800));
	assert_eq!(png["height_px"], json!(600));

	let cells = result(request("render.cells", json!({})));
	assert!(
		cells["text"]
			.as_str()
			.expect("cell text")
			.contains("Settings")
	);

	let scroll = nodes
		.iter()
		.find(|node| node["scroll"] == true)
		.expect("scroll scene node");
	let scroll_key = scroll["key"].as_str().expect("scroll key");
	let scrolled = result(request("scroll.set", json!({"key": scroll_key, "axis": 0, "off": 40})));
	assert_eq!(scrolled["off"], json!(0.0), "empty hole clamps to zero");

	let unknown = request("missing.method", json!({}));
	assert_eq!(unknown["error"]["code"], json!(-32601));
	let bad_key = request("input.click", json!({"key": "nope"}));
	assert_eq!(bad_key["error"]["code"], json!(-32000));

	let quit = result(request("protocol.quit", json!({})));
	assert_eq!(quit, json!({"ok": true}));
	drop(writer);
	assert!(child.wait().expect("wait for slab drive").success());
}

#[test]
fn drive_accessibility_scene_exports_typed_values() {
	let mut child = Command::new(env!("CARGO_BIN_EXE_slab"))
		.arg("drive")
		.arg(a11y_example())
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.spawn()
		.expect("spawn accessibility drive");
	let mut writer = child.stdin.take().expect("drive stdin");
	let mut reader = BufReader::new(child.stdout.take().expect("drive stdout"));
	let mut id = 0_u64;
	let mut request = |method: &str, params: Value| {
		id += 1;
		result(exchange(&mut writer, &mut reader, id, method, params))
	};

	assert_eq!(
		request(
			"param.set",
			json!({
				 "sets": {
					  "active": "#list/items~alpha/item",
					  "items": [
							{
								 "key": "alpha",
								 "label": "Runtime Alpha",
								 "check": "mixed",
								 "chosen": true,
								 "open": true,
								 "now": 3,
								 "announcement": "assertive",
								 "position": 1,
								 "total": 2
							},
							{
								 "key": "beta",
								 "label": "Runtime Beta",
								 "check": "false",
								 "chosen": false,
								 "open": false,
								 "now": 9,
								 "announcement": "off",
								 "position": 2,
								 "total": 2
							}
					  ]
				 }
			}),
		),
		json!({"ok": true})
	);

	let tree = request("scene.tree", json!({}));
	let nodes = tree["nodes"].as_array().expect("accessibility scene nodes");
	let root = nodes
		.iter()
		.find(|node| node["key"] == "#list")
		.expect("semantic list root");
	assert_eq!(root["role"], "listbox");
	assert_eq!(root["label"], "Dynamic records");
	assert_eq!(root["desc"], "Runtime list semantics");
	assert_eq!(root["active_descendant"], "#list/items~alpha/item");
	assert_eq!(root["controls"], "#detail");
	assert!(root["checked"].is_null());
	assert!(root["value_now"].is_null());

	let alpha = nodes
		.iter()
		.find(|node| node["key"] == "#list/items~alpha/item")
		.expect("runtime alpha option");
	assert_eq!(alpha["role"], "option");
	assert_eq!(alpha["label"], "Runtime Alpha");
	assert_eq!(alpha["desc"], "Runtime Alpha");
	assert_eq!(alpha["checked"], "mixed");
	assert_eq!(alpha["expanded"], true);
	assert_eq!(alpha["selected"], true);
	assert_eq!(alpha["value_now"], 3.0);
	assert_eq!(alpha["value_min"], 0.0);
	assert_eq!(alpha["value_max"], 10.0);
	assert_eq!(alpha["value_text"], "Runtime Alpha");
	assert_eq!(alpha["live"], "assertive");
	assert_eq!(alpha["live_atomic"], true);
	assert_eq!(alpha["level"], 1.0);
	assert_eq!(alpha["pos_in_set"], 1.0);
	assert_eq!(alpha["set_size"], 2.0);
	assert_eq!(alpha["modal"], Value::Null);
	assert_eq!(alpha["disabled"], false);
	assert_eq!(alpha["focused"], false);

	let beta = nodes
		.iter()
		.find(|node| node["key"] == "#list/items~beta/item")
		.expect("runtime beta option");
	assert_eq!(beta["checked"], false);
	assert_eq!(beta["expanded"], false);
	assert_eq!(beta["selected"], false);
	assert_eq!(beta["live"], "off");
	assert_eq!(beta["live_atomic"], false);
	assert_eq!(beta["pos_in_set"], 2.0);

	let detail = nodes
		.iter()
		.find(|node| node["key"] == "#detail")
		.expect("controlled detail node");
	assert_eq!(detail["role"], "region");
	assert_eq!(detail["modal"], true);

	let node = request("scene.node", json!({"key": "#list/items~alpha/item"}));
	assert_eq!(node["key"], "#list/items~alpha/item");
	assert_eq!(node["checked"], "mixed");
	assert_eq!(node["value_text"], "Runtime Alpha");
	assert_eq!(node["selected"], true);
	assert_eq!(node["states"]["focus"], false);
	assert_eq!(node["states"]["disabled"], false);
	assert!(node["states"].get("selected").is_none());

	assert_eq!(request("protocol.quit", json!({})), json!({"ok": true}));
	drop(writer);
	assert!(
		child
			.wait()
			.expect("wait for accessibility drive")
			.success()
	);
}

#[test]
fn drive_tcp_smoke() {
	let mut child = Command::new(env!("CARGO_BIN_EXE_slab"))
		.arg("drive")
		.arg(settings_example())
		.args(["--port", "0"])
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::piped())
		.spawn()
		.expect("spawn TCP slab drive");
	let mut stderr = BufReader::new(child.stderr.take().expect("drive stderr"));
	let mut banner = String::new();
	loop {
		banner.clear();
		if stderr.read_line(&mut banner).expect("read SDP banner") == 0 {
			panic!("EOF waiting for SDP banner");
		}
		if banner.trim().starts_with("sdp: listening on 127.0.0.1:") {
			break;
		}
	}
	let port = banner
		.trim()
		.strip_prefix("sdp: listening on 127.0.0.1:")
		.expect("SDP listener banner")
		.parse::<u16>()
		.expect("SDP listener port");

	let mut writer = TcpStream::connect(("127.0.0.1", port)).expect("connect to SDP");
	let mut reader = BufReader::new(writer.try_clone().expect("clone SDP stream"));
	let info = result(exchange(&mut writer, &mut reader, 1, "protocol.info", json!({})));
	assert_eq!(info["name"], json!("sdp"));
	let reload = result(exchange(&mut writer, &mut reader, 2, "doc.reload", json!({})));
	assert_eq!(reload["ok"], json!(true), "{reload}");
	let quit = result(exchange(&mut writer, &mut reader, 3, "protocol.quit", json!({})));
	assert_eq!(quit, json!({"ok": true}));
	drop(writer);
	assert!(child.wait().expect("wait for TCP slab drive").success());
}
