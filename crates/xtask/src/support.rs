//! `xtask support-md` / `xtask gen-caps` — the platform support chart.
//!
//! `spec/support.toml` is the single source of truth: one `[feature.NAME]`
//! table per chart row, one key per client column (`web gpu tui svg png`,
//! matching kernel client codes 0..4), values
//! `"full" | "degraded:<sentence>" | "none:<cap-* code>"`.
//!
//! - `support-md` renders the chart as a markdown table into `spec/SPEC.md`
//!   between `<!-- support-chart:begin -->` / `<!-- support-chart:end -->`
//!   (inserting a `## Platform support` section before the changelog when
//!   the markers are absent). Idempotent.
//! - `gen-caps` emits `crates/slab-kernel/src/caps.rs` (per-client feature
//!   levels as u8 tables plus note/diagnostic-code strings).

use std::path::Path;
use std::process::ExitCode;

/// Client columns; index = the kernel client code (`inst_set_env`).
const CLIENTS: [&str; 5] = ["web", "gpu", "tui", "svg", "png"];

const MARK_BEGIN: &str = "<!-- support-chart:begin -->";
const MARK_END: &str = "<!-- support-chart:end -->";

/// One chart cell.
enum Level {
    Full,
    /// Degradation note (one concrete sentence).
    Degraded(String),
    /// `cap-*` diagnostic code reported once per document.
    None(String),
}

struct Feature {
    name: String,
    /// One cell per CLIENTS column.
    cells: Vec<Level>,
}

/// Parse the strict support.toml subset; hard error on anything unexpected
/// so chart drift can never be silent.
fn parse_support(text: &str) -> Result<Vec<Feature>, String> {
    let mut feats: Vec<Feature> = Vec::new();
    for (ln, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let err = |m: &str| format!("support.toml:{}: {m}", ln + 1);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix("[feature.")
            .and_then(|s| s.strip_suffix(']'))
        {
            if name.is_empty() {
                return Err(err("empty feature name"));
            }
            if feats.iter().any(|f| f.name == name) {
                return Err(err(&format!("duplicate feature '{name}'")));
            }
            feats.push(Feature {
                name: name.to_string(),
                cells: Vec::new(),
            });
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            return Err(err("expected `[feature.NAME]` or `client = \"value\"`"));
        };
        let key = key.trim();
        let col = CLIENTS
            .iter()
            .position(|c| *c == key)
            .ok_or_else(|| err(&format!("unknown client column '{key}'")))?;
        let val = val.trim();
        let val = val
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .ok_or_else(|| err("value must be a double-quoted string"))?;
        if val.contains(['"', '\\', '|']) {
            return Err(err("value may not contain '\"', '\\', or '|'"));
        }
        let feat = feats
            .last_mut()
            .ok_or_else(|| err("client value before any [feature.*] header"))?;
        if feat.cells.len() != col {
            return Err(err(&format!(
                "'{key}' out of order in feature '{}' (columns must be {})",
                feat.name,
                CLIENTS.join(" ")
            )));
        }
        let cell = if val == "full" {
            Level::Full
        } else if let Some(note) = val.strip_prefix("degraded:") {
            if note.trim().is_empty() {
                return Err(err("degraded needs a note sentence"));
            }
            Level::Degraded(note.to_string())
        } else if let Some(code) = val.strip_prefix("none:") {
            if !code.starts_with("cap-") || code.len() <= 4 {
                return Err(err("none needs a cap-* diag code"));
            }
            Level::None(code.to_string())
        } else {
            return Err(err(&format!(
                "bad value '{val}' (full | degraded:<note> | none:<cap-code>)"
            )));
        };
        feat.cells.push(cell);
    }
    if feats.is_empty() {
        return Err("support.toml: no features".into());
    }
    for f in &feats {
        if f.cells.len() != CLIENTS.len() {
            return Err(format!(
                "support.toml: feature '{}' has {} of {} client columns",
                f.name,
                f.cells.len(),
                CLIENTS.len()
            ));
        }
    }
    Ok(feats)
}

