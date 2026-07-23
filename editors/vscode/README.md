# Slab for VSCode

Language support for Slab (see `spec/SPEC.md` in the repo root), a design language for agents. Provides syntax highlighting, snippets, bracket/comment editing behavior, an LSP client for `slab lsp`, and a live SVG preview.

## Install

Every `ci` run uploads an `editor-plugins` artifact and every `v*` tag attaches
the same files to its GitHub release; download `slab-lang-<version>.vsix` from
either and run `code --install-extension slab-lang-<version>.vsix`.

To build it yourself, from the repo root:

```sh
just editors           # -> out/editors/slab-lang-<version>.vsix
```

For development, symlink instead of packaging:

```sh
ln -s "$(pwd)" ~/.vscode/extensions/slab-lang.slab-lang-0.1.0
```

## Language server

The extension launches the stdio LSP server built into the `slab` reference CLI. Default command: `slab lsp` — install the CLI once with `cargo install --path crates/slab-cli` (from the repo root) so it is on PATH for every workspace. Inside the Slab repo itself, `["cargo", "run", "-q", "-p", "slab-cli", "--", "lsp"]` also works via the setting below.

If the server cannot spawn, the extension warns once and keeps working as a grammar-only extension.

## Preview

`Slab: Open Preview` (editor title button on `.slab` files) opens a live-updating panel beside the editor. It re-renders ~150 ms after each edit via the custom `slab/preview` LSP request (compile → kernel solve → SVG, the same path as `slab render`). The solve width tracks the panel automatically: resizing the panel re-solves the document, so `when w<...` breakpoints respond. Toolbar: Fit / 100% / 200% zoom and a `w` input that pins a manual width (clear it to return to auto). Diagnostics from the render are listed under the canvas; if rendering fails, the error list is shown instead. Requires the language server.

## Settings

| Setting | Default | Description |
| --- | --- | --- |
| `slab.lsp.enabled` | `true` | Start the language server for `.slab` files. |
| `slab.lsp.command` | `["slab", "lsp"]` | Argv used to launch the server over stdio. |

## Commands

- `Slab: Open Preview` — open/reveal the live preview panel for the active `.slab` document.
- `Slab: Restart Language Server` — stop and relaunch the server (also refreshes open previews).

## Development

```sh
bun run check           # bunx tsc --noEmit
bun run check-grammar   # tokenizes examples/*.slab against the TextMate grammar
```
