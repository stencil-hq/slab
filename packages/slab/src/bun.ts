// Bun plugin: import `.slab` files at runtime (`Bun.plugin`) or in
// `Bun.build`. No HMR footer — Bun's dev server does a full reload.

import { readFileSync } from 'node:fs';
import type { BunPlugin } from 'bun';
import { compileSlab, type SlabPluginOptions, writeDeclaration } from './core.ts';

export type { SlabPluginOptions } from './core.ts';

/** Bun plugin factory: compile `.slab` imports via the Slab WASM compiler. */
export default function slab(options: SlabPluginOptions = {}): BunPlugin {
   const declarations = options.declarations ?? true;
   return {
      name: 'slab',
      setup(build) {
         build.onLoad({ filter: /\.slab$/ }, (args) => {
            const source = readFileSync(args.path, 'utf8');
            const mod = compileSlab(args.path, source);
            if (declarations) writeDeclaration(args.path, mod.declaration);
            return {
               contents: mod.code,
               loader: 'js',
               watchFiles: [args.path, ...mod.imports, ...mod.assets],
            };
         });
      },
   };
}
