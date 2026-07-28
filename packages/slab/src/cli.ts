#!/usr/bin/env node
// `@stencil-hq/slab` — the npm CLI. Mirrors `crates/slab-cli/src/main.rs`
// for check | build | dump | render | gen wc | gen react | gen rust | dev,
// backed by slab-wasm (zero Rust on the host). conformance/selftest/lsp
// stay Rust-only.
//
// Flow per compile command: read the .slab (or .slir) file; for compile
// paths call `image_srcs`, read each existing file relative to the .slab's
// dir, base64 into `assets_json` (missing files are NOT an error here — the
// compiler's own warn covers it); call the wasm fn; write outputs / print
// `formatted` diags to stderr; exit 1 on any `level=="error"`, 2 on usage
// errors (matching the Rust CLI codes).

import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, extname, join, resolve } from 'node:path';
import { loadImports } from './core.ts';
import {
   DEV_USAGE,
   type DevBuildResult,
   type DevOptions,
   DevUsageError,
   parseDevArgs,
   startDevServer,
} from './dev.ts';
import { wasm } from './wasm.ts';

const packageJson: unknown = JSON.parse(
   readFileSync(new URL('../package.json', import.meta.url), 'utf8'),
);
if (
   packageJson === null ||
   typeof packageJson !== 'object' ||
   !('version' in packageJson) ||
   typeof packageJson.version !== 'string'
) {
   throw new Error('package.json has no string version');
}
const PACKAGE_VERSION = packageJson.version;

const USAGE = `\
usage: slab <command> [args]

commands:
  check FILE [--width N] [--height N] [--state a,b] [--env portrait,dark]
             [--client gpu]                print diagnostics (exit 1 on errors)
  build FILE -o OUT.slir [--no-embed-assets]   compile to SLIR
  dump FILE.slir                           print the canonical slir-dump text
  render FILE -o OUT.{svg,png,apng,txt}    static export (see \`slab render --help\`)
  gen wc FILE -o DIR [--tag NAME] [--separate-ir]   emit a web-component module
  gen react FILE -o DIR [--tag NAME] [--separate-ir]   emit a web component + typed React wrapper
  dev FILE [-o DIR] [--tag NAME] [--separate-ir] [--host HOST] [--port N]
                                            serve a live web-component preview
  gen rust FILE -o OUT.rs                  emit a typed Rust module (native client)
  drive                                    requires native slab-cli (see below)

drive requires the native slab-cli:
  cargo install --git https://github.com/stencil-hq/slab slab-cli
`;
const CHECK_USAGE = `usage: slab check FILE [--width N] [--height N] [--state a,b]
                       [--env portrait,dark,coarse] [--client web|gpu|tui|svg|png]
`;
const BUILD_USAGE = 'usage: slab build FILE -o OUT.slir [--no-embed-assets]\n';
const DUMP_USAGE = 'usage: slab dump FILE.slir\n';
const RENDER_USAGE = `usage: slab render FILE [-o OUT.{svg,png,apng,txt}]
                        [--client web|gpu|tui|svg|png] [--theme NAME]
                        [--width N] [--height N] [--scale N] [--t MS]
                        [--dur S] [--fps N] [--state a,b]
                        [--env portrait,dark,coarse] [--set param=value]... [--plain]
  --state previews document-global states only; it cannot target one node.
`;
const GEN_USAGE = `usage: slab gen wc FILE -o DIR [--tag NAME] [--separate-ir]
       slab gen react FILE -o DIR [--tag NAME] [--separate-ir]
       slab gen rust FILE -o OUT.rs
`;
const DRIVE_USAGE = `usage: slab drive [FILE] [OPTIONS]

drive requires the native slab-cli:
  cargo install --git https://github.com/stencil-hq/slab slab-cli
`;

function b64(buf: Buffer): string {
   return buf.toString('base64');
}

/** Read image assets relative to the `.slab` file into a JSON map.
 *  `$slabSourceName` carries generator attribution when supplied. Missing
 *  files stay absent, so the compiler emits its normal warning. */
