// Shared core for the Vite and Bun `.slab` plugins: compile one source file
// through the WASM compiler, rewrite the generated module onto the published
// runtime package, and derive the sibling `<stem>.d.slab.ts` declaration
// TypeScript resolves for `.slab` imports (tsconfig
// `allowArbitraryExtensions`).

import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, resolve } from 'node:path';
import { wasm } from './wasm.ts';

/** Exact runtime import emitted by `slab gen wc` (a sidecar file we do not ship). */
const RUNTIME_IMPORT = "import { SlabElement } from './slab-runtime.js';";
/** Replacement import resolving the same runtime through the published package. */
const WSLAB_IMPORT = "import { SlabElement } from '@stencil-hq/wslab';";
/** Exact runtime re-export emitted by `slab gen wc`. */
const RUNTIME_EXPORT = "export { itemKey } from './slab-runtime.js';";
/** Replacement re-export resolving through the published package. */
const WSLAB_EXPORT = "export { itemKey } from '@stencil-hq/wslab';";

/** Imported source text and file dependencies for one root document. */
export interface SlabImports {
   /** Normalized import-key to source-text JSON map for the WASM compiler. */
   sourcesJson: string;
   /** Absolute imported source paths for file watchers. */
   paths: string[];
}

function jsonStrings(json: string): string[] {
   try {
      const value: unknown = JSON.parse(json);
      return Array.isArray(value)
         ? value.filter((entry): entry is string => typeof entry === 'string')
         : [];
   } catch {
      return [];
   }
}

function importKey(importer: string | undefined, path: string): string {
   const parts =
      importer
         ?.split('/')
         .slice(0, -1)
         .filter((part) => part.length > 0) ?? [];
   for (const part of path.split('/')) {
      if (part === '' || part === '.') continue;
      if (part === '..' && parts.at(-1) !== undefined && parts.at(-1) !== '..') {
         parts.pop();
      } else {
         parts.push(part);
      }
   }
   return parts.join('/');
}

/** Load every reachable import from disk, with keys matching the Slab compiler. */
export function loadImports(file: string, source: string): SlabImports {
   const W = wasm();
   const baseDir = dirname(resolve(file));
   const sources: Record<string, string> = {};
   const paths: string[] = [];
   const seen = new Set<string>();
   const pending: { key: string | undefined; source: string }[] = [{ key: undefined, source }];
   for (let index = 0; index < pending.length; index++) {
      const current = pending[index];
      if (current === undefined) continue;
      for (const path of jsonStrings(W.import_paths(current.source))) {
         const key = importKey(current.key, path);
         if (seen.has(key)) continue;
         seen.add(key);
         const absolute = resolve(baseDir, key);
         let imported: string;
         try {
            imported = readFileSync(absolute, 'utf8');
         } catch {
            continue;
         }
         sources[key] = imported;
         paths.push(absolute);
         pending.push({ key, source: imported });
      }
   }
   return { sourcesJson: JSON.stringify(sources), paths };
}

/** One compiler diagnostic as surfaced by the WASM compiler. */
export interface SlabDiagnostic {
   level: 'error' | 'warning' | 'note';
   code: string;
   msg: string;
   line: number;
   remedy: string | null;
   formatted: string;
}

/** A custom element defined by a generated module. */
export interface SlabElementTag {
   /** The `customElements.define` tag name. */
   tag: string;
   /** The exported class identifier bound to the tag. */
   className: string;
}

/** One external binary emitted beside a generated JavaScript module. */
export interface SlabSidecar {
   /** Module-relative output name. */
   name: string;
   /** Compiled binary contents. */
   bytes: Uint8Array;
}

