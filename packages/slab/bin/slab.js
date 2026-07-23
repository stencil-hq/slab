#!/usr/bin/env node
// slab CLI entry — delegates to the compiled dist/cli.js (wasm-backed,
// zero Rust on the host). `bin` in package.json points here.
import '../dist/cli.js';
