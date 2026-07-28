//! Canonical formatter for `.slab` sources (`slab fmt`, LSP formatting).
//!
//! Line-preserving: statements are never merged or split. Normalizes
//! indentation (2 spaces per block depth), intra-line spacing (`k=v` tight,
//! `, ` inside parens/brackets, tight value tuples, spaced `=` in `params`
//! declarations), collapses blank-line runs, and aligns entry names inside
//! `tokens`/`theme`/`params`/`anim` blocks. Comments and string literals are
//! preserved verbatim.
//!
//! Safety: the result must lex to the same token stream as the input
//! (modulo newline runs); on any mismatch the source is returned unchanged.

use crate::diag::Diagnostics;
use crate::lex::{TokKind, lex};

/// Formatter token kind; comments are tokens here, unlike [`crate::lex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum K {
    Word,
    Str,
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
    Semi,
    LineComment,
    BlockComment,
}

impl K {
    fn is_closer(self) -> bool {
        matches!(self, K::Rb | K::Rp | K::Rs)
    }
}

/// Block context: decides `=` spacing and column alignment for child lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ctx {
    Node,
    Tokens,
    Params,
    Anim,
}

/// One rendered token plus the separator that precedes it.
struct Part {
    sep: &'static str,
    text: String,
    k: K,
}

enum Line {
    Blank,
    Text {
        indent: usize,
        parts: Vec<Part>,
        ctx: Ctx,
        /// Line ends with a `\` continuation.
        cont: bool,
    },
}

/// Reformat `src` into canonical style; returns `src` unchanged when the
/// result would not lex to an equivalent token stream (defensive: comments
/// and strings aside, formatting must never change meaning).
pub fn format(src: &str) -> String {
    let out = render(assemble(scan(src)));
    if lex_equiv(src, &out) {
        out
    } else {
        src.to_string()
    }
}

// ------------------------------------------------------------------ scanner

/// Raw scan token: formatter kinds plus line breaks.
enum S {
    Tok(K, String),
    Nl,
    /// `\`-newline continuation.
    Cont,
}

/// Scan `src` into formatter tokens, keeping comments and raw string slices.
fn scan(src: &str) -> Vec<S> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b' ' | b'\t' | b'\r' => i += 1,
            b'\n' => {
                out.push(S::Nl);
                i += 1;
            }
            b'\\'
                if matches!(b.get(i + 1), Some(b'\n'))
                    || (matches!(b.get(i + 1), Some(b'\r'))
                        && matches!(b.get(i + 2), Some(b'\n'))) =>
            {
                out.push(S::Cont);
                i += if b[i + 1] == b'\r' { 3 } else { 2 };
            }
            b'"' => {
                let start = i;
                i += 1;
                while i < b.len() {
                    match b[i] {
                        b'\\' => i = (i + 2).min(b.len()),
                        b'"' => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
                out.push(S::Tok(K::Str, src[start..i].to_string()));
            }
            b'/' if matches!(b.get(i + 1), Some(b'/')) => {
                let end = src[i..].find('\n').map_or(b.len(), |n| i + n);
                out.push(S::Tok(K::LineComment, src[i..end].trim_end().to_string()));
                i = end;
            }
            b'/' if matches!(b.get(i + 1), Some(b'*')) => {
                let end = src[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2);
                out.push(S::Tok(K::BlockComment, src[i..end].to_string()));
                i = end;
            }
            b'<' | b'>' => {
                let two = matches!(b.get(i + 1), Some(b'='));
                let end = i + if two { 2 } else { 1 };
                out.push(S::Tok(K::Cmp, src[i..end].to_string()));
                i = end;
            }
            _ => {
                let one = |k: K| S::Tok(k, (c as char).to_string());
                let tok = match c {
                    b'{' => Some(one(K::Lb)),
                    b'}' => Some(one(K::Rb)),
                    b'(' => Some(one(K::Lp)),
                    b')' => Some(one(K::Rp)),
                    b'[' => Some(one(K::Ls)),
                    b']' => Some(one(K::Rs)),
                    b'=' => Some(one(K::Eq)),
                    b',' => Some(one(K::Comma)),
                    b':' => Some(one(K::Colon)),
                    b'!' => Some(one(K::Bang)),
                    b';' => Some(one(K::Semi)),
                    _ => None,
                };
                if let Some(t) = tok {
                    out.push(t);
                    i += 1;
                } else {
                    // word run: everything up to whitespace or a special char
                    let start = i;
                    while i < b.len() && !word_break(b, i) {
                        i += 1;
                    }
                    i = i.max(start + 1); // lone special-ish byte, keep verbatim
                    out.push(S::Tok(K::Word, src[start..i].to_string()));
                }
            }
        }
    }
    out
}