fn load() -> Result<Vec<Feature>, String> {
    let path = super::repo_root().join("spec/support.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_support(&text)
}

// ------------------------------------------------------------- support-md

/// The chart between the markers (markers included).
fn chart_md(feats: &[Feature]) -> String {
    let mut md = String::new();
    md.push_str(MARK_BEGIN);
    md.push('\n');
    md.push_str(
        "<!-- GENERATED from spec/support.toml by `cargo run -p xtask -- support-md`; edit the toml, not this table. -->\n\n",
    );
    md.push_str("| feature |");
    for c in CLIENTS {
        md.push_str(&format!(" {c} |"));
    }
    md.push_str("\n| --- |");
    for _ in CLIENTS {
        md.push_str(" --- |");
    }
    md.push('\n');
    for f in feats {
        md.push_str(&format!("| {} |", f.name));
        for cell in &f.cells {
            let text = match cell {
                Level::Full => "full".to_string(),
                Level::Degraded(note) => format!("degraded — {note}"),
                Level::None(code) => format!("none (`{code}`)"),
            };
            md.push_str(&format!(" {text} |"));
        }
        md.push('\n');
    }
    md.push('\n');
    md.push_str(MARK_END);
    md
}

/// Section prose written once, when the markers are first inserted.
fn section_md(feats: &[Feature]) -> String {
    format!(
        "## Platform support\n\n\
         One row per feature, one column per renderer client (§11's `when`\n\
         client classes; the column order is the kernel client code). **full**\n\
         = renders as specified. **degraded** = renders with the documented\n\
         approximation; the note is the whole contract. **none** = the\n\
         renderer skips the feature and reports the `cap-*` code once per\n\
         document (§12 notes; TUI notes accumulate on the cell grid, static\n\
         exporters print to stderr). Machine-readable source:\n\
         `spec/support.toml`; the driver lookup table is generated into\n\
         `crates/slab-kernel/src/caps.rs` by `cargo run -p xtask -- gen-caps`.\n\n{}\n",
        chart_md(feats)
    )
}

pub fn cmd_support_md() -> ExitCode {
    let feats = match load() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let spec_path = super::repo_root().join("spec/SPEC.md");
    let spec = match std::fs::read_to_string(&spec_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}: {e}", spec_path.display());
            return ExitCode::from(2);
        }
    };
    let next = match (spec.find(MARK_BEGIN), spec.find(MARK_END)) {
        (Some(b), Some(e)) if e > b => {
            let mut s = String::with_capacity(spec.len());
            s.push_str(&spec[..b]);
            s.push_str(&chart_md(&feats));
            s.push_str(&spec[e + MARK_END.len()..]);
            s
        }
        (None, None) => {
            // first run: insert the whole section before the changelog,
            // else append at the end
            let section = section_md(&feats);
            match spec.find("\n## 17. Changelog") {
                Some(at) => {
                    let mut s = String::with_capacity(spec.len() + section.len());
                    s.push_str(&spec[..at + 1]);
                    s.push_str(&section);
                    s.push('\n');
                    s.push_str(&spec[at + 1..]);
                    s
                }
                None => {
                    let mut s = spec.clone();
                    if !s.ends_with('\n') {
                        s.push('\n');
                    }
                    s.push('\n');
                    s.push_str(&section);
                    s
                }
            }
        }
        _ => {
            eprintln!("error: spec/SPEC.md has mismatched support-chart markers");
            return ExitCode::from(2);
        }
    };
    if next != spec {
        if let Err(e) = std::fs::write(&spec_path, &next) {
            eprintln!("error: {}: {e}", spec_path.display());
            return ExitCode::from(2);
        }
        eprintln!("support-md: updated {}", spec_path.display());
    } else {
        eprintln!("support-md: {} up to date", spec_path.display());
    }
    ExitCode::SUCCESS
}

// --------------------------------------------------------------- gen-caps

