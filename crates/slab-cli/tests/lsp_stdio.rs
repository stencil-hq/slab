//! End-to-end smoke: spawn `slab lsp` as a child process and drive real
//! stdio Content-Length framing — initialize, didOpen a broken doc, read
//! publishDiagnostics, then shutdown/exit cleanly.

use std::{
	io::{Read, Write},
	process::{Command, Stdio},
};

use serde_json::{Value, json};

fn send(w: &mut impl Write, msg: &Value) {
	let data = serde_json::to_vec(msg).unwrap();
	write!(w, "Content-Length: {}\r\n\r\n", data.len()).unwrap();
	w.write_all(&data).unwrap();
}

/// Split a raw byte stream into framed JSON messages.
fn parse_frames(mut buf: &[u8]) -> Vec<Value> {
	let mut out = Vec::new();
	while let Some(sep) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
		let headers = std::str::from_utf8(&buf[..sep]).unwrap();
		let len: usize = headers
			.lines()
			.find_map(|l| {
				let (k, v) = l.split_once(':')?;
				k.trim()
					.eq_ignore_ascii_case("content-length")
					.then(|| v.trim().parse().ok())?
			})
			.expect("Content-Length header");
		let body = &buf[sep + 4..sep + 4 + len];
		out.push(serde_json::from_slice(body).unwrap());
		buf = &buf[sep + 4 + len..];
	}
	out
}

#[test]
fn lsp_stdio_smoke() {
	let mut child = Command::new(env!("CARGO_BIN_EXE_slab"))
		.arg("lsp")
		.stdin(Stdio::piped())
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.spawn()
		.expect("spawn slab lsp");
	let mut stdin = child.stdin.take().unwrap();
	let uri = "file:///t.slab";

	send(&mut stdin, &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}));
	send(
		&mut stdin,
		&json!({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
            "textDocument": {"uri": uri, "languageId": "slab", "version": 1,
                             "text": "col w=360 {\n  text )\n}\n"}}}),
	);
	send(&mut stdin, &json!({"jsonrpc": "2.0", "id": 2, "method": "shutdown"}));
	send(&mut stdin, &json!({"jsonrpc": "2.0", "method": "exit"}));
	drop(stdin);

	let mut raw = Vec::new();
	child
		.stdout
		.take()
		.unwrap()
		.read_to_end(&mut raw)
		.expect("read stdout");
	let status = child.wait().expect("wait");
	assert!(status.success(), "exit after shutdown must be 0: {status:?}");

	let frames = parse_frames(&raw);
	assert_eq!(frames.len(), 3, "{frames:?}");

	// initialize response
	assert_eq!(frames[0]["id"], json!(1));
	assert_eq!(frames[0]["result"]["capabilities"]["textDocumentSync"]["change"], json!(2));

	// publishDiagnostics with the parse error on line 1 (0-based)
	assert_eq!(frames[1]["method"], json!("textDocument/publishDiagnostics"));
	assert_eq!(frames[1]["params"]["uri"], json!(uri));
	let diags = frames[1]["params"]["diagnostics"].as_array().unwrap();
	assert!(
		diags.iter().any(|d| d["code"] == json!("parse")
			&& d["severity"] == json!(1)
			&& d["range"]["start"]["line"] == json!(1)),
		"{diags:?}"
	);

	// shutdown response
	assert_eq!(frames[2]["id"], json!(2));
	assert_eq!(frames[2]["result"], Value::Null);
}
