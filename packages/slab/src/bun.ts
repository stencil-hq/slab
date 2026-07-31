// Bun plugin: import `.slab` files at runtime (`Bun.plugin`) or in
// `Bun.build`. No HMR footer — Bun's dev server does a full reload.

import { readFileSync } from 'node:fs';
import type { BunPlugin } from 'bun';
import {
   compileSlab,
   type SlabPluginOptions,
   type SlabSidecar,
   withSlirImports,
   writeDeclaration,
} from './core.ts';

export type { SlabPluginOptions } from './core.ts';

const VIRTUAL_SLIR = 'slab-slir:';

function sidecarToken(file: string, name: string): string {
   return `${encodeURIComponent(file)}:${encodeURIComponent(name)}`;
}

/** Bun plugin factory: compile `.slab` imports via the Slab WASM compiler. */
export default function slab(options: SlabPluginOptions = {}): BunPlugin {
   const declarations = options.declarations ?? true;
   const sidecars = new Map<string, SlabSidecar>();
   return {
      name: 'slab',
      setup(build) {
         build.onResolve({ filter: /^slab-slir:/ }, (args) => ({
            path: args.path.slice(VIRTUAL_SLIR.length),
            namespace: 'slab-slir',
         }));
         build.onLoad({ filter: /.*/, namespace: 'slab-slir' }, (args) => {
            const sidecar = sidecars.get(args.path);
            if (sidecar === undefined) throw new Error(`slab: missing virtual SLIR ${args.path}`);
            return { contents: sidecar.bytes, loader: 'file' };
         });
         build.onLoad({ filter: /\.slab$/ }, (args) => {
            const source = readFileSync(args.path, 'utf8');
            const mod = compileSlab(args.path, source);
            if (declarations) writeDeclaration(args.path, mod.declaration);
            return {
               contents: withSlirImports(mod.code, mod.sidecars, (sidecar) => {
                  const path = `${sidecarToken(args.path, sidecar.name)}/${sidecar.name}`;
                  sidecars.set(path, sidecar);
                  return `${VIRTUAL_SLIR}${path}`;
               }),
               loader: 'js',
               watchFiles: [args.path, ...mod.imports, ...mod.assets],
            };
         });
      },
   };
}