function assetsJsonFor(
   src: string,
   baseDir: string,
   sourcesJson: string,
   sourceName?: string,
): string {
   const W = wasm();
   let srcs: string[] = [];
   try {
      const value: unknown = JSON.parse(W.image_srcs_with_sources(src, sourcesJson));
      if (Array.isArray(value)) {
         srcs = value.filter((entry): entry is string => typeof entry === 'string');
      }
   } catch {
      srcs = [];
   }
   const map: Record<string, string> = {};
   if (sourceName !== undefined) map.$slabSourceName = sourceName;
   for (const source of srcs) {
      const path = join(baseDir, source);
      if (existsSync(path)) {
         map[source] = b64(readFileSync(path));
      }
   }
   return JSON.stringify(map);
}

type Diag = {
   level: 'error' | 'warning' | 'note';
   code: string;
   msg: string;
   line: number;
   remedy: string | null;
   formatted: string;
};

function printDiags(json: string): Diag[] {
   let diags: Diag[];
   try {
      diags = JSON.parse(json) as Diag[];
   } catch {
      // wasm threw a raw error string (not a diagnostics JSON) — surface it.
      process.stderr.write(`error: ${json}\n`);
      process.exit(1);
   }
   for (const d of diags) {
      process.stderr.write(`${d.formatted}\n`);
   }
   return diags;
}

function hasErrors(diags: Diag[]): boolean {
   return diags.some((d) => d.level === 'error');
}

function usageErr(msg: string): never {
   process.stderr.write(`error: ${msg}\n`);
   process.stderr.write(USAGE);
   process.exit(2);
}

/** Tiny flag parser matching the Rust `parse_args`. */
function parseArgs(
   args: string[],
   valueFlags: string[],
): {
   file?: string;
   out?: string;
   embedAssets: boolean;
   rest: string[];
} {
   const p = {
      file: undefined as string | undefined,
      out: undefined as string | undefined,
      embedAssets: true,
      rest: [] as string[],
   };
   const it = args[Symbol.iterator]();
   while (true) {
      const r = it.next();
      if (r.done) break;
      const a = r.value as string;
      if (a === '-o' || a === '--out') {
         const v = it.next();
         if (v.done) usageErr('missing value for -o');
         p.out = v.value as string;
      } else if (a === '--no-embed-assets') {
         p.embedAssets = false;
      } else if (a.startsWith('--')) {
         const name = a.slice(2);
         if (valueFlags.includes(name)) {
            const v = it.next();
            if (v.done) usageErr(`missing value for ${a}`);
            p.rest.push(a, v.value as string);
         } else {
            usageErr(`unknown flag ${a}`);
         }
      } else if (p.file === undefined) {
         p.file = a;
      } else {
         usageErr(`unexpected argument '${a}'`);
      }
   }
   return p;
}

function readSource(path: string): { src: string; baseDir: string } {
   let src: string;
   try {
      src = readFileSync(path, 'utf8');
   } catch (e) {
      usageErr(`cannot read ${path}: ${(e as Error).message}`);
   }
   return { src, baseDir: dirname(resolve(path)) };
}

// --- commands ----------------------------------------------------------------

function cmdCheck(args: string[]): void {
   const p = parseArgs(args, ['width', 'height', 'state', 'env', 'client', 'renderer']);
   if (!p.file) usageErr('check needs a FILE');
   const { src } = readSource(p.file);
   const sourcesJson = loadImports(resolve(p.file), src).sourcesJson;
   const W = wasm();
   const compilerVersion = W.compiler_version();
   process.stderr.write(
      `slab compiler ${compilerVersion} (package @stencil-hq/slab ${PACKAGE_VERSION})\n`,
   );
   const json = W.check_with_sources(src, p.file, sourcesJson);
   const diags = printDiags(json);
   if (hasErrors(diags)) process.exit(1);
   if (diags.length === 0) {
      process.stderr.write('ok\n');
   } else {
      process.stderr.write('ok with warnings\n');
   }
}

