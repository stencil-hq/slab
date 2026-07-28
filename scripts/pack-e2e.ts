// End-to-end proof of the npm distribution (run in CI after `just pack`).
//
// Packs tarballs of @stencil-hq/wslab, @stencil-hq/dslab, and @stencil-hq/slab
// into a temp dir OUTSIDE the repo, installs them into a scratch project, and
// asserts the full user journey: `slab check` → `slab render` (PNG) → `slab gen wc` → generated
// component fetches its Rust-kernel WASM sidecar and works in chromium (upgrade,
// paint, property repaint, Save click fires the `save` CustomEvent).
//
//   bun scripts/pack-e2e.ts
//
// Prereqs: `just pack` and `bun x playwright install chromium`.

import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { extname, join, resolve } from 'node:path';
import { chromium } from 'playwright';

const ROOT = resolve(import.meta.dirname, '..');
const WORK = mkdtempSync(join(tmpdir(), 'slab-pack-e2e-'));
console.log(`pack-e2e: scratch dir ${WORK}`);

let failures = 0;
function check(name: string, cond: boolean, detail: string): void {
   if (cond) {
      console.log(`PASS ${name} — ${detail}`);
   } else {
      failures += 1;
      console.error(`FAIL ${name} — ${detail}`);
   }
}

function run(cmd: string[], cwd: string): { code: number; out: string } {
   const r = Bun.spawnSync({ cmd, cwd, stdin: 'ignore', stdout: 'pipe', stderr: 'pipe' });
   const out = `${r.stdout?.toString() ?? ''}${r.stderr?.toString() ?? ''}`;
   return { code: r.exitCode ?? 1, out };
}

// ── 1. pack tarballs ─────────────────────────────────────────────────

const tarballs: string[] = [];
for (const dir of ['clients/web', 'packages/dslab', 'packages/slab']) {
   const r = run(['bun', 'pm', 'pack', '--destination', WORK], join(ROOT, dir));
   if (r.code !== 0) {
      console.error(`pack-e2e: bun pm pack failed in ${dir}:\n${r.out}`);
      process.exit(1);
   }
}
for (const f of new Bun.Glob('*.tgz').scanSync(WORK)) tarballs.push(join(WORK, f));
check('pack tarballs', tarballs.length === 3, tarballs.map((t) => t.split('/').pop()).join(', '));
const wslabTgz = tarballs.find((t) => t.includes('wslab'));
const wslabFiles = new Set(wslabTgz ? run(['tar', '-tf', wslabTgz], WORK).out.split('\n') : []);
const dslabTgz = tarballs.find((t) => t.includes('dslab'));
const dslabFiles = new Set(dslabTgz ? run(['tar', '-tf', dslabTgz], WORK).out.split('\n') : []);
check(
   'dslab client package',
   dslabFiles.has('package/bin/dslab.js') &&
      dslabFiles.has('package/dist/index.js') &&
      dslabFiles.has('package/dist/index.d.ts') &&
      dslabFiles.has('package/LICENSE'),
   'CLI + runtime + declarations + license',
);
check(
   'wslab kernel wasm layouts',
   wslabFiles.has('package/wasm/slab_kernel.js') &&
      wslabFiles.has('package/wasm/slab_kernel_bg.wasm') &&
      wslabFiles.has('package/dist/wasm/slab_kernel.js') &&
      wslabFiles.has('package/dist/wasm/slab_kernel_bg.wasm'),
   'root and dist glue + sidecar',
);

// ── 2. scratch project ───────────────────────────────────────────────

const proj = join(WORK, 'proj');
mkdirSync(proj, { recursive: true });
writeFileSync(
   join(proj, 'package.json'),
   JSON.stringify({
      name: 'scratch',
      private: true,
   }),
);
{
   const r = run(['bun', 'add', ...tarballs], proj);
   check('bun add tarballs', r.code === 0, r.code === 0 ? 'installed' : r.out.slice(0, 400));
   if (r.code !== 0) process.exit(1);
}