/// True when the byte at `i` terminates a word run.
fn word_break(b: &[u8], i: usize) -> bool {
    match b[i] {
        b' ' | b'\t' | b'\r' | b'\n' | b'"' | b'{' | b'}' | b'(' | b')' | b'[' | b']' | b'='
        | b',' | b':' | b'!' | b';' | b'<' | b'>' | b'\\' => true,
        b'/' => matches!(b.get(i + 1), Some(b'/') | Some(b'*')),
        _ => false,
    }
}

// ----------------------------------------------------------------- assembly

/// Group scan tokens into lines with computed indent, context, and spacing.
fn assemble(toks: Vec<S>) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut cur: Vec<Part> = Vec::new();
    let mut depth = 0usize; // {, (, [ nesting for indentation
    let mut paren = 0usize; // (, [ nesting for comma spacing
    let mut ctxs = vec![Ctx::Node];
    let mut line_depth = 0usize; // depth at start of current line
    let mut line_ctx = Ctx::Node;
    let mut first_word: Option<String> = None; // of the logical line
    // continuation indent: set on the first line of a `\` group
    let mut cont_base: Option<usize> = None;

    let flush = |cur: &mut Vec<Part>,
                 lines: &mut Vec<Line>,
                 line_depth: usize,
                 line_ctx: Ctx,
                 cont: bool,
                 cont_base: &mut Option<usize>| {
        // drop a trailing `;` (a `;` right before the newline is redundant)
        if !cont && cur.last().is_some_and(|p| p.k == K::Semi) {
            cur.pop();
        }
        if cur.is_empty() {
            lines.push(Line::Blank);
            if !cont {
                *cont_base = None;
            }
            return;
        }
        let leading = cur.iter().take_while(|p| p.k.is_closer()).count();
        let indent = match *cont_base {
            Some(base) => base + 2,
            None => line_depth.saturating_sub(leading),
        };
        if cont && cont_base.is_none() {
            *cont_base = Some(indent);
        }
        if !cont {
            *cont_base = None;
        }
        lines.push(Line::Text {
            indent,
            parts: std::mem::take(cur),
            ctx: line_ctx,
            cont,
        });
    };

    for t in toks {
        match t {
            S::Nl | S::Cont => {
                let cont = matches!(t, S::Cont);
                flush(
                    &mut cur,
                    &mut lines,
                    line_depth,
                    line_ctx,
                    cont,
                    &mut cont_base,
                );
                if !cont {
                    first_word = None;
                }
                line_depth = depth;
                line_ctx = *ctxs.last().unwrap_or(&Ctx::Node);
            }
            S::Tok(k, text) => {
                if k == K::Word && first_word.is_none() {
                    first_word = Some(text.clone());
                }
                let sep = if cur.is_empty() {
                    ""
                } else {
                    sep(
                        cur.last().unwrap().k,
                        k,
                        paren,
                        *ctxs.last().unwrap_or(&Ctx::Node),
                    )
                };
                match k {
                    K::Lb => {
                        depth += 1;
                        ctxs.push(block_ctx(
                            first_word.as_deref(),
                            *ctxs.last().unwrap_or(&Ctx::Node),
                        ));
                    }
                    K::Rb => {
                        depth = depth.saturating_sub(1);
                        if ctxs.len() > 1 {
                            ctxs.pop();
                        }
                    }
                    K::Lp | K::Ls => {
                        depth += 1;
                        paren += 1;
                    }
                    K::Rp | K::Rs => {
                        depth = depth.saturating_sub(1);
                        paren = paren.saturating_sub(1);
                    }
                    _ => {}
                }
                cur.push(Part { sep, text, k });
            }
        }
    }
    flush(
        &mut cur,
        &mut lines,
        line_depth,
        line_ctx,
        false,
        &mut cont_base,
    );
    lines
}

/// Context a `{` opens, from the logical line's first word and parent block.
fn block_ctx(first_word: Option<&str>, parent: Ctx) -> Ctx {
    match first_word {
        Some("tokens") | Some("theme") => Ctx::Tokens,
        Some("params") => Ctx::Params,
        Some("anim") => Ctx::Anim,
        // nested token groups (`color { … }`) inherit the tokens context
        _ if parent == Ctx::Tokens => Ctx::Tokens,
        _ => Ctx::Node,
    }
}

