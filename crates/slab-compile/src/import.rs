//! Compile-time loading for Slab import closures.

use crate::{Options, expand::MAX_DEPTH};
use slab_syntax::{
    ast::{AImport, Document},
    diag::Diagnostics,
};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

/// One parsed source file in document assembly order.
#[derive(Debug, Clone)]
pub struct Unit {
    /// Normalized source key, or `None` for the root document.
    pub file: Option<String>,
    /// Absolute source path for filesystem-backed imported modules.
    pub abs: Option<PathBuf>,
    /// Parsed source document.
    pub doc: Document,
}

struct Loader<'a> {
    opts: &'a Options,
    diags: &'a mut Diagnostics,
    units: Vec<Unit>,
    visited: HashSet<String>,
    stack: Vec<String>,
}

impl Loader<'_> {
    fn error(&mut self, code: &'static str, message: String, line: u32, file: Option<&str>) {
        self.diags.error(code, message, line);
        if let Some(file) = file
            && let Some(diagnostic) = self.diags.0.last_mut()
        {
            diagnostic.file = Some(file.to_string());
        }
    }

    fn load(&mut self, importer: Option<&str>, import: &AImport) {
        let key = normalize(importer, &import.path);
        if let Some(start) = self.stack.iter().position(|candidate| candidate == &key) {
            let mut chain = self.stack[start..].to_vec();
            chain.push(key);
            self.error(
                "import-cycle",
                format!("import cycle: {}", chain.join(" -> ")),
                import.line,
                importer,
            );
            return;
        }
        if self.visited.contains(&key) {
            return;
        }
        if self.stack.len() >= MAX_DEPTH {
            let mut chain = self.stack.clone();
            chain.push(key);
            self.error(
                "import-depth",
                format!("import nesting exceeds {MAX_DEPTH}: {}", chain.join(" -> ")),
                import.line,
                importer,
            );
            return;
        }
        self.visited.insert(key.clone());

        let (source, abs) = match self.read(&key) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.error(
                    "import-io",
                    format!("could not import '{key}': {error}"),
                    import.line,
                    importer,
                );
                return;
            }
        };
        let mut local = Diagnostics::new();
        let document = slab_syntax::parse(&source, &mut local);
        for mut diagnostic in local.0 {
            diagnostic.file = Some(key.clone());
            self.diags.0.push(diagnostic);
        }

        self.stack.push(key.clone());
        for child in &document.imports {
            self.load(Some(&key), child);
        }
        self.stack.pop();

        for root in &document.roots {
            self.error(
                "import-content",
                "imported files may not contain root content nodes".into(),
                root.line,
                Some(&key),
            );
        }
        self.units.push(Unit {
            file: Some(key),
            abs,
            doc: document,
        });
    }

    fn read(&self, key: &str) -> Result<(String, Option<PathBuf>), String> {
        if let Some(sources) = &self.opts.sources {
            return sources
                .get(key)
                .cloned()
                .map(|source| (source, None))
                .ok_or_else(|| "source is not available in the virtual source map".into());
        }

        let path = self.opts.base_dir.join(key);
        let source = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let abs = absolute_lexical(&path);
        Ok((source, abs))
    }
}

/// Parse the root and load its reachable imports in post-order.
///
/// Each imported file occurs once. The root unit is always last.
pub fn closure(root_src: &str, opts: &Options, diags: &mut Diagnostics) -> Vec<Unit> {
    let root = slab_syntax::parse(root_src, diags);
    let mut loader = Loader {
        opts,
        diags,
        units: Vec::new(),
        visited: HashSet::new(),
        stack: Vec::new(),
    };
    for import in &root.imports {
        loader.load(None, import);
    }
    loader.units.push(Unit {
        file: None,
        abs: None,
        doc: root,
    });
    loader.units
}

/// Normalize an import path relative to its normalized importer key.
pub fn normalize(importer: Option<&str>, path: &str) -> String {
    let mut parts = importer
        .and_then(|file| file.rsplit_once('/').map(|(directory, _)| directory))
        .into_iter()
        .flat_map(|directory| directory.split('/'))
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();

    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| last != "..") => {
                parts.pop();
            }
            ".." => parts.push(part.into()),
            _ => parts.push(part.into()),
        }
    }
    parts.join("/")
}

fn absolute_lexical(path: &Path) -> Option<PathBuf> {
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        std::env::current_dir().ok().map(|cwd| cwd.join(path))
    }
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn normalizes_imports_relative_to_the_importer() {
        assert_eq!(normalize(None, "./ui/../theme.slab"), "theme.slab");
        assert_eq!(
            normalize(Some("ui/panels/view.slab"), "../theme.slab"),
            "ui/theme.slab"
        );
        assert_eq!(normalize(Some("ui/view.slab"), "../../x.slab"), "../x.slab");
    }
}
