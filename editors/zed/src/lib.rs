//! Zed extension entry point wiring the `slab lsp` language server.

use zed_extension_api::settings::LspSettings;
use zed_extension_api::{self as zed, LanguageServerId, Result};

/// Zed extension providing the Slab language server command.
struct SlabExtension;

impl zed::Extension for SlabExtension {
    fn new() -> Self {
        SlabExtension
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        // user settings override everything (lsp.slab-lsp.binary.{path,arguments})
        let binary_settings = LspSettings::for_worktree("slab-lsp", worktree)
            .ok()
            .and_then(|settings| settings.binary);
        if let Some(binary) = binary_settings {
            if let Some(path) = binary.path {
                return Ok(zed::Command {
                    command: path,
                    args: binary.arguments.unwrap_or_else(|| vec!["lsp".to_string()]),
                    env: Default::default(),
                });
            }
        }

        // the reference CLI ships the server: `slab lsp`
        let slab = worktree.which("slab").ok_or_else(|| {
            "`slab` not found on PATH; install the reference CLI with \
             `cargo install --path crates/slab-cli` from the slab repo"
                .to_string()
        })?;
        Ok(zed::Command {
            command: slab,
            args: vec!["lsp".to_string()],
            env: Default::default(),
        })
    }
}

zed::register_extension!(SlabExtension);