function cmdBuild(args: string[]): void {
   const p = parseArgs(args, []);
   if (!p.file || !p.out) usageErr('build needs FILE and -o OUT.slir');
   const { src, baseDir } = readSource(p.file);
   const sourcesJson = loadImports(resolve(p.file), src).sourcesJson;
   const W = wasm();
   const assetsJson = assetsJsonFor(src, baseDir, sourcesJson);
   let bytes: Uint8Array;
   try {
      bytes = W.build_with_sources(src, assetsJson, sourcesJson);
   } catch (e) {
      printDiags(String(e));
      process.exit(1);
   }
   // wasm-bindgen returns Uint8Array; write it directly.
   const buf = Buffer.from(bytes);
   writeFileSync(p.out, buf);
   process.stderr.write(`wrote ${p.out} (${buf.length} bytes)\n`);
}

function cmdDump(args: string[]): void {
   const p = parseArgs(args, []);
   if (!p.file) usageErr('dump needs a FILE.slir');
   let bytes: Buffer;
   try {
      bytes = readFileSync(p.file);
   } catch (e) {
      usageErr(`${p.file}: ${(e as Error).message}`);
   }
   const W = wasm();
   let text: string;
   try {
      text = W.dump(Buffer.from(bytes));
   } catch (e) {
      process.stderr.write(`error: ${String(e)}\n`);
      process.exit(1);
   }
   process.stdout.write(text);
}

/** Output kind from the -o extension (or `--client tui` for stdout). */
function kindOf(
   out: string | undefined,
   client: string | undefined,
): 'svg' | 'png' | 'apng' | 'tui' {
   if (out) {
      const ext = extname(out).slice(1);
      if (ext === 'svg' || ext === 'png' || ext === 'apng') return ext;
      if (ext === 'txt' || ext === 'ansi') return 'tui';
      usageErr(`cannot infer output kind from extension '.${ext}'`);
   }
   if (client === 'tui') return 'tui';
   usageErr('render needs -o OUT (or --client tui for stdout)');
}

function cmdRender(args: string[]): void {
   // render has its own flag set; parse inline to keep the Rust parity tight.
   const o: Record<string, unknown> = {
      file: undefined as string | undefined,
      out: undefined as string | undefined,
      client: undefined as string | undefined,
      width: 800,
      height: 0,
      scale: 1,
      t: 0,
      theme: undefined as string | undefined,
      dur: 2,
      fps: 20,
      states: [] as string[],
      env: [] as string[],
      sets: [] as [string, string][],
      plain: false,
   };
   const it = args[Symbol.iterator]();
   let file: string | undefined;
   while (true) {
      const r = it.next();
      if (r.done) break;
      const a = r.value as string;
      const val = (name: string): string => {
         const v = it.next();
         if (v.done) usageErr(`missing value for ${name}`);
         return v.value as string;
      };
      if (a === '-o' || a === '--out') o.out = val('-o');
      else if (a === '--client') o.client = val('--client');
      else if (a === '--theme') o.theme = val('--theme');
      else if (a === '--width') o.width = Number(val('--width'));
      else if (a === '--height') o.height = Number(val('--height'));
      else if (a === '--scale') o.scale = Number(val('--scale'));
      else if (a === '--t') o.t = Number(val('--t'));
      else if (a === '--dur') o.dur = Number(val('--dur'));
      else if (a === '--fps') o.fps = Number(val('--fps'));
      else if (a === '--state') (o.states as string[]).push(...val('--state').split(','));
      else if (a === '--env') (o.env as string[]).push(...val('--env').split(','));
      else if (a === '--set') {
         const v = val('--set');
         const eq = v.indexOf('=');
         if (eq < 0) usageErr('--set needs param=value');
         (o.sets as [string, string][]).push([v.slice(0, eq), v.slice(eq + 1)]);
      } else if (a === '--plain') o.plain = true;
      else if (a.startsWith('-')) usageErr(`unknown flag ${a}`);
      else if (!file) file = a;
      else usageErr(`unexpected argument '${a}'`);
   }
   if (!file) usageErr('render needs a FILE');
   const { src, baseDir } = readSource(file);
   const sourcesJson = loadImports(resolve(file), src).sourcesJson;
   const kind = kindOf(o.out as string | undefined, o.client as string | undefined);
   const W = wasm();
   const assetsJson = assetsJsonFor(src, baseDir, sourcesJson);
   const optsJson = JSON.stringify({ ...o, kind });
   let resultJson: string;
   try {
      resultJson = W.render_with_sources(src, optsJson, assetsJson, sourcesJson);
   } catch (e) {
      printDiags(String(e));
      process.exit(1);
   }
   const result = JSON.parse(resultJson) as {
      file: { name: string; b64?: string; text?: string };
      notes: string[];
      summary: string;
   };
   for (const n of result.notes) process.stderr.write(`${n}\n`);
   if (o.out) {
      const out = o.out as string;
      const bytes = result.file.b64
         ? Buffer.from(result.file.b64, 'base64')
         : Buffer.from(result.file.text ?? '', 'utf8');
      writeFileSync(out, bytes);
      process.stderr.write(`wrote ${out} (${bytes.length} bytes)${result.summary}\n`);
   } else {
      // TUI-to-stdout
      process.stdout.write(result.file.text ?? '');
   }
}

