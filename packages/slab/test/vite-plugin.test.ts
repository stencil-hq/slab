import { afterEach, describe, expect, test } from 'bun:test';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import type { Plugin, ResolvedConfig } from 'vite';
import slab from '../src/vite.ts';

const FIXTURE = fileURLToPath(new URL('./fixtures/hello.slab', import.meta.url));

const roots: string[] = [];
afterEach(() => {
   for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

/** Minimal Rollup plugin context capturing watch files and warnings. */
class Context {
   watched: string[] = [];
   warnings: string[] = [];
   addWatchFile(id: string): void {
      this.watched.push(id);
   }
   warn(message: string): void {
      this.warnings.push(message);
   }
}

function run(
   plugin: Plugin,
   id: string,
   command: 'build' | 'serve',
): { result: { code: string } | null; context: Context } {
   const configResolved = plugin.configResolved;
   if (typeof configResolved !== 'function') throw new Error('configResolved hook missing');
   configResolved.call(undefined as never, { command } as ResolvedConfig);
   const load = plugin.load;
   if (typeof load !== 'function') throw new Error('load hook missing');
   const context = new Context();
   const result = load.call(context as never, id) as { code: string } | null;
   return { result, context };
}

function fixtureCopy(): { root: string; file: string } {
   const root = mkdtempSync(join(tmpdir(), 'slab-plugin-vite-'));
   roots.push(root);
   const file = join(root, 'app.slab');
   writeFileSync(file, readFileSync(FIXTURE));
   return { root, file };
}

describe('vite plugin', () => {
   test('ignores non-slab ids', () => {
      const { result } = run(slab({ declarations: false }), '/x/app.ts', 'serve');
      expect(result).toBeNull();
   });

   test('serve: emits the rewritten module with a self-accepting HMR footer', () => {
      const { result, context } = run(slab({ declarations: false }), `${FIXTURE}?import`, 'serve');
      expect(result?.code).toContain("import { SlabElement } from '@stencil-hq/wslab';");
      expect(result?.code).toContain('import.meta.hot.accept();');
      expect(result?.code).toContain("customElements.get('slab-hello')");
      expect(result?.code).toContain('hotReplaceSlir');
      expect(context.watched).toEqual([FIXTURE]);
   });

   test('build: emits the module without the HMR footer', () => {
      const { result } = run(slab({ declarations: false }), FIXTURE, 'build');
      expect(result?.code).toContain('export class SlabHelloElement extends SlabElement');
      expect(result?.code).not.toContain('import.meta.hot');
   });

   test('writes the sibling declaration by default', () => {
      const { root, file } = fixtureCopy();
      run(slab(), file, 'build');
      const decl = readFileSync(join(root, 'app.d.slab.ts'), 'utf8');
      expect(decl).toContain('export declare class SlabAppElement');
      expect(existsSync(join(root, 'app.d.slab.ts'))).toBe(true);
   });
});
