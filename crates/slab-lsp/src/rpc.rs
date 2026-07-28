//! Content-Length framed JSON-RPC loop over generic Read/Write, so tests can
//! drive the server in-memory and `slab lsp` can pass stdin/stdout.

use std::io::{BufRead, Write};

use serde_json::Value;

use crate::server::Server;

/// Write one framed message.
pub fn send(out: &mut impl Write, msg: &Value) -> std::io::Result<()> {
	let data = serde_json::to_vec(msg)?;
	write!(out, "Content-Length: {}\r\n\r\n", data.len())?;
	out.write_all(&data)?;
	out.flush()
}

/// Read one framed message body; `None` on clean EOF or missing header.
fn read_frame(input: &mut impl BufRead) -> std::io::Result<Option<Vec<u8>>> {
	let mut length: Option<usize> = None;
	let mut line = String::new();
	if input.read_line(&mut line)? == 0 {
		return Ok(None); // EOF
	}
	while !line.trim().is_empty() {
		if let Some((key, val)) = line.split_once(':')
			&& key.trim().eq_ignore_ascii_case("content-length")
		{
			length = val.trim().parse::<usize>().ok();
		}
		line.clear();
		if input.read_line(&mut line)? == 0 {
			return Ok(None);
		}
	}
	let Some(length) = length else {
		return Ok(Some(Vec::new())); // header block without a length: skip
	};
	let mut body = vec![0u8; length];
	std::io::Read::read_exact(input, &mut body)?;
	Ok(Some(body))
}

/// Serve LSP over the given streams until `exit`; returns the exit code.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> i32 {
	let mut srv = Server::new();
	while srv.running {
		let body = match read_frame(&mut input) {
			Ok(Some(b)) if b.is_empty() => continue,
			Ok(Some(b)) => b,
			Ok(None) | Err(_) => break,
		};
		let msg: Value = if let Ok(m) = serde_json::from_slice(&body) {
			m
		} else {
			let err = serde_json::json!({"jsonrpc": "2.0", "id": null,
                      "error": {"code": -32700, "message": "parse error"}});
			if send(&mut output, &err).is_err() {
				break;
			}
			continue;
		};
		for out in srv.handle(&msg) {
			if send(&mut output, &out).is_err() {
				return srv.exit_code;
			}
		}
	}
	srv.exit_code
}
