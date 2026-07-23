//! `slab-syntax` — lexer + parser for `.slab` sources (SPEC §2), producing a
//! spanned raw AST plus §12 diagnostics. Semantic resolution lives in
//! `slab-compile`.

pub mod ast;
pub mod diag;
pub mod fmt;
pub mod lex;
pub mod parse;

pub use ast::Document;
pub use diag::{Diag, Diagnostics, Level};
pub use fmt::format;
pub use parse::parse;