{
   const r = run(
      [
         'bun',
         '-e',
         "import { DriveClient } from '@stencil-hq/dslab'; if (typeof DriveClient.connect !== 'function') process.exit(1);",
      ],
      proj,
   );
   check('dslab import', r.code === 0, r.code === 0 ? 'loaded' : r.out.slice(0, 400));
}

{
   const r = run(['bun', 'node_modules/@stencil-hq/dslab/bin/dslab.js', '--help'], proj);
   check(
      'dslab help',
      r.code === 0 && r.out.includes('usage:'),
      r.code === 0 ? 'printed usage' : r.out.slice(0, 400),
   );
}

copyFileSync(join(ROOT, 'examples/10-settings.slab'), join(proj, '10-settings.slab'));
copyFileSync(join(ROOT, 'examples/12-tracklist.slab'), join(proj, '12-tracklist.slab'));

// ── 3. CLI journey ───────────────────────────────────────────────────

{
   const r = run(['bun', 'x', 'slab', 'check', '10-settings.slab'], proj);
   check(
      'slab check version stamp',
      r.code === 0 &&
         r.out.includes('slab compiler ') &&
         r.out.includes('package @stencil-hq/slab '),
      r.out.trim().split('\n').slice(0, 2).join(' | '),
   );
}

{
   const r = run(['bun', 'x', 'slab', 'render', '--help'], proj);
   check(
      'slab render help',
      r.code === 0 && r.out.includes('--theme NAME'),
      r.out.trim().split('\n')[0] ?? '',
   );
}

{
   const r = run(
      ['bun', 'x', 'slab', 'render', '12-tracklist.slab', '-o', 'themed.svg', '--theme', 'dusk'],
      proj,
   );
   const svg = r.code === 0 ? readFileSync(join(proj, 'themed.svg'), 'utf8') : '';
   check('slab render named theme', r.code === 0 && svg.startsWith('<svg'), `${svg.length} chars`);
}

{
   const r = run(
      ['bun', 'x', 'slab', 'render', '10-settings.slab', '-o', 'out.png', '--width', '800'],
      proj,
   );
   const png = r.code === 0 ? readFileSync(join(proj, 'out.png')) : Buffer.alloc(0);
   const magic = png.subarray(0, 4).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47]));
   check('slab render PNG', r.code === 0 && magic && png.length > 10_000, `${png.length} bytes`);
}

{
   const r = run(
      ['bun', 'x', 'slab', 'gen', 'wc', '10-settings.slab', '-o', 'dist', '--tag', 't-settings'],
      proj,
   );
   check('slab gen wc', r.code === 0, r.out.trim().split('\n').pop() ?? '');
}

// node parity: the published bin must run under plain node ≥20 too.
{
   const nv = run(['node', '--version'], proj);
   const major = Number(nv.out.trim().replace(/^v/, '').split('.')[0] ?? 0);
   if (nv.code === 0 && major >= 20) {
      const r = run(
         ['node', 'node_modules/@stencil-hq/slab/bin/slab.js', 'check', '10-settings.slab'],
         proj,
      );
      check('node bin check', r.code === 0, `node ${nv.out.trim()}`);
   } else {
      console.log('skip node bin check (node >=20 not present)');
   }
}

// ── 4. browser: generated component behaves ──────────────────────────

writeFileSync(
   join(proj, 'index.html'),
   `<!doctype html>
<html><head><meta charset="utf-8"><style>body{margin:0;padding:24px}#host{width:800px;height:640px}</style></head>
<body>
<t-settings id="host" title="Settings" status="ready"></t-settings>
<script type="module">import './dist/10-settings.js';</script>
</body></html>`,
);

const TYPES: Record<string, string> = {
   '.html': 'text/html; charset=utf-8',
   '.js': 'text/javascript; charset=utf-8',
   '.slir': 'application/octet-stream',
   '.wasm': 'application/wasm',
};

const server = Bun.serve({
   port: 0,
   async fetch(req) {
      const path = new URL(req.url).pathname;
      const rel = path === '/' ? 'index.html' : path;
      const file = Bun.file(join(proj, rel));
      if (!(await file.exists())) return new Response('not found', { status: 404 });
      return new Response(file, {
         headers: { 'content-type': TYPES[extname(rel)] ?? 'application/octet-stream' },
      });
   },
});

