// `bun scripts/serve.ts [dir]` — static file server for local dev.
// Serves site/dist by default; port 0 picks a free port and prints the URL.

import { relative, resolve } from 'node:path';
import Bun from 'bun';
import { resolveStatic } from './static.ts';

const ROOT = resolve(import.meta.dirname, '..');
const DIST = resolve(ROOT, process.argv[2] ?? 'site/dist');

const server = Bun.serve({
   port: 0,
   fetch: async (req) => {
      const url = new URL(req.url);
      const hit = await resolveStatic(DIST, url.pathname);
      if (!hit) return new Response('not found', { status: 404 });
      return new Response(hit.file, { headers: { 'content-type': hit.mime } });
   },
});
console.log(`serving ${relative(ROOT, DIST)}: http://localhost:${server.port}/`);