/** Everything a bundler needs from one compiled `.slab` module. */
export interface SlabModule {
   /** ES module text with the runtime import rewritten to `@stencil-hq/wslab`. */
   code: string;
   /** Raw `gen wc` declaration text (`<stem>.d.ts`). */
   dts: string;
   /** `<stem>.d.slab.ts` content derived from {@link SlabModule.dts}. */
   declaration: string;
   /** External SLIR files referenced by {@link SlabModule.code}. */
   sidecars: SlabSidecar[];
   /** Absolute paths of the image assets read into the compile (watch these). */
   assets: string[];
   /** Absolute paths of imported Slab modules read into the compile. */
   imports: string[];
   /** Custom elements the module defines, in definition order. */
   tags: SlabElementTag[];
   /** Non-error diagnostics from the compile. */
   warnings: SlabDiagnostic[];
}

/** Options accepted by both the Vite and Bun plugin factories. */
export interface SlabPluginOptions {
   /** Write a sibling `<stem>.d.slab.ts` next to each imported `.slab` (default true). */
   declarations?: boolean;
}

/** Compile failure carrying the compiler's formatted diagnostic lines. */
export class SlabCompileError extends Error {
   /** Every diagnostic from the failed compile, errors included. */
   readonly diagnostics: SlabDiagnostic[];

   constructor(file: string, diagnostics: SlabDiagnostic[]) {
      const lines = diagnostics.map((d) => d.formatted).filter((line) => line.length > 0);
      super(`slab: compiling ${file} failed\n${lines.join('\n')}`);
      this.name = 'SlabCompileError';
      this.diagnostics = diagnostics;
   }
}

/** Parse a diagnostics payload thrown by the WASM compiler (raw string fallback). */
function parseDiagnostics(raw: string): SlabDiagnostic[] {
   try {
      const parsed: unknown = JSON.parse(raw);
      if (Array.isArray(parsed)) return parsed as SlabDiagnostic[];
   } catch {
      // Not diagnostics JSON — surface the raw message below.
   }
   return [{ level: 'error', code: '', msg: raw, line: 0, remedy: null, formatted: raw }];
}

/** Collect referenced image assets relative to the `.slab` file. Missing files
 * stay absent so the compiler emits its own warning. */
function collectAssets(
   source: string,
   baseDir: string,
   sourcesJson: string,
): { assetsJson: string; paths: string[] } {
   const W = wasm();
   const srcs = jsonStrings(W.image_srcs_with_sources(source, sourcesJson));
   const map: Record<string, string> = {};
   const paths: string[] = [];
   for (const src of srcs) {
      const path = resolve(baseDir, src);
      if (!existsSync(path)) continue;
      map[src] = readFileSync(path).toString('base64');
      paths.push(path);
   }
   return { assetsJson: JSON.stringify(map), paths };
}

/** Convert `gen wc` declaration text into `<stem>.d.slab.ts` content. */
export function toDeclaration(dts: string): string {
   const body = dts.replace(/^\/\/ GENERATED by[^\n]*\n/, '');
   return `// GENERATED by @stencil-hq/slab-plugin — do not edit.\n${body}`;
}

/** The sibling declaration path for a `.slab` file (`app.slab` → `app.d.slab.ts`). */
export function declarationPath(file: string): string {
   return file.replace(/\.slab$/, '.d.slab.ts');
}

/** Write the sibling declaration, skipping byte-identical content. Returns
 * true when the file changed on disk. */
export function writeDeclaration(file: string, declaration: string): boolean {
   const path = declarationPath(file);
   try {
      if (readFileSync(path, 'utf8') === declaration) return false;
   } catch {
      // Missing or unreadable — write below.
   }
   writeFileSync(path, declaration);
   return true;
}

/** Rewrite generated `new URL` expressions to bundler-owned asset imports. */
export function withSlirImports(
   code: string,
   sidecars: readonly SlabSidecar[],
   specifier: (sidecar: SlabSidecar, index: number) => string,
): string {
   const imports: string[] = [];
   let rewritten = code;
   for (const [index, sidecar] of sidecars.entries()) {
      const binding = `__slabSlir${index}`;
      const expression = `new URL('./${sidecar.name}', import.meta.url).href`;
      if (!rewritten.includes(expression)) {
         throw new Error(`slab: generated module is missing the URL for ${sidecar.name}`);
      }
      imports.push(`import ${binding} from ${JSON.stringify(specifier(sidecar, index))};`);
      rewritten = rewritten.replace(expression, binding);
   }
   return `${imports.join('\n')}\n${rewritten}`;
}

