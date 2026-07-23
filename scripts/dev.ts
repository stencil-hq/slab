// `just dev` — playground dev loop with live reload.
//
// Builds site/dist once, then watches site/, clients/web/, and examples/;
// every change reruns site-build and reloads connected browser tabs over a
// `/__dev` SSE channel injected into served HTML. A failed rebuild keeps the
// last good dist and logs the error; the tab reloads on the next green build.
//
// Run `just site` once first so the compiler/kernel wasm is already in
// site/dist (this loop never rebuilds wasm).

import { readdirSync, readFileSync, statSync, watch } from 'node:fs';
import { join, resolve } from 'node:path';
import Bun from 'bun';
import { resolveStatic } from './static.ts';

const ROOT = resolve(import.meta.dirname, '..');
const DIST = join(ROOT, 'site/dist');
const WATCH_DIRS = ['site', 'clients/web', 'examples'];

const RELOAD_SNIPPET =
   "<script>new EventSource('/__dev').onmessage = () => location.reload();</script>";

async function build(): Promise<boolean> {
   const proc = Bun.spawn(['bun', 'scripts/site-build.ts'], {
      cwd: ROOT,
      stdout: 'inherit',
      stderr: 'inherit',
   });
   return (await proc.exited) === 0;
}

/** Content digest of site/dist — reloads only fire when output truly changed
 * (macOS FSEvents replays stale events; rebuilds from those are no-ops).
 * Entries can vanish mid-scan while a build replaces them; they land in the
 * next digest. */
function distDigest(): string {
   const hasher = new Bun.CryptoHasher('md5');
   for (const entry of readdirSync(DIST, { recursive: true, encoding: 'utf8' }).sort()) {
      const path = join(DIST, entry);
      if (!statSync(path, { throwIfNoEntry: false })?.isFile()) continue;
      hasher.update(entry);
      hasher.update(readFileSync(path));
   }
   return hasher.digest('hex');
}

// ── SSE reload channel ───────────────────────────────────────────────

const encoder = new TextEncoder();
const clients = new Set<ReadableStreamDefaultController<Uint8Array>>();

/** Push one SSE frame to every client, dropping closed connections. */
function broadcast(frame: string): void {
   for (const client of [...clients]) {
      try {
         client.enqueue(encoder.encode(frame));
      } catch {
         clients.delete(client);
      }
   }
}

// Keepalive comments so proxies and Bun's idle accounting never see a
// silent stream (idleTimeout is disabled below, but pings also reap
// controllers whose tabs are gone).
setInterval(() => broadcast(': ping\n\n'), 15_000);

function sseResponse(): Response {
   let ctrl: ReadableStreamDefaultController<Uint8Array> | null = null;
   const stream = new ReadableStream<Uint8Array>({
      start(controller) {
         ctrl = controller;
         clients.add(controller);
         controller.enqueue(encoder.encode(': connected\n\n'));
      },
      cancel() {
         if (ctrl) clients.delete(ctrl);
      },
   });
   return new Response(stream, {
      headers: { 'content-type': 'text/event-stream', 'cache-control': 'no-store' },
   });
}

// ── watch → debounced rebuild → reload ───────────────────────────────

let pending: NodeJS.Timeout | undefined;
let building = false;
let dirty = false;

let lastDigest = '';

function schedule(name: string): void {
   clearTimeout(pending);
   pending = setTimeout(async () => {
      if (building) {
         dirty = true;
         return;
      }
      building = true;
      console.log(`dev: ${name} changed — rebuilding`);
      const ok = await build();
      building = false;
      if (dirty) {
         dirty = false;
         schedule('(queued changes)');
         return;
      }
      if (!ok) return;
      const digest = distDigest();
      if (digest === lastDigest) return;
      lastDigest = digest;
      broadcast('data: reload\n\n');
   }, 80);
}

/** Copying the kernel wasm into site/dist clones it on APFS, which makes
 * FSEvents report the SOURCE file as changed — every build would retrigger
 * itself. Generated-artifact events only count when size or mtime moved. */
const statCache = new Map<string, string>();
function reallyChanged(abs: string): boolean {
   try {
      const s = statSync(abs);
      const sig = `${s.size}:${s.mtimeMs}`;
      if (statCache.get(abs) === sig) return false;
      statCache.set(abs, sig);
      return true;
   } catch {
      return true;
   }
}

for (const dir of WATCH_DIRS) {
   watch(join(ROOT, dir), { recursive: true }, (_event, filename) => {
      const name = filename ?? '';
      // site/dist is our own output — never rebuild on it.
      if (dir === 'site' && (name === 'dist' || name.startsWith('dist/'))) return;
      // generated wasm artifacts: ignore clone-echo events from our own build
      if (name.startsWith('wasm/') && !reallyChanged(join(ROOT, dir, name))) return;
      // editor/sed temp files and other dotfiles never affect the build
      const base = name.slice(name.lastIndexOf('/') + 1);
      if (base.startsWith('.')) return;
      schedule(`${dir}/${name}`);
   });
}

// ── server ───────────────────────────────────────────────────────────

// Prime the clone-echo gate so the initial build's kernel-wasm copy is
// silent instead of costing one extra rebuild.
try {
   for (const f of readdirSync(join(ROOT, 'clients/web/wasm'))) {
      reallyChanged(join(ROOT, 'clients/web/wasm', f));
   }
} catch {
   // site-build reports the missing wasm with a real error message.
}

if (!(await build())) process.exit(1);
lastDigest = distDigest();

const server = Bun.serve({
   port: 0,
   // SSE reload channel must outlive the 10s default idle timeout
   idleTimeout: 0,
   fetch: async (req) => {
      const url = new URL(req.url);
      if (url.pathname === '/__dev') return sseResponse();
      const hit = await resolveStatic(DIST, url.pathname);
      if (!hit) return new Response('not found', { status: 404 });
      if (hit.mime === 'text/html') {
         const html = (await hit.file.text()).replace('</body>', `${RELOAD_SNIPPET}</body>`);
         return new Response(html, {
            headers: { 'content-type': hit.mime, 'cache-control': 'no-store' },
         });
      }
      return new Response(hit.file, {
         headers: { 'content-type': hit.mime, 'cache-control': 'no-store' },
      });
   },
});
console.log(`dev: http://localhost:${server.port}/ — watching ${WATCH_DIRS.join(', ')}`);