function cmdGenWc(args: string[]): void {
   let file: string | undefined;
   let out: string | undefined;
   let tag: string | undefined;
   let separateIr = false;
   const it = args[Symbol.iterator]();
   while (true) {
      const r = it.next();
      if (r.done) break;
      const a = r.value as string;
      if (a === '-o' || a === '--out') {
         const v = it.next();
         if (v.done) usageErr('missing value for -o');
         out = v.value as string;
      } else if (a === '--tag') {
         const v = it.next();
         if (v.done) usageErr('missing value for --tag');
         tag = v.value as string;
      } else if (a === '--separate-ir') separateIr = true;
      else if (a.startsWith('-')) usageErr(`unknown flag ${a}`);
      else if (!file) file = a;
      else usageErr(`unexpected argument '${a}'`);
   }
   if (!file || !out) usageErr('gen wc needs FILE and -o DIR');
   const { src, baseDir } = readSource(file);
   const sourcesJson = loadImports(resolve(file), src).sourcesJson;
   const stem =
      file
         .replace(/\.[^.]+$/, '')
         .split('/')
         .pop() ?? 'slab';
   const W = wasm();
   const assetsJson = assetsJsonFor(src, baseDir, sourcesJson);
   const optsJson = JSON.stringify({ tag, separateIr, stem, sourceName: file });
   let resultJson: string;
   try {
      resultJson = W.gen_wc_with_sources(src, optsJson, assetsJson, sourcesJson);
   } catch (e) {
      printDiags(String(e));
      process.exit(1);
   }
   const result = JSON.parse(resultJson) as {
      files: { name: string; b64?: string; text?: string }[];
      diagnostics: Diag[];
   };
   for (const d of result.diagnostics) process.stderr.write(`${d.formatted}\n`);
   if (result.diagnostics.some((d) => d.level === 'error')) process.exit(1);
   let nElems = 0;
   for (const f of result.files) {
      const bytes = f.b64 ? Buffer.from(f.b64, 'base64') : Buffer.from(f.text ?? '', 'utf8');
      const path = join(out, f.name);
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, bytes);
      if (f.name === `${stem}.js`) {
         nElems = (f.text ?? '').split('customElements.define').length - 1;
      }
   }
   process.stderr.write(
      `wrote ${join(out, `${stem}.js`)} + .d.ts + slab-runtime.js (${nElems} element${nElems === 1 ? '' : 's'})\n`,
   );
}

