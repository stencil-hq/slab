// Vite plugin: import `.slab` files as web-component ES modules with honest
// dev-server HMR (SLIR bytes swap through the stable registered element).

import { readFileSync } from 'node:fs';
import type { Plugin } from 'vite';
import { compileSlab, hmrFooter, type SlabPluginOptions, writeDeclaration } from './core.ts';

export type { SlabPluginOptions } from './core.ts';

/** Vite plugin factory: compile `.slab` imports via the Slab WASM compiler. */
export default function slab(options: SlabPluginOptions = {}): Plugin {
   const declarations = options.declarations ?? true;
   let serve = false;
   return {
      name: 'slab',
      configResolved(config) {
         serve = config.command === 'serve';
      },
      load(id) {
         const file = id.replace(/[?#].*$/, '');
         if (!file.endsWith('.slab')) return null;
         const source = readFileSync(file, 'utf8');
         const mod = compileSlab(file, source);
         this.addWatchFile(file);
         for (const asset of mod.assets) this.addWatchFile(asset);
         for (const warning of mod.warnings) this.warn(warning.formatted);
         if (declarations) writeDeclaration(file, mod.declaration);
         const code = serve ? mod.code + hmrFooter(mod.tags) : mod.code;
         return { code, map: null };
      },
   };
}
