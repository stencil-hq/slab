# Repository Guidelines

Use this file to navigate Slab safely. Prefer a small, targeted check over a
workspace-wide rebuild unless a change crosses compiler, kernel, or generated-artifact boundaries.

## Project Overview

Slab is a design language for agents. Rust compiles `.slab` source to **SLIR**
(Protobuf in a raw-Snappy envelope); one hand-maintained Rust kernel evaluates
layout, responsive conditions, animation, editing, hit-testing, focus, and
events. Thin native, terminal, browser/WASM, and static-rendering drivers paint
the same kernel output.

**Core invariant:** there is one solver in `crates/slab-kernel/`. Do not add
platform-specific layout or interaction behavior to a driver when it belongs in
the kernel.

## Architecture & Data Flow

```text
.slab source
  -> crates/slab-syntax       lex, parse, format, diagnostics
  -> crates/slab-compile      expand components/imports, embed assets, lower
  -> crates/slab-slir         encode/decode/dump the binary document
  -> crates/slab-kernel       instantiate, lay out, dispatch, produce Frame/Scene
  -> slab-cli | slab-tui | slab-native | slab-wasm | clients/web
```

- `crates/slab-cli/src/main.rs` is the reference command surface: `check`,
  `build`, `dump`, `fmt`, `render`, `conformance`, `drive`, `lsp`, and `gen`.
- The kernel accepts decoded SLIR plus environment, params, and retained state;
  clients translate host input into kernel events and paint returned frames.
- `slab-wasm` exposes compiler and renderer paths to JavaScript. `clients/web/`
  renders its kernel frames as custom elements; `site/` is the live WASM
  playground.
- `packages/dslab/` speaks the newline-delimited Slab Drive Protocol (SDP),
  either over TCP or a spawned `slab drive` process.
- Determinism is intentional: kernel arithmetic, output quantization, and
  ordering must stay host-independent so native and WASM conformance goldens
  remain byte-identical.

## Key Directories

| Path | Purpose |
| --- | --- |
| `crates/slab-syntax/` | Lexer, parser, AST spans, formatter, diagnostics. |
| `crates/slab-compile/` | Semantic compilation, expansion, SLIR lowering, static render and code generation. |
| `crates/slab-slir/` | Normative SLIR structures, binary reader/writer, canonical dump. |
| `crates/slab-kernel/` | Shared deterministic runtime: layout, scene, input, editing, animation. |
| `crates/slab-cli/` | Native CLI, conformance runner, Drive Protocol server. |
| `crates/slab-{tui,native,lsp,wasm}/` | Terminal, wgpu, language-server, and WASM host adapters. |
| `clients/web/` | `@stencil-hq/wslab`: `SlabElement`, frame decode, DOM/canvas painter, browser WASM glue. |
| `packages/slab/` | `@stencil-hq/slab` WASM-backed npm CLI. |
| `packages/dslab/` | `@stencil-hq/dslab` typed SDP client and `dslab` CLI. |
| `site/` | CodeMirror playground, preview, inspector, and design-mode UI. |
| `conformance/` | Shared cases, traces, manifest, and byte-exact expected outputs. |
| `spec/` | Normative language, SLIR, frame API, and platform-support specifications. |
| `tree-sitter-slab/` | Editor grammar and corpus/highlight tests. |
| `scripts/`, `tools/` | Packaging, site/dev servers, conformance, and browser E2E runners. |

## Development Commands

```sh
# Bootstrap
bun install

# Usual validation layers
just check          # rustfmt, clippy -D warnings, Biome, tree-sitter checks
just test           # cargo test --workspace
just conformance    # native and WASM cases against checked-in goldens
just freshness      # regenerate in a temp snapshot and reject drift
just ci             # check + test + conformance + freshness

# Generation and packages
just gen             # refresh all committed derived artifacts
just pack            # build @stencil-hq/wslab, slab, and dslab distributions
bun scripts/pack-e2e.ts
just editors         # build the VSCode .vsix and Zed .tar.gz into out/editors

# Browser/site work
just site            # bundle site/dist
just dev             # local site server with live reload
just dev-wasm        # refresh the playground WASM compiler after a WASM change
just web-e2e         # Playwright web-component integration tests
```

Use focused commands while iterating:

```sh
cargo test -p slab-kernel
cargo test -p slab-compile
cargo run -q -p slab-cli -- check examples/10-settings.slab
cargo run -q -p slab-cli -- render examples/10-settings.slab -o /tmp/settings.png
cargo run -q -p slab-cli -- dump path/to/document.slir
bun test packages/dslab/test/drive.test.ts
```

## Code Conventions & Common Patterns

### Architecture patterns

- Keep compiler, binary-format, runtime, and host concerns in their existing
  layers. A syntax or semantic change normally flows through `slab-syntax` and
  `slab-compile`; a layout or input behavior change belongs in `slab-kernel`.
- Preserve the shared-kernel model. Drivers should adapt input/output, not
  independently solve layout, focus, animation, or text editing.
- Treat frame/scene output as a deterministic contract. Avoid unordered output,
  host-dependent metrics, lossy numeric changes, and platform-only fallbacks.
- State is retained by kernel instances. Apply parameter/environment/input
  changes through the existing instance and dispatch APIs rather than recreating
  ad-hoc state in a renderer.

### Rust

- Use Rust stable, edition 2024. `cargo clippy -- -D warnings` is part of the
  normal gate.
- Return and accumulate diagnostics through the existing collectors; do not
  replace user-facing parse/compile failures with panics.
- Follow compact kernel representations and indexed-tree conventions already in
  the module being changed. Do not introduce broad allocations on hot frame or
  event paths without need.
- Keep tests near their crate when exercising runtime/compiler behavior; use
  `crates/*/tests/` for integration-level contracts.

