# slab-lang build orchestration

# regenerate generated artifacts, then report any drift through `freshness`
# by snapshotting each committed output before rebuilding it
gen:
    cargo run -q -p xtask -- gen-caps
    cargo run -q -p xtask -- support-md
    cargo run -q -p xtask -- gen-proto
    cd tree-sitter-slab && bun x tree-sitter generate
    # Kernel WASM bindings + bundled web runtime are untracked build outputs,
    # but slab-compile embeds slab-runtime.js via include_str!, so they must
    # exist before anything downstream of slab-compile (slab-cli, slab-abi)
    # can build.
    just web-runtime
    # Checked-in typed native modules include external SLIR sidecars. Keep both
    # in lockstep with the codec and compiler alongside every generated artifact.
    cargo run -q -p slab-cli -- gen rust examples/00-player.slab -o crates/slab-native/src/gen_player.rs
    cargo run -q -p slab-cli -- gen rust examples/10-settings.slab -o crates/slab-native/src/gen_settings.rs
    cargo run -q -p slab-cli -- gen rust demos/vscode/vscode.slab -o crates/slab-native/src/gen_vscode.rs
    bun demos/vscode/gen_fs.ts

    # The Go and Python clients embed the same C-ABI kernel+compiler module,
    # and `clients/go/gen` proves the Go generator against a real document.
    cargo run -q -p xtask -- abi-wasm
    cargo run -q -p slab-cli -- gen go examples/10-settings.slab -o clients/go/gen/settings/settings.go --package settings
    gofmt -w clients/go/gen/settings/settings.go

# browser kernel bindings (clients/web/wasm + target/kernel-wasm-node) and the
# bundled web runtime slab-compile embeds via include_str!; untracked, so any
# cargo build of slab-compile needs this first
web-runtime:
    cargo run -q -p xtask -- kernel-wasm
    bun scripts/build-web-runtime.ts

# C-ABI kernel+compiler module embedded by the Go and Python clients (untracked)
abi-wasm: web-runtime
    cargo run -q -p xtask -- abi-wasm

# static checks
check: web-runtime
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    bun x biome check clients packages scripts site tools
    sh -c 'cd tree-sitter-slab && bun x tree-sitter test'
    sh -c 'cd tree-sitter-slab && bun x tree-sitter query queries/highlights.scm ../examples/12-tracklist.slab > /dev/null'

# editor-scale kernel performance benchmarks
bench: web-runtime
    cargo bench -p slab-perf

# Rust workspace unit tests
test: web-runtime
    cargo test --workspace

# Go client: runtime, terminal driver, and generated typed module
go-test: abi-wasm
    cd clients/go && test -z "$(gofmt -l .)" && go build ./... && go vet ./... && go test ./...

# Python client: wasmtime runtime, terminal driver, on-the-fly compilation
py-test: abi-wasm
    cd clients/python && uv run --extra dev pytest -q

# full conformance suite (compile -> native + current Node WASM bindings -> byte-exact goldens)
conformance: web-runtime
    cargo run -q -p slab-cli -- conformance
    bun run tools/conformance-wasm.ts