interface SceneGeom {
   key: string;
   x: number;
   y: number;
   w: number;
   h: number;
}
interface DebugGlobal {
   __slabDebug?: Map<Element, { geom(): SceneGeom[] }>;
   __sig?: Record<string, unknown[]>;
}

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1100, height: 800 } });
page.on('pageerror', (e) => console.error('pageerror:', e.message));
await page.addInitScript(() => {
   // Debug hook read by SlabElement.#registerDebug before elements connect.
   const g = globalThis as { __SLAB_DEBUG__?: boolean };
   g.__SLAB_DEBUG__ = true;
});
const kernelWasmResponse = page.waitForResponse((response) =>
   new URL(response.url()).pathname.endsWith('/dist/wasm/slab_kernel_bg.wasm'),
);
await page.goto(`http://127.0.0.1:${server.port}/`);
const kernelWasm = await kernelWasmResponse;
const kernelWasmBytes = kernelWasm.ok() ? (await kernelWasm.body()).byteLength : 0;
check(
   'kernel WASM sidecar loaded',
   kernelWasm.ok() && kernelWasmBytes > 0,
   `${kernelWasm.status()} ${kernelWasmBytes} bytes`,
);

// (a) upgrade + paint
await page.evaluate(() => customElements.whenDefined('t-settings'));
await page.waitForFunction(() => {
   const host = document.getElementById('host');
   return (host?.shadowRoot?.querySelectorAll('.slab-ops div, .slab-ops span').length ?? 0) > 10;
});
const boxCount = await page.evaluate(
   () =>
      document.getElementById('host')?.shadowRoot?.querySelectorAll('.slab-ops div, .slab-ops span')
         .length ?? 0,
);
check('component upgrade+paint', boxCount > 10, `${boxCount} shadow boxes`);

// (b) title property repaint
const titleBefore = await page.evaluate(
   () => document.getElementById('host')?.shadowRoot?.querySelector('.slab-ops')?.textContent ?? '',
);
await page.evaluate(() => {
   // HTMLElement.title is string; the generated element shadows it as a param.
   const host = document.getElementById('host');
   if (host) host.title = 'Repainted';
});
await page.waitForFunction(() =>
   (
      document.getElementById('host')?.shadowRoot?.querySelector('.slab-ops')?.textContent ?? ''
   ).includes('Repainted'),
);
check('title property repaint', !titleBefore.includes('Repainted'), 'shadow text updated');

// (c) Save click → save CustomEvent (kernel geometry via __slabDebug)
await page.evaluate(() => {
   const host = document.getElementById('host');
   const g = globalThis as DebugGlobal;
   const sig: Record<string, unknown[]> = { save: [] };
   g.__sig = sig;
   host?.addEventListener('save', (e) => {
      // signal events are always CustomEvents (SlabElement contract)
      const ce = e as CustomEvent;
      sig.save.push(ce.detail);
   });
});
const save = await page.evaluate(() => {
   const host = document.getElementById('host');
   if (!host) return null;
   const dbg = globalThis as DebugGlobal;
   const g = dbg.__slabDebug
      ?.get(host)
      ?.geom()
      .find((s) => s.key.includes('#save'));
   if (!g) return null;
   const r = host.getBoundingClientRect();
   return { cx: r.left + g.x + g.w / 2, cy: r.top + g.y + g.h / 2 };
});
check('save button located', save !== null, JSON.stringify(save));
if (save) {
   await page.mouse.click(save.cx, save.cy);
   await page.waitForFunction(() => {
      const dbg = globalThis as DebugGlobal;
      return (dbg.__sig?.save?.length ?? 0) > 0;
   });
   check('save CustomEvent fired', true, 'detail null (Activate)');
}

await browser.close();
server.stop();
rmSync(WORK, { recursive: true, force: true });

if (failures > 0) {
   console.error(`\n${failures} pack-e2e assertion(s) failed`);
   process.exit(1);
}
console.log('\nall pack-e2e assertions passed');