/// Separator between adjacent tokens `a`, `b` at paren depth `p` in `ctx`.
fn sep(a: K, b: K, p: usize, ctx: Ctx) -> &'static str {
    use K::*;
    if a == Eq || b == Eq {
        // params declarations read `name type = default`; attrs stay tight
        return if ctx == Ctx::Params && p == 0 {
            " "
        } else {
            ""
        };
    }
    if a == Cmp || b == Cmp {
        return ""; // when w<600
    }
    if matches!(b, Comma | Semi | Rp | Rs | Colon) {
        return "";
    }
    if matches!(a, Lp | Ls | Bang | Colon) {
        return "";
    }
    if a == Comma {
        // argument lists breathe; value tuples (pad=6,12) stay tight
        return if p > 0 { " " } else { "" };
    }
    if b == Lp && a == Word {
        return ""; // calls: linear(…), list(Row), Tag(…)
    }
    " "
}

// ---------------------------------------------------------------- rendering

/// First-token width when the line participates in column alignment.
fn align_width(l: &Line) -> Option<usize> {
    let Line::Text {
        parts, ctx, cont, ..
    } = l
    else {
        return None;
    };
    if *cont || parts.len() < 2 || parts[0].k != K::Word {
        return None;
    }
    let eligible = match ctx {
        // simple entries (`bg #10141B`) and inline groups (`body { … }`);
        // multi-token headers of multi-line groups never form runs anyway
        Ctx::Tokens => parts[1].k == K::Lb || !parts.iter().any(|p| p.k == K::Lb),
        // declarations: `title text = "…"`
        Ctx::Params => !parts.iter().any(|p| p.k == K::Lb),
        // keyframes: `0% { … }` / `100% { … }`
        Ctx::Anim => parts[1].k == K::Lb,
        Ctx::Node => false,
    };
    eligible.then(|| parts[0].text.chars().count())
}

/// Render lines: trim/collapse blanks, pad aligned columns, emit text.
fn render(lines: Vec<Line>) -> String {
    // column widths per run of consecutive aligned siblings at equal indent
    let mut pad = vec![0usize; lines.len()];
    let mut run: Vec<(usize, usize)> = Vec::new(); // (line index, width)
    let mut run_indent = usize::MAX;
    let close_run = |run: &mut Vec<(usize, usize)>, pad: &mut Vec<usize>| {
        if run.len() > 1 {
            let max = run.iter().map(|&(_, w)| w).max().unwrap_or(0);
            for &(i, w) in run.iter() {
                pad[i] = max - w;
            }
        }
        run.clear();
    };
    for (i, l) in lines.iter().enumerate() {
        match align_width(l) {
            Some(w) => {
                let indent = match l {
                    Line::Text { indent, .. } => *indent,
                    Line::Blank => 0,
                };
                if indent != run_indent {
                    close_run(&mut run, &mut pad);
                    run_indent = indent;
                }
                run.push((i, w));
            }
            None => {
                close_run(&mut run, &mut pad);
                run_indent = usize::MAX;
            }
        }
    }
    close_run(&mut run, &mut pad);

    let mut out = String::new();
    let mut pending_blank = false;
    let mut any = false;
    for (i, l) in lines.iter().enumerate() {
        let Line::Text {
            indent,
            parts,
            cont,
            ..
        } = l
        else {
            pending_blank = any;
            continue;
        };
        if pending_blank {
            out.push('\n');
            pending_blank = false;
        }
        any = true;
        for _ in 0..*indent {
            out.push_str("  ");
        }
        for (j, p) in parts.iter().enumerate() {
            out.push_str(p.sep);
            if j == 1 {
                for _ in 0..pad[i] {
                    out.push(' ');
                }
            }
            out.push_str(&p.text);
        }
        if *cont {
            out.push_str(" \\");
        }
        out.push('\n');
    }
    out
}

// ------------------------------------------------------------- verification

