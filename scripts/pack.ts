// `just pack` — build the three npm packages from the single Rust kernel.
//
//  1. Resolve the pinned wasm-bindgen-cli version from Cargo.lock and install
//     it if the installed CLI differs.
//  2. Build the browser kernel bindings and sidecar with xtask.
//  3. Build `slab-wasm`, then emit its Node binding for the CLI and web binding
//     for the playground.
//  4. `tsc` the web driver, CLI, and SDP client, then copy the browser kernel
//     artifacts beside the compiled web driver.
//  5. Copy license files into the publishable packages (fonts are embedded in
//     the compiler wasm; OFL notices must travel with the CLI package).
//
// Run from the repo root: `bun scripts/pack.ts`.

import {
   copyFileSync,
   cpSync,
   existsSync,
   mkdirSync,
   readFileSync,
   rmSync,
   writeFileSync,
} from 'node:fs';
import { join, resolve } from 'node:path';

const ROOT = resolve(import.meta.dirname, '..');
const run = (cmd: string[], opts?: { cwd?: string; env?: Record<string, string> }) => {
   const r = Bun.spawnSync({
      cmd,
      cwd: opts?.cwd ?? ROOT,
      env: { ...process.env, ...opts?.env },
      stdin: 'ignore',
      stdout: 'inherit',
      stderr: 'inherit',
   });
   if (r.exitCode !== 0) {
      console.error(`pack: command failed: ${cmd.join(' ')}`);
      process.exit(1);
   }
};

// 1. wasm-bindgen-cli version pinned to Cargo.lock.
const lock = readFileSync(join(ROOT, 'Cargo.lock'), 'utf8');
const m = lock.match(/^name = "wasm-bindgen"\nversion = "([^"]+)"/m);
if (!m) {
   console.error('pack: cannot find wasm-bindgen version in Cargo.lock');
   process.exit(1);
}
const wbVersion = m[1];
console.log(`pack: wasm-bindgen ${wbVersion}`);
// Check installed CLI version; install if missing or mismatched.
const installed = Bun.spawnSync({
   cmd: ['wasm-bindgen', '--version'],
   stdin: 'ignore',
   stdout: 'pipe',
   stderr: 'pipe',
});
const instOut = installed.stdout?.toString() ?? '';
const instMatch = instOut.match(/([\d.]+)/);
if (!instMatch || instMatch[1] !== wbVersion) {
   console.log(`pack: installing wasm-bindgen-cli ${wbVersion}`);
   run(['cargo', 'install', 'wasm-bindgen-cli', '--version', wbVersion, '--locked']);
}

// 2. Build the browser kernel bindings and sidecar.
console.log('pack: cargo run -q -p xtask -- kernel-wasm');
run(['cargo', 'run', '-q', '-p', 'xtask', '--', 'kernel-wasm']);

// 3. Build the compiler wasm crate.
console.log('pack: cargo build --profile dist --target wasm32-unknown-unknown -p slab-wasm');
run([
   'cargo',
   'build',
   '--profile',
   'dist',
   '--target',
   'wasm32-unknown-unknown',
   '-p',
   'slab-wasm',
]);

const wasmPath = join(ROOT, 'target/wasm32-unknown-unknown/dist/slab_wasm.wasm');
if (!existsSync(wasmPath)) {
   console.error(`pack: expected wasm output at ${wasmPath}`);
   process.exit(1);
}

// 4a. nodejs binding → packages/slab/wasm (CLI).
rmSync(join(ROOT, 'packages/slab/wasm'), { recursive: true, force: true });
mkdirSync(join(ROOT, 'packages/slab/wasm'), { recursive: true });
// The nodejs binding is CJS; packages/slab is "type": "module", so mark the
// wasm subdir as commonjs so node treats slab_wasm.js correctly. Written
// AFTER wasm-bindgen so it isn't overwritten.
console.log('pack: wasm-bindgen --target nodejs → packages/slab/wasm');
run([
   'wasm-bindgen',
   '--target',
   'nodejs',
   '--out-dir',
   join(ROOT, 'packages/slab/wasm'),
   wasmPath,
]);
writeFileSync(join(ROOT, 'packages/slab/wasm/package.json'), '{ "type": "commonjs" }\n');

// 4b. web binding → site/dist/wasm (playground).
mkdirSync(join(ROOT, 'site/dist/wasm'), { recursive: true });
console.log('pack: wasm-bindgen --target web → site/dist/wasm');
run(['wasm-bindgen', '--target', 'web', '--out-dir', join(ROOT, 'site/dist/wasm'), wasmPath]);

// 4. Compile the three TypeScript packages and ship the kernel artifacts beside
// both source and default/dist wslab entry points.
console.log('pack: tsc wslab → clients/web/dist');
rmSync(join(ROOT, 'clients/web/dist'), { recursive: true, force: true });
run(['bun', 'x', 'tsc', '-p', 'scripts/tsconfig.wslab.json']);
cpSync(join(ROOT, 'clients/web/wasm'), join(ROOT, 'clients/web/dist/wasm'), {
   recursive: true,
});

console.log('pack: tsc slab-cli → packages/slab/dist');
rmSync(join(ROOT, 'packages/slab/dist'), { recursive: true, force: true });
run(['bun', 'x', 'tsc', '-p', 'scripts/tsconfig.slab-cli.json']);

console.log('pack: tsc dslab → packages/dslab/dist');
rmSync(join(ROOT, 'packages/dslab/dist'), { recursive: true, force: true });
run(['bun', 'x', 'tsc', '-p', 'scripts/tsconfig.dslab.json']);

// 5. Copy package license files.
const copy = (src: string, dst: string) => copyFileSync(src, dst);
copy(join(ROOT, 'LICENSE'), join(ROOT, 'clients/web/LICENSE'));
copy(join(ROOT, 'LICENSE'), join(ROOT, 'packages/slab/LICENSE'));
copy(join(ROOT, 'LICENSE'), join(ROOT, 'packages/dslab/LICENSE'));
copy(
   join(ROOT, 'assets/fonts/LICENSE-Inter-OFL.txt'),
   join(ROOT, 'packages/slab/LICENSE-Inter-OFL.txt'),
);
copy(
   join(ROOT, 'assets/fonts/LICENSE-JetBrainsMono-OFL.txt'),
   join(ROOT, 'packages/slab/LICENSE-JetBrainsMono-OFL.txt'),
);

console.log('pack: done');
