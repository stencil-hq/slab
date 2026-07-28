//! Host-agnostic C ABI for the Slab Drive Protocol, built for
//! `wasm32-unknown-unknown` without any language-specific bindings.
//!
//! The module embeds the compiler, the kernel, and the SDP session layer, so a
//! host that can run WebAssembly and pass bytes gets the whole Slab runtime:
//! compile `.slab` source on the fly with `doc.open`, drive input, and read
//! back terminal cells or scene JSON. The Go client (`clients/go`, wazero) and
//! the Python client (`packages/pyslab`, wasmtime) are thin wrappers over these
//! six exports.
//!
//! # Calling convention
//!
//! - Pointers and lengths are pointer-width integers (`i32` on wasm32).
//! - Request bodies cross as a `(ptr, len)` pair the host owns.
//! - [`slab_request`] answers with one length-prefixed block: a little-endian
//!   `u32` byte count, then that many UTF-8 bytes. The host reads the count,
//!   copies the payload, and releases the block with `slab_free(ptr, 4 + n)`.
//! - Handles are opaque nonzero `u32` values; handle `0` is never valid.
//!
//! # Example (host pseudocode)
//!
//! ```text
//! session = slab_session_new()
//! body    = slab_alloc(len); memory.write(body, line)
//! block   = slab_request(session, body, len)
//! slab_free(body, len)
//! n        = memory.read_u32_le(block)
//! response = memory.read(block + 4, n)
//! slab_free(block, 4 + n)
//! ```

use std::{
	alloc::{Layout, alloc, dealloc},
	cell::RefCell,
	collections::BTreeMap,
};

use slab_drive::Server;

/// ABI revision reported by [`slab_abi_version`].
///
/// Hosts MUST refuse a module whose version they do not implement. Bumps are
/// reserved for incompatible changes to the calling convention above; new SDP
/// methods are protocol-level and never bump this number.
pub const ABI_VERSION: u32 = 1;

/// Bytes reserved for the little-endian length prefix on a response block.
const HEADER: usize = 4;

thread_local! {
	 /// Live sessions keyed by handle, plus the next handle to mint.
	 ///
	 /// Handles are never reused: a host that keeps a freed handle sees a
	 /// protocol error instead of silently driving somebody else's session.
	 static SESSIONS: RefCell<Registry> = const {
		  RefCell::new(Registry { next: 1, live: BTreeMap::new() })
	 };
}

/// Handle table for the sessions this module owns.
struct Registry {
	/// Handle minted for the next [`slab_session_new`] call.
	next: u32,
	/// Sessions still alive, keyed by their minted handle.
	live: BTreeMap<u32, Server>,
}

/// Response returned when a host passes a handle that is not live.
const UNKNOWN_SESSION: &str =
	r#"{"id":null,"error":{"code":-32000,"message":"unknown session handle"}}"#;

/// Response returned when a request body is not UTF-8.
const NOT_UTF8: &str = r#"{"id":null,"error":{"code":-32700,"message":"request is not UTF-8"}}"#;

/// Returns the ABI revision this module implements.
#[unsafe(no_mangle)]
pub const extern "C" fn slab_abi_version() -> u32 {
	ABI_VERSION
}

/// Allocates `len` bytes of linear memory and returns its address.
///
/// Returns `0` for a zero-length or unrepresentable request. Release the block
/// with [`slab_free`] and the identical length.
#[unsafe(no_mangle)]
pub extern "C" fn slab_alloc(len: usize) -> *mut u8 {
	match block_layout(len) {
		// SAFETY: `block_layout` rejects zero-sized and unrepresentable layouts.
		Some(layout) => unsafe { alloc(layout) },
		None => std::ptr::null_mut(),
	}
}

/// Releases a block previously returned by [`slab_alloc`] or [`slab_request`].
///
/// A null pointer is a no-op.
///
/// # Safety
///
/// `ptr` must come from this module and `len` must be its allocation length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn slab_free(ptr: *mut u8, len: usize) {
	let Some(layout) = block_layout(len) else {
		return;
	};
	if ptr.is_null() {
		return;
	}
	// SAFETY: the contract above requires `ptr`/`len` to name a live block
	// allocated by this module with the same layout.
	unsafe { dealloc(ptr, layout) };
}

/// Creates an SDP session with no document loaded and returns its handle.
///
/// The session starts with the default 800x600 `gpu` environment; a terminal
/// host sends `env.set` with `client: "tui"` and its cell-derived pixel size.
#[unsafe(no_mangle)]
pub extern "C" fn slab_session_new() -> u32 {
	SESSIONS.with_borrow_mut(|registry| {
		let handle = registry.next;
		registry.next = registry.next.wrapping_add(1).max(1);
		registry.live.insert(handle, Server::new());
		handle
	})
}

/// Destroys a session and releases its document, kernel state, and handle.
///
/// Freeing an unknown or already-freed handle is a no-op. The handle stays
/// retired: later requests against it report an unknown-session error.
#[unsafe(no_mangle)]
pub extern "C" fn slab_session_free(handle: u32) {
	SESSIONS.with_borrow_mut(|registry| registry.live.remove(&handle));
}

/// Whether the session has ended through `protocol.quit`.
///
/// Returns `0` for a live session or an unknown handle, `1` after quit.
#[unsafe(no_mangle)]
pub extern "C" fn slab_session_quit(handle: u32) -> u32 {
	SESSIONS.with_borrow(|registry| u32::from(registry.live.get(&handle).is_some_and(Server::quit)))
}

