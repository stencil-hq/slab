// `just site` — build the static playground into site/dist.
//
// 1. `bun build site/main.ts` → site/dist/main.js (ESM, browser target) plus
//    the split chunk holding the terminal emulator the TUI view imports on
//    demand.
// 2. Copy index.html, style.css, the committed web runtime and kernel sidecar,
//    and examples (+ a generated manifest.json) into site/dist.
// 3. Expects the compiler wasm in site/dist/wasm from `just pack` step 3b.
//    The `site` justfile target depends on `pack`.
//
// Run from the repo root: `bun scripts/site-build.ts`.

import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import Bun from 'bun';

const ROOT = resolve(import.meta.dirname, '..');
const DIST = join(ROOT, 'site/dist');
const DIST_WASM = join(DIST, 'wasm');
const KERNEL_WASM = join(ROOT, 'clients/web/wasm/slab_kernel_bg.wasm');

if (!existsSync(KERNEL_WASM)) {
   console.error(
      'site-build: missing clients/web/wasm/slab_kernel_bg.wasm — run `cargo run -q -p xtask -- kernel-wasm` first',
   );
   process.exit(1);
}

// 1. bundle main.ts — chunk names carry content hashes, so clear the old set
if (existsSync(DIST)) {
   for (const file of readdirSync(DIST)) {
      if (file === 'main.js' || (file.startsWith('chunk-') && file.endsWith('.js'))) {
         rmSync(join(DIST, file));
      }
   }
}
const result = await Bun.build({
   entrypoints: [join(ROOT, 'site/main.ts')],
   outdir: DIST,
   format: 'esm',
   target: 'browser',
   minify: true,
   splitting: true,
   external: ['./wasm/slab_wasm.js'],
   // @stencil-hq/wslab ships workspace TS sources behind the `bun` condition.
   conditions: ['bun'],
});
if (!result.success) {
   for (const log of result.logs) console.error(log);
   process.exit(1);
}

// 2. copy static assets
const copy = (src: string, dst: string) => copyFileSync(src, dst);
copy(join(ROOT, 'site/index.html'), join(DIST, 'index.html'));
copy(join(ROOT, 'site/style.css'), join(DIST, 'style.css'));
copy(join(ROOT, 'site/favicon.svg'), join(DIST, 'favicon.svg'));

// web-runtime (shared by gen_wc modules the playground emits)
copy(join(ROOT, 'gen/web-runtime/slab-runtime.js'), join(DIST, 'slab-runtime.js'));
mkdirSync(DIST_WASM, { recursive: true });
copy(KERNEL_WASM, join(DIST_WASM, 'slab_kernel_bg.wasm'));

// examples + manifest
mkdirSync(join(DIST, 'examples'), { recursive: true });
const examples = readdirSync(join(ROOT, 'examples'))
   .filter((f) => f.endsWith('.slab'))
   .sort();
for (const f of examples) {
   copy(join(ROOT, 'examples', f), join(DIST, 'examples', f));
}
writeFileSync(join(DIST, 'examples/manifest.json'), `${JSON.stringify(examples)}\n`);

// 3. verify the compiler glue and kernel sidecar exist
if (!existsSync(join(DIST_WASM, 'slab_wasm.js'))) {
   console.error('site-build: missing site/dist/wasm/slab_wasm.js — run `just pack` first');
   process.exit(1);
}
if (!existsSync(join(DIST_WASM, 'slab_kernel_bg.wasm'))) {
   console.error(
      'site-build: missing site/dist/wasm/slab_kernel_bg.wasm — run `cargo run -q -p xtask -- kernel-wasm`, then rerun the site build',
   );
   process.exit(1);
}

console.log('site-build: done → site/dist');