fn caps_rs(feats: &[Feature]) -> String {
    let mut s = String::new();
    s.push_str(
        "// GENERATED by `cargo run -p xtask -- gen-caps` from spec/support.toml — do not edit\n",
    );
    s.push_str("//! Platform × feature capability tables (spec/SPEC.md §Platform support).\n");
    s.push_str("//! Drivers look degradations up here instead of hardcoding them.\n\n");
    s.push_str("/// Feature levels.\npub const NONE: u8 = 0;\npub const DEGRADED: u8 = 1;\npub const FULL: u8 = 2;\n\n");
    s.push_str("/// Client columns; the index is the kernel client code (`inst_set_env`).\n");
    s.push_str(&format!(
        "pub const CLIENTS: [&str; {}] = [{}];\n\n",
        CLIENTS.len(),
        CLIENTS
            .iter()
            .map(|c| format!("{c:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    s.push_str("/// Feature rows, chart order.\n");
    s.push_str(&format!(
        "pub const FEATURES: [&str; {}] = [\n{}];\n\n",
        feats.len(),
        feats
            .iter()
            .map(|f| format!("    {:?},\n", f.name))
            .collect::<String>()
    ));
    s.push_str("/// `LEVELS[feature][client]`.\n");
    s.push_str(&format!(
        "pub const LEVELS: [[u8; {}]; {}] = [\n",
        CLIENTS.len(),
        feats.len()
    ));
    for f in feats {
        let row: Vec<&str> = f
            .cells
            .iter()
            .map(|c| match c {
                Level::Full => "FULL",
                Level::Degraded(_) => "DEGRADED",
                Level::None(_) => "NONE",
            })
            .collect();
        s.push_str(&format!("    [{}], // {}\n", row.join(", "), f.name));
    }
    s.push_str("];\n\n");
    s.push_str("/// `NOTES[feature][client]`: `\"\"` for full, the degradation sentence\n/// for degraded, the `cap-*` diag code for none.\n");
    s.push_str(&format!(
        "pub const NOTES: [[&str; {}]; {}] = [\n",
        CLIENTS.len(),
        feats.len()
    ));
    for f in feats {
        s.push_str(&format!("    // {}\n    [\n", f.name));
        for c in &f.cells {
            let txt = match c {
                Level::Full => "",
                Level::Degraded(n) => n.as_str(),
                Level::None(code) => code.as_str(),
            };
            s.push_str(&format!("        {txt:?},\n"));
        }
        s.push_str("    ],\n");
    }
    s.push_str("];\n\n");
    s.push_str(
        "/// Level of `feature` on the client with kernel code `client`;\n\
         /// unknown names/codes count as FULL (nothing to report).\n\
         pub fn level(feature: &str, client: u32) -> u8 {\n\
         \x20   let Some(f) = FEATURES.iter().position(|n| *n == feature) else {\n\
         \x20       return FULL;\n\
         \x20   };\n\
         \x20   let Some(row) = LEVELS.get(f) else { return FULL };\n\
         \x20   *row.get(client as usize).unwrap_or(&FULL)\n\
         }\n\n\
         /// Note/diag-code of `feature` on `client` (`\"\"` when full/unknown).\n\
         pub fn note(feature: &str, client: u32) -> &'static str {\n\
         \x20   let Some(f) = FEATURES.iter().position(|n| *n == feature) else {\n\
         \x20       return \"\";\n\
         \x20   };\n\
         \x20   let Some(row) = NOTES.get(f) else { return \"\" };\n\
         \x20   row.get(client as usize).copied().unwrap_or(\"\")\n\
         }\n",
    );
    s
}

pub fn cmd_gen_caps() -> ExitCode {
    let feats = match load() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let root = super::repo_root();
    let rs_path = root.join("crates/slab-kernel/src/caps.rs");
    if let Err(e) = std::fs::write(&rs_path, caps_rs(&feats)) {
        eprintln!("error: {}: {e}", rs_path.display());
        return ExitCode::from(2);
    }
    eprintln!("gen-caps: wrote {}", rs_path.display());
    rustfmt(&rs_path);
    ExitCode::SUCCESS
}

/// Best-effort formatting: the file is valid without rustfmt, while formatting
/// keeps `cargo fmt --check` green on the committed tree.
fn rustfmt(path: &Path) {
    match std::process::Command::new("rustfmt")
        .arg("--edition")
        .arg("2024")
        .arg(path)
        .status()
    {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("gen-caps: rustfmt exited with {s}; left unformatted"),
        Err(_) => eprintln!("gen-caps: rustfmt not found; left unformatted"),
    }
}