/** Self-accepting Vite HMR footer that fetches each updated SLIR sidecar and
 * swaps it through the stable registered element class. */
export function hmrFooter(tags: SlabElementTag[]): string {
   const pairs = tags.map((t) => `['${t.tag}', ${t.className}]`).join(', ');
   return `
if (import.meta.hot) {
   import.meta.hot.accept();
   void (async () => {
      for (const [tag, next] of [${pairs}]) {
         const current = customElements.get(tag);
         if (current === undefined || current === next) continue;
         const response = await fetch(next.slir, { cache: 'no-store' });
         if (!response.ok) throw new Error(\`slab: fetching SLIR failed (\${response.status})\`);
         const bytes = new Uint8Array(await response.arrayBuffer());
         current.hotReplaceSlir(bytes);
         for (const el of document.querySelectorAll(tag)) el.loadSlir(bytes);
      }
   })();
}
`;
}

/** Compile one `.slab` source into a bundler-ready ES module. Throws
 * {@link SlabCompileError} when the compile reports errors. */
export function compileSlab(file: string, source: string): SlabModule {
   const W = wasm();
   const stem = basename(file).replace(/\.[^.]+$/, '') || 'slab';
   const baseDir = dirname(resolve(file));
   const imports = loadImports(file, source);
   const { assetsJson, paths } = collectAssets(source, baseDir, imports.sourcesJson);
   const optsJson = JSON.stringify({ stem, sourceName: file });
   let resultJson: string;
   try {
      resultJson = W.gen_wc_with_sources(source, optsJson, assetsJson, imports.sourcesJson);
   } catch (error) {
      throw new SlabCompileError(file, parseDiagnostics(String(error)));
   }
   const result = JSON.parse(resultJson) as {
      files: { name: string; b64?: string; text?: string }[];
      diagnostics: SlabDiagnostic[];
   };
   if (result.diagnostics.some((d) => d.level === 'error')) {
      throw new SlabCompileError(file, result.diagnostics);
   }
   const moduleText = result.files.find((f) => f.name === `${stem}.js`)?.text;
   const dts = result.files.find((f) => f.name === `${stem}.d.ts`)?.text;
   if (moduleText === undefined || dts === undefined) {
      throw new Error(`slab: gen_wc returned no ${stem}.js/${stem}.d.ts for ${file}`);
   }
   const sidecars = result.files
      .filter((generated) => generated.name.endsWith('.slir'))
      .map((generated): SlabSidecar => {
         if (generated.b64 === undefined) {
            throw new Error(`slab: gen_wc returned non-binary ${generated.name}`);
         }
         return { name: generated.name, bytes: Buffer.from(generated.b64, 'base64') };
      });
   if (!moduleText.includes(RUNTIME_IMPORT) || !moduleText.includes(RUNTIME_EXPORT)) {
      throw new Error(`slab: generated module for ${file} is missing the runtime import`);
   }
   const code = moduleText
      .replace(RUNTIME_IMPORT, WSLAB_IMPORT)
      .replace(RUNTIME_EXPORT, WSLAB_EXPORT);
   const tags: SlabElementTag[] = [];
   for (const m of code.matchAll(/customElements\.define\('([^']+)', ([A-Za-z_$][\w$]*)\)/g)) {
      tags.push({ tag: m[1] as string, className: m[2] as string });
   }
   return {
      code,
      dts,
      declaration: toDeclaration(dts),
      assets: paths,
      sidecars,
      imports: imports.paths,
      tags,
      warnings: result.diagnostics,
   };
}
