#![allow(clippy::collapsible_if, clippy::absurd_extreme_comparisons)]
//! `slab-lsp` — hand-rolled stdio LSP server for the slab language (SPEC §12
//! diagnostics, completion, hover, definition, symbols, colors, and the
//! custom `slab/preview` render request). Synchronous JSON-RPC with
//! Content-Length framing over generic Read/Write; `slab lsp` wires it to
//! stdin/stdout, tests drive it in-memory.

pub mod index;
pub mod rpc;
pub mod server;
pub mod vocab;

pub use rpc::serve;
pub use server::Server;
