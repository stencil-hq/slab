// `just editors` — build the distributable VSCode and Zed plugins into out/editors.
//
//  1. VSCode: install the extension's own dependencies, bundle
//     `src/extension.ts`, copy the repo LICENSE next to the manifest, then let
//     `vsce` produce `slab-lang-<version>.vsix`.
//  2. Zed: compile `src/lib.rs` to a `wasm32-wasip2` component and stage the
//     extension tree (manifest, crate sources, language queries, prebuilt
//     `extension.wasm`) into `slab-zed-<version>.tar.gz`.
//
// The staged Zed manifest has `[grammars.slab].rev` restamped to the commit
// being packaged, so a released archive always fetches the grammar revision it
// was built from instead of whatever SHA was last committed by hand.
//
// Run from the repo root: `bun scripts/pack-editors.ts`.

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
const OUT = join(ROOT, 'out/editors');
const VSCODE = join(ROOT, 'editors/vscode');
const ZED = join(ROOT, 'editors/zed');

const run = (cmd: string[], cwd: string) => {
   const p = Bun.spawnSync({ cmd, cwd, stdout: 'inherit', stderr: 'inherit' });
   if (p.exitCode !== 0) {
      throw new Error(`pack-editors: command failed (${p.exitCode}): ${cmd.join(' ')}`);
   }
};

const capture = (cmd: string[]) => {
   const p = Bun.spawnSync({ cmd, cwd: ROOT, stderr: 'inherit' });
   if (p.exitCode !== 0) {
      throw new Error(`pack-editors: command failed (${p.exitCode}): ${cmd.join(' ')}`);
   }
   return p.stdout.toString().trim();
};

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });

// 1. VSCode → out/editors/slab-lang-<version>.vsix
const manifest = JSON.parse(readFileSync(join(VSCODE, 'package.json'), 'utf8')) as {
   name: string;
   version: string;
};
console.log(`pack-editors: vsce package ${manifest.name} ${manifest.version}`);
run(['bun', 'install', '--frozen-lockfile'], VSCODE);
run(['bun', 'run', 'build'], VSCODE);
copyFileSync(join(ROOT, 'LICENSE'), join(VSCODE, 'LICENSE'));
const vsix = join(OUT, `${manifest.name}-${manifest.version}.vsix`);
// `--no-dependencies`: `dist/extension.js` is a self-contained bundle, so the
// runtime `node_modules` tree must not be walked or shipped.
run(['bun', 'x', 'vsce', 'package', '--no-dependencies', '--out', vsix], VSCODE);

// 2. Zed → out/editors/slab-zed-<version>.tar.gz
const extensionToml = readFileSync(join(ZED, 'extension.toml'), 'utf8');
const version = extensionToml.match(/^version = "([^"]+)"/m)?.[1];
if (!version) {
   throw new Error('pack-editors: no `version` in editors/zed/extension.toml');
}
console.log(`pack-editors: cargo build --release --target wasm32-wasip2 (zed ${version})`);
run(['cargo', 'build', '--release', '--target', 'wasm32-wasip2'], ZED);

const wasm = join(ZED, 'target/wasm32-wasip2/release/zed_slab.wasm');
if (!existsSync(wasm)) {
   throw new Error(`pack-editors: missing ${wasm}`);
}

const stageName = `slab-zed-${version}`;
const stage = join(OUT, stageName);
mkdirSync(stage, { recursive: true });
for (const entry of ['Cargo.toml', 'src', 'languages', 'README.md']) {
   cpSync(join(ZED, entry), join(stage, entry), { recursive: true });
}
copyFileSync(join(ROOT, 'LICENSE'), join(stage, 'LICENSE'));
copyFileSync(wasm, join(stage, 'extension.wasm'));

const head = capture(['git', 'rev-parse', 'HEAD']);
writeFileSync(
   join(stage, 'extension.toml'),
   extensionToml.replace(/^rev = "[^"]+"$/m, `rev = "${head}"`),
);

run(['tar', '-czf', join(OUT, `${stageName}.tar.gz`), '-C', OUT, stageName], ROOT);
rmSync(stage, { recursive: true, force: true });

console.log(
   `pack-editors: done → out/editors/${manifest.name}-${manifest.version}.vsix, out/editors/${stageName}.tar.gz`,
);
