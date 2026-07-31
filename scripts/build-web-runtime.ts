// Builds the browser runtime embedded by slab-compile without invalidating Cargo on no-op runs.

import { mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';

import { writeGenerated } from './generation.ts';

const ROOT = resolve(import.meta.dirname, '..');
const destination = join(ROOT, 'gen/web-runtime/slab-runtime.js');
const build = await Bun.build({
   entrypoints: [join(ROOT, 'clients/web/index.ts')],
   format: 'esm',
   target: 'browser',
   minify: true,
   conditions: ['bun'],
});

if (!build.success) {
   for (const log of build.logs) console.error(log);
   process.exit(1);
}
const [artifact] = build.outputs;
if (build.outputs.length !== 1 || !artifact || artifact.kind !== 'entry-point') {
   console.error(`web-runtime: expected one entry-point, got ${build.outputs.length} outputs`);
   process.exit(1);
}

const bytes = new Uint8Array(await artifact.arrayBuffer());
mkdirSync(dirname(destination), { recursive: true });
const changed = writeGenerated(destination, bytes);
console.log(
   `web-runtime: ${changed ? 'wrote' : 'unchanged'} ${destination} (${bytes.length} bytes)`,
);
