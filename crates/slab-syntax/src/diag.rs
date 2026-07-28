//! Diagnostics accumulator shared by the slab front end and compiler (SPEC §12).

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
    Note,
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Level::Error => "error",
            Level::Warning => "warning",
            Level::Note => "note",
        })
    }
}

/// One diagnostic; `code` values are enumerated in SPEC §12.
#[derive(Debug, Clone)]
pub struct Diag {
    pub level: Level,
    pub code: &'static str,
    pub msg: String,
    pub line: u32,
    /// Source file override for diagnostics from imported units.
    pub file: Option<String>,
    /// Optional remedy, printed indented under the main line.
    pub remedy: Option<String>,
}

impl Diag {
    /// `file:line: level[code]: message` (+ indented remedy lines).
    pub fn format(&self, file: &str) -> String {
        let file = self.file.as_deref().unwrap_or(file);
        let mut out = String::new();
        if !file.is_empty() {
            out.push_str(&format!("{}:{}: ", file, self.line));
        }
        out.push_str(&format!("{}[{}]: {}", self.level, self.code, self.msg));
        if let Some(r) = &self.remedy {
            for line in r.lines() {
                out.push_str("\n  ");
                out.push_str(line);
            }
        }
        out
    }
}

/// Accumulator passed through every pipeline stage.
#[derive(Debug, Default)]
pub struct Diagnostics(pub Vec<Diag>);

impl Diagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn error(&mut self, code: &'static str, msg: impl Into<String>, line: u32) {
        self.0.push(Diag {
            level: Level::Error,
            code,
            msg: msg.into(),
            line,
            file: None,
            remedy: None,
        });
    }

    pub fn error_with(
        &mut self,
        code: &'static str,
        msg: impl Into<String>,
        line: u32,
        remedy: impl Into<String>,
    ) {
        self.0.push(Diag {
            level: Level::Error,
            code,
            msg: msg.into(),
            line,
            file: None,
            remedy: Some(remedy.into()),
        });
    }

    pub fn warn(&mut self, code: &'static str, msg: impl Into<String>, line: u32) {
        self.0.push(Diag {
            level: Level::Warning,
            code,
            msg: msg.into(),
            line,
            file: None,
            remedy: None,
        });
    }

    pub fn warn_with(
        &mut self,
        code: &'static str,
        msg: impl Into<String>,
        line: u32,
        remedy: impl Into<String>,
    ) {
        self.0.push(Diag {
            level: Level::Warning,
            code,
            msg: msg.into(),
            line,
            file: None,
            remedy: Some(remedy.into()),
        });
    }

    pub fn note(&mut self, code: &'static str, msg: impl Into<String>, line: u32) {
        self.0.push(Diag {
            level: Level::Note,
            code,
            msg: msg.into(),
            line,
            file: None,
            remedy: None,
        });
    }

    pub fn has_errors(&self) -> bool {
        self.0.iter().any(|d| d.level == Level::Error)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_file_overrides_the_format_fallback() {
        let mut diagnostics = Diagnostics::new();
        diagnostics.error("parse", "invalid source", 7);
        let diagnostic = &mut diagnostics.0[0];

        assert!(diagnostic.file.is_none());
        assert_eq!(
            diagnostic.format("root.slab"),
            "root.slab:7: error[parse]: invalid source"
        );

        diagnostic.file = Some("modules/panel.slab".into());
        assert_eq!(
            diagnostic.format("root.slab"),
            "modules/panel.slab:7: error[parse]: invalid source"
        );
    }
}
