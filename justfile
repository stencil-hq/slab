# slab-lang build orchestration

# regenerate generated artifacts, then report any drift through `freshness`
# by snapshotting each committed output before rebuilding it
gen:
    cargo run -q -p xtask -- gen-caps
    cargo run -q -p xtask -- support-md
    cargo run -q -p xtask -- gen-proto
    cd tree-sitter-slab && bun x tree-sitter generate
    # Checked-in typed native modules embed compiled SLIR. Keep their bytes in
    # lockstep with the codec and compiler alongside every generated artifact.
    cargo run -q -p slab-cli -- gen rust examples/00-player.slab -o crates/slab-native/src/gen_player.rs
    cargo run -q -p slab-cli -- gen rust examples/10-settings.slab -o crates/slab-native/src/gen_settings.rs
    rustfmt --edition 2024 crates/slab-native/src/gen_player.rs crates/slab-native/src/gen_settings.rs

    cargo run -q -p xtask -- kernel-wasm

    # `gen wc` publishes this bundle alongside the kernel WASM that
    # `kernel-wasm` just wrote to clients/web/wasm. Build it last so its
    # bundled wasm-bindgen glue matches that kernel.
    bun build clients/web/index.ts --outfile gen/web-runtime/slab-runtime.js --format=esm --target=browser --minify --conditions=bun

# static checks
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    bun x biome check clients packages scripts site tools
    sh -c 'cd tree-sitter-slab && bun x tree-sitter test'
    sh -c 'cd tree-sitter-slab && bun x tree-sitter query queries/highlights.scm ../examples/12-tracklist.slab > /dev/null'

# Rust workspace unit tests
test:
    cargo test --workspace

# full conformance suite (compile -> native + current Node WASM bindings -> byte-exact goldens)
conformance:
    cargo run -q -p slab-cli -- conformance
    cargo run -q -p xtask -- kernel-wasm
    bun run tools/conformance-wasm.ts

# regeneration must leave every checked-in artifact byte-identical
freshness:
    #!/usr/bin/env bash
    set -euo pipefail
    snapshot="$(mktemp -d)"
    trap 'rm -rf "$snapshot"' EXIT
    mkdir -p "$snapshot/clients/web" "$snapshot/crates/slab-kernel/src" "$snapshot/crates/slab-slir/src" "$snapshot/crates/slab-native/src" "$snapshot/gen" "$snapshot/spec" "$snapshot/tree-sitter-slab"
    cp -R gen/web-runtime "$snapshot/gen/web-runtime"
    cp -R clients/web/wasm "$snapshot/clients/web/wasm"
    cp crates/slab-kernel/src/caps.rs "$snapshot/crates/slab-kernel/src/caps.rs"
    cp crates/slab-slir/src/pb.rs "$snapshot/crates/slab-slir/src/pb.rs"
    cp crates/slab-native/src/gen_player.rs "$snapshot/crates/slab-native/src/gen_player.rs"
    cp crates/slab-native/src/gen_settings.rs "$snapshot/crates/slab-native/src/gen_settings.rs"
    cp spec/SPEC.md "$snapshot/spec/SPEC.md"
    cp -R tree-sitter-slab/src "$snapshot/tree-sitter-slab/src"
    just gen
    diff -ru "$snapshot/gen/web-runtime" gen/web-runtime
    diff -ru "$snapshot/clients/web/wasm" clients/web/wasm
    diff -u "$snapshot/crates/slab-kernel/src/caps.rs" crates/slab-kernel/src/caps.rs
    diff -u "$snapshot/crates/slab-slir/src/pb.rs" crates/slab-slir/src/pb.rs
    diff -u "$snapshot/crates/slab-native/src/gen_player.rs" crates/slab-native/src/gen_player.rs
    diff -u "$snapshot/crates/slab-native/src/gen_settings.rs" crates/slab-native/src/gen_settings.rs
    diff -u "$snapshot/spec/SPEC.md" spec/SPEC.md
    diff -ru "$snapshot/tree-sitter-slab/src" tree-sitter-slab/src

# build the three npm packages (wasm + tsc dists + license copies)
pack: gen
    bun scripts/pack.ts

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
    cargo run -q -p slab-native -- {{file}}

# open a .slab document in the terminal client
tui file="examples/00-player.slab":
    cargo run -q -p slab-tui -- {{file}}

# browse a directory of .slab documents in the terminal client (Ctrl-N / Ctrl-P)
gallery dir="examples":
    cargo run -q -p slab-tui -- --examples {{dir}}

# render documents through ghostty (libghostty-vt), native wgpu, and the web
# runtime, then stack each set into out/compare/<doc>.png
compare *docs:
    bun tools/compare.ts {{docs}}

ci: check test conformance freshness
