//! Logos lexer for the slab syntax (SPEC §2), a port of the 0.5 reference
//! lexer: newline-sensitive, dotted refs are single tokens, `%` folds into
//! the number token, `-` continues an ident only before a letter.

use crate::diag::Diagnostics;
use logos::{Lexer, Logos, Skip};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokKind {
    Nl,
    Str,
    Hash,
    Num,
    Pct,
    Id,
    Ref,
    Cmp,
    Lb,
    Rb,
    Lp,
    Rp,
    Ls,
    Rs,
    Eq,
    Comma,
    Colon,
    Bang,
    Eof,
}

impl TokKind {
    /// Diagnostic name, matching the 0.5 reference messages.
    pub fn name(self) -> &'static str {
        match self {
            TokKind::Nl => "NL",
            TokKind::Str => "STR",
            TokKind::Hash => "HASH",
            TokKind::Num => "NUM",
            TokKind::Pct => "PCT",
            TokKind::Id => "ID",
            TokKind::Ref => "REF",
            TokKind::Cmp => "CMP",
            TokKind::Lb => "LB",
            TokKind::Rb => "RB",
            TokKind::Lp => "LP",
            TokKind::Rp => "RP",
            TokKind::Ls => "LS",
            TokKind::Rs => "RS",
            TokKind::Eq => "EQ",
            TokKind::Comma => "COMMA",
            TokKind::Colon => "COLON",
            TokKind::Bang => "BANG",
            TokKind::Eof => "EOF",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Tok {
    pub kind: TokKind,
    pub text: String,
    pub line: u32,
    pub val: f64,
}

#[derive(Default)]
pub struct LexExtras {
    /// (byte offset, message) for errors detected inside callbacks.
    errors: Vec<(usize, &'static str)>,
}

fn lex_string(lex: &mut Lexer<'_, RawTok>) -> String {
    let rem = lex.remainder();
    let mut buf = String::new();
    let mut it = rem.char_indices();
    let mut consumed = rem.len();
    let mut terminated = false;
    while let Some((i, c)) = it.next() {
        if c == '"' {
            consumed = i + c.len_utf8();
            terminated = true;
            break;
        }
        if c == '\\' {
            match it.next() {
                Some((_, e)) => buf.push(match e {
                    'n' => '\n',
                    't' => '\t',
                    '"' => '"',
                    '\\' => '\\',
                    '_' => '\u{a0}',
                    other => other,
                }),
                None => break,
            }
        } else {
            buf.push(c);
        }
    }
    lex.bump(consumed);
    if !terminated {
        lex.extras
            .errors
            .push((lex.span().end, "unterminated string"));
    }
    buf
}

fn skip_block_comment(lex: &mut Lexer<'_, RawTok>) -> Skip {
    let rem = lex.remainder();
    match rem.find("*/") {
        Some(i) => lex.bump(i + 2),
        None => lex.bump(rem.len()),
    }
    Skip
}

fn num_val(lex: &Lexer<'_, RawTok>) -> f64 {
    let s = lex.slice().trim_end_matches('%');
    s.parse().unwrap_or(0.0)
}

#[derive(Logos, Debug, PartialEq)]
#[logos(extras = LexExtras)]
enum RawTok {
    #[regex(r"[ \t\r]+", logos::skip)]
    #[regex(r"//[^\n]*", logos::skip, allow_greedy = true)]
    #[regex(r"\\\r?\n", logos::skip)]
    #[token("/*", skip_block_comment)]
    #[token("\n")]
    #[token(";")]
    Nl,

    #[token("\"", lex_string)]
    Str(String),

    #[regex(r"#[A-Za-z0-9_-]*", |lex| lex.slice()[1..].to_string())]
    Hash(String),

    #[regex(r"-?([0-9]+\.?[0-9]*|\.[0-9]+)%", num_val)]
    Pct(f64),

    #[regex(r"-?([0-9]+\.?[0-9]*|\.[0-9]+)", num_val)]
    Num(f64),

    #[regex(
        r"[A-Za-z_][A-Za-z0-9_]*(-[A-Za-z_][A-Za-z0-9_]*)*(\.[A-Za-z_][A-Za-z0-9_]*(-[A-Za-z_][A-Za-z0-9_]*)*)+",
        |lex| lex.slice().to_string()
    )]
    Ref(String),

    #[regex(r"[A-Za-z_][A-Za-z0-9_]*(-[A-Za-z_][A-Za-z0-9_]*)*", |lex| lex.slice().to_string())]
    Id(String),

    #[regex(r"<=|>=|<|>", |lex| lex.slice().to_string())]
    Cmp(String),

    #[token("{")]
    Lb,
    #[token("}")]
    Rb,
    #[token("(")]
    Lp,
    #[token(")")]
    Rp,
    #[token("[")]
    Ls,
    #[token("]")]
    Rs,
    #[token("=")]
    Eq,
    #[token(",")]
    Comma,
    #[token(":")]
    Colon,
    #[token("!")]
    Bang,
}

/// Byte offset -> 1-based line number lookup.
pub struct LineMap {
    starts: Vec<usize>,
}

impl LineMap {
    pub fn new(src: &str) -> Self {
        let mut starts = vec![0usize];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Self { starts }
    }

