import { afterEach, describe, expect, test } from 'bun:test';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { BunPlugin, PluginBuilder } from 'bun';
import slab from '../src/bun.ts';

const FIXTURE = fileURLToPath(new URL('./fixtures/hello.slab', import.meta.url));

const roots: string[] = [];
afterEach(() => {
   for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

type OnLoadArgs = { path: string };
type OnLoadResult = { contents: string | Uint8Array; loader: string };
type OnLoadCallback = (args: OnLoadArgs) => OnLoadResult;
type OnResolveCallback = (args: OnLoadArgs) => { path: string; namespace: string };

interface CapturedPlugin {
   filter: RegExp;
   onLoad: OnLoadCallback;
   resolveSlir: OnResolveCallback;
   loadSlir: OnLoadCallback;
}

// The generated module (and `@stencil-hq/wslab` behind it) touches
// `HTMLElement`/`customElements` at import time, so a DOM-less `bun test`
// cannot evaluate it; assert the plugin callbacks instead of faking a DOM.
function capture(plugin: BunPlugin): CapturedPlugin {
   let filter: RegExp | undefined;
   let onLoad: OnLoadCallback | undefined;
   let resolveSlir: OnResolveCallback | undefined;
   let loadSlir: OnLoadCallback | undefined;
   const build = {
      onResolve(_constraints: { filter: RegExp }, callback: OnResolveCallback) {
         resolveSlir = callback;
      },
      onLoad(constraints: { filter: RegExp; namespace?: string }, callback: OnLoadCallback) {
         if (constraints.namespace === 'slab-slir') {
            loadSlir = callback;
         } else {
            filter = constraints.filter;
            onLoad = callback;
         }
      },
   };
   plugin.setup(build as unknown as PluginBuilder);
   if (!filter || !onLoad || !resolveSlir || !loadSlir) {
      throw new Error('plugin did not register every Slab loader');
   }
   return { filter, onLoad, resolveSlir, loadSlir };
}

describe('bun plugin', () => {
   test('registers an onLoad handler for .slab files', () => {
      const plugin = slab();
      expect(plugin.name).toBe('slab');
      const { filter } = capture(plugin);
      expect(filter.test('/x/app.slab')).toBe(true);
      expect(filter.test('/x/app.ts')).toBe(false);
   });

   test('onLoad returns the rewritten web-component module as js', () => {
      const captured = capture(slab({ declarations: false }));
      const result = captured.onLoad({ path: FIXTURE });
      expect(result.loader).toBe('js');
      expect(result.contents).toContain("import { SlabElement } from '@stencil-hq/wslab';");
      expect(result.contents).toContain('export class SlabHelloElement extends SlabElement');
      expect(result.contents).toContain("if (!customElements.get('slab-hello'))");
      expect(result.contents).not.toContain('import.meta.hot');
      const specifier = String(result.contents).match(/from "(slab-slir:[^"]+)";/)?.[1];
      expect(specifier).toBeDefined();
      expect(specifier).not.toContain('%00');
      expect(specifier).toContain(':hello.slir');
      const resolved = captured.resolveSlir({ path: specifier ?? '' });
      const asset = captured.loadSlir({ path: resolved.path });
      expect(resolved.namespace).toBe('slab-slir');
      expect(asset.loader).toBe('file');
      expect(Buffer.from(asset.contents).subarray(0, 4).toString()).toBe('SLIR');
   });

   test('writes the sibling declaration by default and skips it when disabled', () => {
      const root = mkdtempSync(join(tmpdir(), 'slab-plugin-bun-'));
      roots.push(root);
      const file = join(root, 'app.slab');
      writeFileSync(file, readFileSync(FIXTURE));
      capture(slab({ declarations: false })).onLoad({ path: file });
      expect(existsSync(join(root, 'app.d.slab.ts'))).toBe(false);
      capture(slab()).onLoad({ path: file });
      const decl = readFileSync(join(root, 'app.d.slab.ts'), 'utf8');
      expect(decl).toContain('export declare class SlabAppElement');
   });
});