/// True when both sources lex to the same token stream, ignoring line
/// numbers and collapsing newline runs (blank-line edits are legal).
fn lex_equiv(a: &str, b: &str) -> bool {
    fn stream(src: &str) -> Vec<(TokKind, String)> {
        let mut diags = Diagnostics::new();
        let mut out: Vec<(TokKind, String)> = Vec::new();
        for t in lex(src, &mut diags) {
            if t.kind == TokKind::Nl && out.last().is_some_and(|(k, _)| *k == TokKind::Nl) {
                continue;
            }
            out.push((t.kind, t.text));
        }
        while out.first().is_some_and(|(k, _)| *k == TokKind::Nl) {
            out.remove(0);
        }
        if out.len() >= 2 && out[out.len() - 2].0 == TokKind::Nl {
            out.remove(out.len() - 2); // ignore presence of a final newline
        }
        out
    }
    stream(a) == stream(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_spacing_and_indent() {
        let src =
            "col   w = fill  pad=24 {\n      text  \"hi\"   size=12\n   when hover{bg = #fff}\n}\n";
        assert_eq!(
            format(src),
            "col w=fill pad=24 {\n  text \"hi\" size=12\n  when hover { bg=#fff }\n}\n"
        );
    }

    #[test]
    fn aligns_token_entries_and_params() {
        let src = "tokens {\n color {\n bg #10141B\n accent #4FC7E0\n }\n}\n\nparams {\n title text = \"Settings\"\n compact bool = false\n}\n";
        assert_eq!(
            format(src),
            "tokens {\n  color {\n    bg     #10141B\n    accent #4FC7E0\n  }\n}\n\nparams {\n  title   text = \"Settings\"\n  compact bool = false\n}\n"
        );
    }

    #[test]
    fn aligns_anim_keyframes() {
        let src = "anim spin {\n0% {rotate=0}\n100% {rotate=360}\n}\n";
        assert_eq!(
            format(src),
            "anim spin {\n  0%   { rotate=0 }\n  100% { rotate=360 }\n}\n"
        );
    }

    #[test]
    fn commas_tight_in_tuples_spaced_in_calls() {
        let src = "rect pad=6 , 12 bg=linear(90 ,#fff 0% ,#000 100%)\n";
        assert_eq!(
            format(src),
            "rect pad=6,12 bg=linear(90, #fff 0%, #000 100%)\n"
        );
    }

    #[test]
    fn key_maps_keep_tight_pair_separators() {
        let src = "col keys=Escape : clear , F2 : rename\n";
        assert_eq!(format(src), "col keys=Escape:clear,F2:rename\n");
    }

    #[test]
    fn preserves_comments_continuations_and_strings() {
        let src = "// header\ncol w=360 \\\nbg=#fff { // trailing\n  text \"a  b\\n\" /* keep */ size=12\n}\n";
        assert_eq!(
            format(src),
            "// header\ncol w=360 \\\n    bg=#fff { // trailing\n  text \"a  b\\n\" /* keep */ size=12\n}\n"
        );
    }

    #[test]
    fn collapses_blank_runs_and_trims_edges() {
        let src = "\n\ncol {\n\n\n  text \"x\"\n}\n\n\n";
        assert_eq!(format(src), "col {\n\n  text \"x\"\n}\n");
    }

    #[test]
    fn multiline_list_defaults_indent_and_space() {
        let src =
            "params {\ntracks list(Row)=[\nRow(title=\"a\",tone=#fff),\nRow(title=\"b\")\n]\n}\n";
        assert_eq!(
            format(src),
            "params {\n  tracks list(Row) = [\n    Row(title=\"a\", tone=#fff),\n    Row(title=\"b\")\n  ]\n}\n"
        );
    }

    #[test]
    fn conds_and_bangs_stay_tight() {
        let src =
            "col {\nwhen w < 600 { pad=12 }\nwhen ! hover { opacity=0.5 }\nrect w=fill:2\n}\n";
        assert_eq!(
            format(src),
            "col {\n  when w<600 { pad=12 }\n  when !hover { opacity=0.5 }\n  rect w=fill:2\n}\n"
        );
    }

    #[test]
    fn strike_keeps_bare_and_boolean_forms() {
        let src = "col{\ntext \"done\" strike\ntext \"open\" strike = false\n}\n";
        assert_eq!(
            format(src),
            "col {\n  text \"done\" strike\n  text \"open\" strike=false\n}\n"
        );
    }

    #[test]
    fn idempotent() {
        let src = "tokens {\n color {\n bg #101 \n }\n}\ncol w=fill {\n text \"hi\"\n}\n";
        let once = format(src);
        assert_eq!(format(&once), once);
    }

    #[test]
    fn unterminated_string_returns_source_unchanged() {
        let src = "text \"oops\n";
        assert_eq!(format(src), src);
    }
}

#[cfg(test)]
mod corpus {
    use super::format;
    use std::path::PathBuf;

    /// Every checked-in `.slab` stays formatted, and formatting is a fixpoint.
    #[test]
    fn examples_and_cases_are_canonical_and_idempotent() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut seen = 0;
        for dir in ["examples", "conformance/cases"] {
            let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
                continue;
            };
            for e in entries.flatten() {
                let path = e.path();
                if path.extension().is_none_or(|x| x != "slab") {
                    continue;
                }
                let src = std::fs::read_to_string(&path).unwrap();
                let once = format(&src);
                assert_eq!(once, src, "not canonically formatted: {}", path.display());
                assert_eq!(format(&once), once, "not idempotent: {}", path.display());
                seen += 1;
            }
        }
        assert!(seen > 50, "corpus missing? saw {seen} files");
    }
}