function cmdGenReact(args: string[]): void {
   let file: string | undefined;
   let out: string | undefined;
   let tag: string | undefined;
   let separateIr = false;
   const it = args[Symbol.iterator]();
   while (true) {
      const r = it.next();
      if (r.done) break;
      const a = r.value as string;
      if (a === '-o' || a === '--out') {
         const v = it.next();
         if (v.done) usageErr('missing value for -o');
         out = v.value as string;
      } else if (a === '--tag') {
         const v = it.next();
         if (v.done) usageErr('missing value for --tag');
         tag = v.value as string;
      } else if (a === '--separate-ir') separateIr = true;
      else if (a.startsWith('-')) usageErr(`unknown flag ${a}`);
      else if (!file) file = a;
      else usageErr(`unexpected argument '${a}'`);
   }
   if (!file || !out) usageErr('gen react needs FILE and -o DIR');
   const { src, baseDir } = readSource(file);
   const sourcesJson = loadImports(resolve(file), src).sourcesJson;
   const stem =
      file
         .replace(/\.[^.]+$/, '')
         .split('/')
         .pop() ?? 'slab';
   const W = wasm();
   const assetsJson = assetsJsonFor(src, baseDir, sourcesJson);
   const optsJson = JSON.stringify({ tag, separateIr, stem, sourceName: file });
   let resultJson: string;
   try {
      resultJson = W.gen_react_with_sources(src, optsJson, assetsJson, sourcesJson);
   } catch (e) {
      printDiags(String(e));
      process.exit(1);
   }
   const result = JSON.parse(resultJson) as {
      files: { name: string; b64?: string; text?: string }[];
      diagnostics: Diag[];
   };
   for (const d of result.diagnostics) process.stderr.write(`${d.formatted}\n`);
   if (result.diagnostics.some((d) => d.level === 'error')) process.exit(1);
   let nElems = 0;
   for (const f of result.files) {
      const bytes = f.b64 ? Buffer.from(f.b64, 'base64') : Buffer.from(f.text ?? '', 'utf8');
      const path = join(out, f.name);
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, bytes);
      if (f.name === `${stem}.js`) {
         nElems = (f.text ?? '').split('customElements.define').length - 1;
      }
   }
   process.stderr.write(
      `wrote ${join(out, `${stem}.tsx`)} + ${stem}.js + .d.ts + slab-runtime.js (${nElems} element${nElems === 1 ? '' : 's'})\n`,
   );
}

async function cmdDev(args: string[]): Promise<void> {
   let options: DevOptions;
   try {
      options = parseDevArgs(args);
   } catch (error) {
      if (error instanceof DevUsageError) usageErr(error.message);
      throw error;
   }

   const stem =
      options.file
         .replace(/\.[^.]+$/, '')
         .split(/[\\/]/)
         .pop() ?? 'slab';
   const generate = (): DevBuildResult => {
      const src = readFileSync(options.file, 'utf8');
      const baseDir = dirname(options.file);
      const sourcesJson = loadImports(resolve(options.file), src).sourcesJson;
      const assetsJson = assetsJsonFor(src, baseDir, sourcesJson);
      const optsJson = JSON.stringify({
         tag: options.tag,
         separateIr: options.separateIr,
         stem,
         sourceName: options.file,
      });
      let resultJson: string;
      try {
         resultJson = wasm().gen_wc_with_sources(src, optsJson, assetsJson, sourcesJson);
      } catch (error) {
         let message = String(error);
         try {
            const diagnostics = JSON.parse(message) as Diag[];
            if (Array.isArray(diagnostics)) {
               message = diagnostics.map((diagnostic) => diagnostic.formatted).join('\n');
            }
         } catch {
            // Keep a non-diagnostic WASM error unchanged.
         }
         throw new Error(message);
      }
      const result = JSON.parse(resultJson) as {
         files: { name: string; b64?: string; text?: string }[];
         diagnostics: Diag[];
      };
      return {
         files: result.files.map((file) => ({
            name: file.name,
            bytes: file.b64
               ? Buffer.from(file.b64, 'base64')
               : Buffer.from(file.text ?? '', 'utf8'),
            text: file.text,
         })),
         diagnostics: result.diagnostics.map((diagnostic) => diagnostic.formatted),
         hasErrors: result.diagnostics.some((diagnostic) => diagnostic.level === 'error'),
      };
   };

   const session = await startDevServer(options, generate);
   process.stdout.write(`${session.url}\n`);
   let stopping = false;
   const stop = (): void => {
      if (stopping) return;
      stopping = true;
      void session.close().then(
         () => process.exit(0),
         (error) => {
            process.stderr.write(`error: ${String(error)}\n`);
            process.exit(1);
         },
      );
   };
   process.once('SIGINT', stop);
   process.once('SIGTERM', stop);
}

