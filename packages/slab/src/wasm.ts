// Loads the wasm-bindgen `--target nodejs` output (CJS, sitting beside its
// `.wasm` file under ../wasm/). The nodejs loader resolves the wasm path
// relative to itself at runtime and self-initializes on require, so we use
// `createRequire` to pull it in from the ESM CLI. `dist/cli.js` MUST import
// this via a real relative path (not bundled inline) —
// `scripts/tsconfig.slab-cli.json` keeps `module: nodenext` so `../wasm/`
// stays a sibling import.
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const slabWasm = require('../wasm/slab_wasm.js') as typeof import('../wasm/slab_wasm.js');

/** The wasm module surface (check, build, dump, render, gen_wc, gen_react, gen_rust). */
export function wasm(): typeof slabWasm {
   return slabWasm;
}
