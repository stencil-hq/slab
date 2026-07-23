# Slab for Zed

Zed extension for the [Slab](../../spec/SPEC.md) design language (`.slab` files): tree-sitter
syntax highlighting, outline, auto-indent, bracket matching, and LSP diagnostics via
`slab lsp`.

## Install

Every `ci` run uploads an `editor-plugins` artifact and every `v*` tag attaches
the same files to its GitHub release. Download `slab-zed-<version>.tar.gz`,
extract it, and install the extracted directory through **Install Dev
Extension** (below) — Zed recompiles the bundled crate against your Zed version
and compiles the grammar itself.

Build the same archive locally from the repo root with `just editors`
(`-> out/editors/slab-zed-<version>.tar.gz`).

## Dev install

1. Prerequisites: Rust with the `wasm32-wasip2` target (`rustup target add wasm32-wasip2`).
2. In Zed, run `zed: extensions` from the command palette, click **Install Dev Extension**,
   and select this directory (`editors/zed`).

Zed compiles `src/lib.rs` to wasm and fetches the grammar declared in `extension.toml`.

## Grammar pinning

`[grammars.slab]` points at the slab-lang repo with `path = "tree-sitter-slab"` and pins a
git `rev`. Whenever `tree-sitter-slab` changes, commit and bump `rev` to the new SHA, then
rebuild the dev extension so Zed refetches the grammar.

`just editors` restamps `rev` in the *packaged* manifest to the commit it built from, so a
released archive always fetches a grammar that matches its own sources. The committed value
only governs dev installs straight from a checkout.

## Language server

The extension launches the `slab lsp` server (built into the reference CLI) over stdio:

- If a `slab` binary is on `PATH`, it runs `slab lsp`.
- Install it once with `cargo install --path crates/slab-cli` from the repo root.

Override the binary in Zed `settings.json`:

```json
{
  "lsp": {
    "slab-lsp": {
      "binary": { "path": "/path/to/slab", "arguments": ["lsp"] }
    }
  }
}
```

## Preview

The VSCode extension ships a live preview panel over the custom `slab/preview` LSP
request. In Zed, render on demand instead — wire a worktree task (`zed: open project
tasks`, paste into `.zed/tasks.json`):

```json
[
  {
    "label": "Slab: render SVG",
    "command": "slab",
    "args": ["render", "$ZED_FILE", "-o", "/tmp/slab-preview.svg"],
    "reveal": "no_focus",
    "allow_concurrent_runs": false
  }
]
```

`$ZED_FILE` expands to the absolute path of the file being edited; run it via
`task: spawn` with a `.slab` buffer focused, then open the written SVG.