/// Applies one NDJSON request line and returns its length-prefixed response.
///
/// The payload is always one complete UTF-8 JSON object without a trailing
/// newline, including for unknown handles and malformed input, so a host never
/// has to distinguish transport failure from protocol failure. Returns a null
/// pointer only when the response cannot be allocated.
///
/// # Safety
///
/// `ptr`/`len` must name a readable block that stays alive for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn slab_request(handle: u32, ptr: *const u8, len: usize) -> *mut u8 {
	// SAFETY: the host guarantees `ptr`/`len` name a readable block it owns.
	let bytes = unsafe { borrow(ptr, len) };
	let Ok(line) = str::from_utf8(bytes) else {
		return block(NOT_UTF8.as_bytes());
	};
	let response =
		SESSIONS.with_borrow_mut(|registry| registry.live.get_mut(&handle).map(|s| s.request(line)));
	match response {
		Some(response) => block(response.as_bytes()),
		None => block(UNKNOWN_SESSION.as_bytes()),
	}
}

/// Layout for a host-visible block, or `None` when unusable.
///
/// Every block is 4-byte aligned so a response header can be written as one
/// aligned `u32`, and so `slab_free` can reconstruct the layout from the length
/// alone.
fn block_layout(len: usize) -> Option<Layout> {
	if len == 0 {
		return None;
	}
	Layout::from_size_align(len, HEADER).ok()
}

/// Borrows a host-owned block as a byte slice.
///
/// # Safety
///
/// `ptr`/`len` must name a readable block that stays alive for the call.
const unsafe fn borrow<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
	if ptr.is_null() || len == 0 {
		return &[];
	}
	// SAFETY: delegated to the caller's contract.
	unsafe { std::slice::from_raw_parts(ptr, len) }
}

/// Copies `bytes` into a fresh length-prefixed host-owned block.
fn block(bytes: &[u8]) -> *mut u8 {
	let Ok(len) = u32::try_from(bytes.len()) else {
		return std::ptr::null_mut();
	};
	let ptr = slab_alloc(HEADER + bytes.len());
	if ptr.is_null() {
		return ptr;
	}
	// SAFETY: the block holds a 4-byte aligned header plus `bytes.len()` bytes.
	unsafe {
		ptr.cast::<u32>().write(len.to_le());
		std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.add(HEADER), bytes.len());
	}
	ptr
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Runs one request against a handle and returns the decoded response.
	fn request(handle: u32, line: &str) -> String {
		let ptr = slab_alloc(line.len());
		// SAFETY: `slab_alloc` returned `line.len()` writable bytes, and the
		// block stays alive and owned by this frame across the request.
		let response = unsafe {
			std::ptr::copy_nonoverlapping(line.as_ptr(), ptr, line.len());
			let response = slab_request(handle, ptr, line.len());
			slab_free(ptr, line.len());
			response
		};
		read_block(response)
	}

	/// Decodes and releases a length-prefixed response block.
	fn read_block(ptr: *mut u8) -> String {
		assert!(!ptr.is_null(), "response block allocated");
		// SAFETY: the module just wrote a header plus payload at `ptr`, and the
		// header counts exactly the payload bytes that follow it.
		let bytes = unsafe {
			let len = ptr.cast::<u32>().read().to_le() as usize;
			let bytes = borrow(ptr.add(HEADER), len).to_vec();
			slab_free(ptr, HEADER + len);
			bytes
		};
		String::from_utf8(bytes).expect("responses are UTF-8")
	}

	#[test]
	fn session_compiles_source_and_renders_cells() {
		let handle = slab_session_new();
		let open = request(
			handle,
			r#"{"id":1,"method":"doc.open","params":{"source":"col { text \"hi\" }"}}"#,
		);
		assert!(open.contains("\"ok\":true"), "{open}");
		request(handle, r#"{"method":"env.set","params":{"width":320,"height":96,"client":"tui"}}"#);
		let cells = request(handle, r#"{"method":"render.cells","params":{"plain":true}}"#);
		assert!(cells.contains("hi"), "{cells}");
		slab_session_free(handle);
	}

	#[test]
	fn freed_handles_retire_and_never_alias_a_new_session() {
		let first = slab_session_new();
		slab_session_free(first);
		let second = slab_session_new();
		assert_ne!(first, second, "a retired handle is never minted again");
		let stale = request(first, r#"{"method":"protocol.info"}"#);
		assert!(stale.contains("unknown session handle"), "{stale}");
		assert_eq!(slab_session_quit(second), 0);
		request(second, r#"{"method":"protocol.quit"}"#);
		assert_eq!(slab_session_quit(second), 1);
		slab_session_free(second);
		let response = request(second, r#"{"method":"protocol.info"}"#);
		assert!(response.contains("unknown session handle"), "{response}");
	}

	#[test]
	fn malformed_input_stays_inside_the_protocol() {
		let handle = slab_session_new();
		let response = request(handle, "not json");
		assert!(response.contains("-32700"), "{response}");
		// SAFETY: a null body with zero length is an explicitly allowed input.
		let empty = read_block(unsafe { slab_request(handle, std::ptr::null(), 0) });
		assert!(empty.contains("-32700"), "an empty body is a parse error, not a crash: {empty}");
		slab_session_free(handle);
	}
}