function cmdGenRust(args: string[]): void {
   let file: string | undefined;
   let out: string | undefined;
   const it = args[Symbol.iterator]();
   while (true) {
      const r = it.next();
      if (r.done) break;
      const a = r.value as string;
      if (a === '-o' || a === '--out') {
         const v = it.next();
         if (v.done) usageErr('missing value for -o');
         out = v.value as string;
      } else if (a.startsWith('-')) usageErr(`unknown flag ${a}`);
      else if (!file) file = a;
      else usageErr(`unexpected argument '${a}'`);
   }
   if (!file || !out) usageErr('gen rust needs FILE and -o OUT.rs');
   const { src, baseDir } = readSource(file);
   const sourcesJson = loadImports(resolve(file), src).sourcesJson;
   const W = wasm();
   const assetsJson = assetsJsonFor(src, baseDir, sourcesJson, file);
   let resultJson: string;
   try {
      resultJson = W.gen_rust_with_sources(src, assetsJson, sourcesJson);
   } catch (e) {
      printDiags(String(e));
      process.exit(1);
   }
   const result = JSON.parse(resultJson) as { module: string; diagnostics: Diag[] };
   for (const d of result.diagnostics) process.stderr.write(`${d.formatted}\n`);
   if (result.diagnostics.some((d) => d.level === 'error')) process.exit(1);
   writeFileSync(out, result.module);
   process.stderr.write(`wrote ${out} (${result.module.length} bytes)\n`);
}

// --- dispatch ----------------------------------------------------------------

const args = process.argv.slice(2);
const cmd = args[0];
if (!cmd) {
   process.stderr.write(USAGE);
   process.exit(2);
}
const rest = args.slice(1);
if (
   cmd === 'gen' &&
   rest.length === 2 &&
   (rest[1] === '--help' || rest[1] === '-h') &&
   (rest[0] === 'wc' || rest[0] === 'react' || rest[0] === 'rust')
) {
   process.stdout.write(
      rest[0] === 'wc'
         ? 'usage: slab gen wc FILE -o DIR [--tag NAME] [--separate-ir]\n'
         : rest[0] === 'react'
           ? 'usage: slab gen react FILE -o DIR [--tag NAME] [--separate-ir]\n'
           : 'usage: slab gen rust FILE -o OUT.rs\n',
   );
   process.exit(0);
}
const help = rest.length === 1 && (rest[0] === '--help' || rest[0] === '-h');
if (help) {
   const usage = {
      check: CHECK_USAGE,
      build: BUILD_USAGE,
      dump: DUMP_USAGE,
      render: RENDER_USAGE,
      gen: GEN_USAGE,
      dev: DEV_USAGE,
      drive: DRIVE_USAGE,
   }[cmd];
   if (usage !== undefined) {
      process.stdout.write(usage);
      process.exit(0);
   }
}
switch (cmd) {
   case 'check':
      cmdCheck(rest);
      break;
   case 'build':
      cmdBuild(rest);
      break;
   case 'dump':
      cmdDump(rest);
      break;
   case 'render':
      cmdRender(rest);
      break;
   case 'dev':
      await cmdDev(rest);
      break;
   case 'gen':
      if (rest[0] === 'wc') cmdGenWc(rest.slice(1));
      else if (rest[0] === 'react') cmdGenReact(rest.slice(1));
      else if (rest[0] === 'rust') cmdGenRust(rest.slice(1));
      else if (rest[0]) usageErr(`unknown gen target '${rest[0]}'`);
      else usageErr('gen needs a target (wc, react, rust)');
      break;
   case 'drive':
      process.stderr.write(DRIVE_USAGE);
      process.exit(2);
      break;
   case '--help':
   case '-h':
   case 'help':
      process.stdout.write(USAGE);
      break;
   default:
      process.stderr.write(`error: unknown command '${cmd}'\n`);
      process.stderr.write(USAGE);
      process.exit(2);
}