# regeneration must leave every checked-in artifact byte-identical
freshness:
    #!/usr/bin/env bash
    set -euo pipefail
    snapshot="$(mktemp -d)"
    trap 'rm -rf "$snapshot"' EXIT
    mkdir -p "$snapshot/crates/slab-kernel/src" "$snapshot/crates/slab-slir/src" "$snapshot/crates/slab-native/src" "$snapshot/spec" "$snapshot/tree-sitter-slab" "$snapshot/clients/go"
    cp crates/slab-kernel/src/caps.rs "$snapshot/crates/slab-kernel/src/caps.rs"
    cp crates/slab-slir/src/pb.rs "$snapshot/crates/slab-slir/src/pb.rs"
    cp crates/slab-native/src/gen_player.rs "$snapshot/crates/slab-native/src/gen_player.rs"
    cp crates/slab-native/src/gen_player.slir "$snapshot/crates/slab-native/src/gen_player.slir"
    cp crates/slab-native/src/gen_settings.rs "$snapshot/crates/slab-native/src/gen_settings.rs"
    cp crates/slab-native/src/gen_settings.slir "$snapshot/crates/slab-native/src/gen_settings.slir"
    cp crates/slab-native/src/gen_vscode.rs "$snapshot/crates/slab-native/src/gen_vscode.rs"
    cp crates/slab-native/src/gen_vscode.slir "$snapshot/crates/slab-native/src/gen_vscode.slir"
    cp crates/slab-native/src/vscode_fs.rs "$snapshot/crates/slab-native/src/vscode_fs.rs"
    cp spec/SPEC.md "$snapshot/spec/SPEC.md"
    cp -R tree-sitter-slab/src "$snapshot/tree-sitter-slab/src"
    cp -R clients/go/gen "$snapshot/clients/go/gen"
    just gen
    diff -u "$snapshot/crates/slab-kernel/src/caps.rs" crates/slab-kernel/src/caps.rs
    diff -u "$snapshot/crates/slab-slir/src/pb.rs" crates/slab-slir/src/pb.rs
    diff -u "$snapshot/crates/slab-native/src/gen_player.rs" crates/slab-native/src/gen_player.rs
    cmp "$snapshot/crates/slab-native/src/gen_player.slir" crates/slab-native/src/gen_player.slir
    diff -u "$snapshot/crates/slab-native/src/gen_settings.rs" crates/slab-native/src/gen_settings.rs
    cmp "$snapshot/crates/slab-native/src/gen_settings.slir" crates/slab-native/src/gen_settings.slir
    diff -u "$snapshot/crates/slab-native/src/gen_vscode.rs" crates/slab-native/src/gen_vscode.rs
    cmp "$snapshot/crates/slab-native/src/gen_vscode.slir" crates/slab-native/src/gen_vscode.slir
    diff -u "$snapshot/crates/slab-native/src/vscode_fs.rs" crates/slab-native/src/vscode_fs.rs
    diff -u "$snapshot/spec/SPEC.md" spec/SPEC.md
    diff -ru "$snapshot/tree-sitter-slab/src" tree-sitter-slab/src
    diff -ru "$snapshot/clients/go/gen" clients/go/gen

# build the three npm packages (wasm + tsc dists + license copies)
pack: gen
    bun scripts/pack.ts

# browser web-component integration through installed package tarballs
web-e2e: pack
    bun scripts/pack-e2e.ts

# publish the three npm packages (pack + tarball e2e first); e.g. `just publish --dry-run`
publish *flags: pack
    bun scripts/pack-e2e.ts
    cd clients/web && bun publish {{ flags }}
    cd packages/dslab && bun publish {{ flags }}
    cd packages/slab && bun publish {{ flags }}

# bump the npm package versions in lockstep, commit, tag vX.Y.Z, and push
# (the tag triggers the release workflow, which publishes to npm via OIDC)
version *args:
    bun scripts/version.ts {{ args }}

# build the VSCode .vsix and Zed .tar.gz plugins into out/editors
editors:
    bun scripts/pack-editors.ts

# build the playground site (depends on pack for the web wasm)
site: pack
    bun scripts/site-build.ts

# serve the built playground locally (build it first with `just site`)
serve:
    bun scripts/serve.ts site/dist

# Run `just site` once first so the compiler/kernel wasm is already in site/dist.
# playground dev loop: rebuild on change + live-reload the browser (no wasm rebuild)
dev:
    bun scripts/dev.ts

# (fast subset of `just pack`; assumes the Cargo.lock-pinned wasm-bindgen CLI)
# refresh the playground's in-browser compiler wasm after Rust compiler changes
dev-wasm:
    cargo build --release --target wasm32-unknown-unknown -p slab-wasm
    wasm-bindgen --target web --out-dir site/dist/wasm target/wasm32-unknown-unknown/release/slab_wasm.wasm

# serve the web-component demo pages
demo:
    bun scripts/serve.ts examples/web-demo

# open a .slab document in the native wgpu client
native file="examples/00-player.slab":
    cargo run -q -p slab-native -- {{ file }}

# open a .slab document in the terminal client
tui file="examples/00-player.slab":
    cargo run -q -p slab-tui -- {{ file }}

# open a .slab document in the Go terminal client (wazero-backed)
go-tui file="examples/00-player.slab":
    cd clients/go && go run ./example -file {{ justfile_directory() }}/{{ file }}

# open a .slab document in the Python terminal client (compiled on the fly)
py-tui file="examples/00-player.slab":
    cd clients/python && uv run python -m slab {{ justfile_directory() }}/{{ file }}

# browse a directory of .slab documents in the terminal client (Ctrl-N / Ctrl-P)
gallery dir="examples":
    cargo run -q -p slab-tui -- --examples {{ dir }}

# render documents through ghostty (libghostty-vt), native wgpu, and the web
# runtime, then stack each set into out/compare/<doc>.png
compare *docs:
    bun tools/compare.ts {{ docs }}

ci: check test conformance freshness go-test py-test
