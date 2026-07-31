// Vite plugin: import `.slab` files as web-component ES modules with honest
// dev-server HMR (SLIR bytes swap through the stable registered element).

import { readFileSync } from 'node:fs';
import type { Plugin } from 'vite';
import {
   compileSlab,
   hmrFooter,
   type SlabPluginOptions,
   type SlabSidecar,
   withSlirImports,
   writeDeclaration,
} from './core.ts';

export type { SlabPluginOptions } from './core.ts';

const VIRTUAL_SLIR = 'virtual:slab-slir/';
const RESOLVED_SLIR = '\0slab-slir:';
const SERVED_SLIR = '/@slab-slir/';

function sidecarToken(file: string, name: string): string {
   return `${encodeURIComponent(file)}:${encodeURIComponent(name)}`;
}

/** Vite plugin factory: compile `.slab` imports via the Slab WASM compiler. */
export default function slab(options: SlabPluginOptions = {}): Plugin {
   const declarations = options.declarations ?? true;
   const sidecars = new Map<string, SlabSidecar>();
   let serve = false;
   return {
      name: 'slab',
      configResolved(config) {
         serve = config.command === 'serve';
      },
      configureServer(server) {
         server.middlewares.use((request, response, next) => {
            const path = request.url?.split('?', 1)[0];
            if (!path?.startsWith(SERVED_SLIR)) {
               next();
               return;
            }
            const sidecar = sidecars.get(path.slice(SERVED_SLIR.length));
            if (sidecar === undefined) {
               next();
               return;
            }
            response.statusCode = 200;
            response.setHeader('Content-Type', 'application/octet-stream');
            response.end(sidecar.bytes);
         });
      },
      resolveId(source) {
         return source.startsWith(VIRTUAL_SLIR)
            ? `${RESOLVED_SLIR}${source.slice(VIRTUAL_SLIR.length)}`
            : null;
      },
      load(id) {
         if (id.startsWith(RESOLVED_SLIR)) {
            const token = id.slice(RESOLVED_SLIR.length);
            const sidecar = sidecars.get(token);
            if (sidecar === undefined) throw new Error(`slab: missing virtual SLIR ${token}`);
            if (serve) return `export default ${JSON.stringify(`${SERVED_SLIR}${token}`)};`;
            const reference = this.emitFile({
               type: 'asset',
               name: sidecar.name,
               source: sidecar.bytes,
            });
            return `export default import.meta.ROLLUP_FILE_URL_${reference};`;
         }

         const file = id.replace(/[?#].*$/, '');
         if (!file.endsWith('.slab')) return null;
         const source = readFileSync(file, 'utf8');
         const mod = compileSlab(file, source);
         this.addWatchFile(file);
         for (const asset of mod.assets) this.addWatchFile(asset);
         for (const imported of mod.imports) this.addWatchFile(imported);
         for (const warning of mod.warnings) this.warn(warning.formatted);
         if (declarations) writeDeclaration(file, mod.declaration);
         const module = withSlirImports(mod.code, mod.sidecars, (sidecar) => {
            const token = sidecarToken(file, sidecar.name);
            sidecars.set(token, sidecar);
            return `${VIRTUAL_SLIR}${token}`;
         });
         const code = serve ? module + hmrFooter(mod.tags) : module;
         return { code, map: null };
      },
   };
}