    pub fn line(&self, offset: usize) -> u32 {
        self.starts.partition_point(|&s| s <= offset) as u32
    }
}

/// Lex `src` into the reference token stream (ends with EOF).
pub fn lex(src: &str, diags: &mut Diagnostics) -> Vec<Tok> {
    let map = LineMap::new(src);
    let mut toks = Vec::new();
    let mut lexer = RawTok::lexer(src);
    while let Some(res) = lexer.next() {
        let span = lexer.span();
        let line = map.line(span.start);
        let tok = match res {
            Ok(raw) => match raw {
                RawTok::Nl => Tok {
                    kind: TokKind::Nl,
                    text: "\n".into(),
                    line,
                    val: 0.0,
                },
                RawTok::Str(s) => Tok {
                    kind: TokKind::Str,
                    text: s,
                    line,
                    val: 0.0,
                },
                RawTok::Hash(s) => Tok {
                    kind: TokKind::Hash,
                    text: s,
                    line,
                    val: 0.0,
                },
                RawTok::Pct(v) => Tok {
                    kind: TokKind::Pct,
                    text: lexer.slice().to_string(),
                    line,
                    val: v,
                },
                RawTok::Num(v) => Tok {
                    kind: TokKind::Num,
                    text: lexer.slice().to_string(),
                    line,
                    val: v,
                },
                RawTok::Ref(s) => Tok {
                    kind: TokKind::Ref,
                    text: s,
                    line,
                    val: 0.0,
                },
                RawTok::Id(s) => Tok {
                    kind: TokKind::Id,
                    text: s,
                    line,
                    val: 0.0,
                },
                RawTok::Cmp(s) => Tok {
                    kind: TokKind::Cmp,
                    text: s,
                    line,
                    val: 0.0,
                },
                RawTok::Lb => simple(TokKind::Lb, "{", line),
                RawTok::Rb => simple(TokKind::Rb, "}", line),
                RawTok::Lp => simple(TokKind::Lp, "(", line),
                RawTok::Rp => simple(TokKind::Rp, ")", line),
                RawTok::Ls => simple(TokKind::Ls, "[", line),
                RawTok::Rs => simple(TokKind::Rs, "]", line),
                RawTok::Eq => simple(TokKind::Eq, "=", line),
                RawTok::Comma => simple(TokKind::Comma, ",", line),
                RawTok::Colon => simple(TokKind::Colon, ":", line),
                RawTok::Bang => simple(TokKind::Bang, "!", line),
            },
            Err(()) => {
                let ch = src[span.start..].chars().next().unwrap_or('\u{fffd}');
                diags.error(
                    "parse",
                    format!("unexpected character {}", py_repr_char(ch)),
                    line,
                );
                continue;
            }
        };
        toks.push(tok);
    }
    for (offset, msg) in std::mem::take(&mut lexer.extras.errors) {
        diags.error("parse", msg, map.line(offset));
    }
    toks.push(Tok {
        kind: TokKind::Eof,
        text: String::new(),
        line: map.line(src.len()),
        val: 0.0,
    });
    toks
}

fn simple(kind: TokKind, text: &str, line: u32) -> Tok {
    Tok {
        kind,
        text: text.into(),
        line,
        val: 0.0,
    }
}

/// Python-style single-quoted repr, used in parser diagnostics.
pub fn py_repr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

fn py_repr_char(c: char) -> String {
    py_repr(&c.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexes_list_brackets() {
        let mut diags = Diagnostics::default();
        let toks = lex("[]", &mut diags);
        assert!(diags.0.is_empty());
        assert_eq!(
            toks.iter().map(|tok| tok.kind).collect::<Vec<_>>(),
            vec![TokKind::Ls, TokKind::Rs, TokKind::Eof]
        );
    }
}