### TypeScript and browser code

- Use strict, explicit types at Rust/WASM and wire-protocol boundaries. Preserve
  C-ABI/event constants and binary frame decoding conventions in `clients/web/`.
- Browser signals are `CustomEvent`s; generated web components build on
  `SlabElement` in `clients/web/element.ts`.
- SDP is line-delimited JSON. Use `DriveClient` in `packages/dslab/src/index.ts`
  for TCP, spawned stdio, or caller-owned streams instead of duplicating protocol
  framing.
- Biome governs JS/TS: 3-space indentation, 100-column width, and single quotes.

### Generated artifacts

Do not hand-edit generated output. Edit its input, run `just gen`, and include
all resulting intentional updates. Important derived targets include
`crates/slab-kernel/src/caps.rs`, `crates/slab-slir/src/pb.rs`,
`tree-sitter-slab/src/`, `clients/web/wasm/` (the only committed kernel WASM),
`gen/web-runtime/slab-runtime.js`, and generated native modules under
`crates/slab-native/src/`.

`spec/support.toml` drives capability tables; `spec/slir.proto` drives generated
bindings. Changes to either require regeneration and freshness verification.

## Important Files

| File | Why it matters |
| --- | --- |
| `Cargo.toml` | Rust workspace members and shared dependencies. |
| `justfile` | Canonical build, validation, generation, and development commands. |
| `biome.json` | JavaScript/TypeScript formatter and linter configuration. |
| `crates/slab-cli/src/main.rs` | CLI command routing and a practical debugging entry point. |
| `crates/slab-compile/src/lib.rs` | Compiler orchestration. |
| `crates/slab-kernel/src/lib.rs` | Kernel public runtime surface. |
| `clients/web/element.ts` | Browser custom-element lifecycle and kernel bridge. |
| `packages/dslab/src/index.ts` | Typed SDP transport/client implementation. |
| `scripts/pack.ts` | WASM binding generation and npm-package assembly. |
| `scripts/pack-e2e.ts` | Isolated tarball-install smoke test. |
| `scripts/pack-editors.ts` | VSCode `.vsix` and Zed `.tar.gz` plugin assembly. |
| `spec/SPEC.md` | Normative source-language and runtime behavior. |
| `spec/SLIR.md`, `spec/slir.proto` | Binary format contract and schema. |
| `spec/FRAME.md` | Kernel frame/event public contract. |

## Runtime/Tooling Preferences

- Use **Bun** for JavaScript/TypeScript dependencies and commands; do not switch
  this workspace to npm, pnpm, or Yarn.
- Required local tools: Rust stable, Bun, and `just`. Add
  `wasm32-unknown-unknown` before regenerating or packaging WASM artifacts:

  ```sh
  rustup target add wasm32-unknown-unknown
  ```

- `scripts/pack.ts` resolves the `wasm-bindgen-cli` version pinned in
  `Cargo.lock`. Do not manually generate bindings with a mismatched CLI version.
- `@stencil-hq/slab` is the WASM-backed npm CLI; `@stencil-hq/dslab` requires
  Node 22+ when used under Node. The repository itself uses Bun for its scripts
  and workspace management.

## Testing & QA

Run the narrowest test that proves the changed contract, then add the relevant
cross-layer check when the change crosses a boundary.

| Change area | Primary proof |
| --- | --- |
| Parser/grammar | `bun x tree-sitter test` and corpus/query checks via `just check`. |
| Compiler diagnostics | Relevant `slab-compile` tests plus `slab check` on a minimal fixture. |
| Kernel layout/input/editing | `cargo test -p slab-kernel`; use an existing conformance trace if behavior is cross-host. |
| SLIR/runtime contract | `just conformance` — native and WASM must match the same goldens byte-for-byte. |
| Browser custom elements | `just web-e2e`; inspect `clients/web/element.ts`, `frame-decode.ts`, and `painter.ts`. |
| npm package layout | `just pack` then `bun scripts/pack-e2e.ts`. |
| Editor plugins | `just editors`; artifacts land in `out/editors/`. |
| Spec/proto/support/generated input | `just gen` then `just freshness`. |

Do not refresh conformance goldens merely to make a test pass. First isolate the
semantic change, verify it is intentional across native and WASM, then update
fixtures/goldens as part of that explicit behavior change.

## Debugging Workflow

1. **Source diagnostics:** run `cargo run -q -p slab-cli -- check FILE.slab`.
   Start with a minimal `.slab` reproduction and preserve the diagnostic code,
   level, and span contract in regression tests.
2. **Inspect the compiler boundary:** build SLIR, then inspect it with
   `slab dump`. This separates syntax/compile errors from kernel behavior.
3. **Inspect runtime output:** render a small SVG/PNG or drive the TUI:

   ```sh
   cargo run -q -p slab-tui -- FILE.slab --debug
   cargo run -q -p slab-tui -- FILE.slab --script 'CLICK:10,20 TICK:100' --dump-after -
   ```

4. **Interrogate a live kernel session:**

   ```sh
   slab drive FILE.slab --port 4242
   dslab --port 4242 scene.tree
   dslab --port 4242 clock.advance '{"ms":25}'
   ```

   Use `packages/dslab/src/index.ts` for programmatic probes; `dslab` prints one
   result JSON value per invocation.
5. **Classify cross-host failures:** run `just conformance`. If native passes and
   WASM fails, inspect the WASM boundary/bindings; if both fail, start in the
   compiler or kernel. For stale-output failures, use `just freshness` and trace
   the changed generator input rather than editing the artifact.
6. **Package/browser failures:** use `bun scripts/pack-e2e.ts` for an installed
   tarball reproduction, and `just web-e2e` for DOM/event behavior. Ensure the
   required WASM sidecars were generated before debugging application code.
