// Render each document through all three slab drivers and stack the results
// into one comparison image.
//
//   bun tools/compare.ts                        # every examples/*.slab
//   bun tools/compare.ts 10-settings 08-glass   # only these
//   bun tools/compare.ts --width 900 --scale 2 --theme dusk
//
// Drivers, all fed the same document at the same viewport:
//   ghostty  `slab-tui --dump-after - --ansi` replayed into ghostty-web, i.e.
//            libghostty's VT engine as WASM behind a canvas grid renderer
//   native   `slab-native --headless-frame` — the offscreen wgpu painter
//   web      `slab gen wc` — the DOM painter from the @stencil-hq/wslab runtime
//
// The TUI grid fixes the shared viewport: width snaps to the 8-unit cell grid
// and height is `rows * 16`, so every driver solves the same box and none of
// them clips or pads the document. Inter and JetBrains Mono are served to
// chromium so the browser panels use the faces the compiler measured and wgpu
// rasterizes; the remaining differences are the renderers themselves.
//
// Writes out/compare/<doc>.png (the sheet) and out/compare/<doc>/ (the three
// panels plus the ANSI dump and pages that produced them).
//
// Prereqs: cargo, `bun install`, `bun x playwright install chromium`.

import { mkdir, rm, writeFile } from 'node:fs/promises';
import { basename, dirname, join } from 'node:path';
import type { Page } from 'playwright';
import { chromium } from 'playwright';
import { resolveStatic } from '../scripts/static.ts';

const ROOT = dirname(import.meta.dir);
const OUT = join(ROOT, 'out/compare');
const TAG = 'slab-cmp';
/** Cell size of the TUI grid in slab units (SPEC §14: 8x16 per cell). */
const CELL_W = 8;
const CELL_H = 16;

/** Comparison viewports: the gallery widths (tools/gallery.sh) snapped to the
 * TUI cell grid. Heights come from the document itself, so only width is fixed. */
const WIDTHS: Record<string, number> = {
   '00-player': 360,
   '01-settings': 800,
   '02-ops': 1920,
   '03-landing': 1440,
   '04-poster': 1000,
   '05-railyard': 800,
   '06-jcard': 1048,
   '07-monitor': 760,
   '08-glass': 896,
   '09-widget': 800,
   '10-settings': 760,
   '11-unicode': 800,
};
const DEFAULT_WIDTH = 800;

interface Options {
   docs: string[];
   width: number | null;
   scale: number;
   fontSize: number;
   theme: string | null;
}

function usage(message?: string): never {
   if (message) console.error(`error: ${message}`);
   console.error(
      'usage: bun tools/compare.ts [DOC...] [--width N] [--scale N] [--font-size N] [--theme NAME]',
   );
   process.exit(2);
}

function parseArgs(argv: string[]): Options {
   // 13px is where ghostty-web's cell advance lands on exactly 8px — the slab
   // cell width — so the terminal panel is column-for-column the same width as
   // the other two (its 15px rows stay 1px short of the 16-unit grid).
   const opts: Options = { docs: [], width: null, scale: 2, fontSize: 13, theme: null };
   for (let i = 0; i < argv.length; i++) {
      const arg = argv[i];
      const value = (): string => argv[++i] ?? usage(`${arg} needs a value`);
      const num = (): number => {
         const n = Number(value());
         if (!Number.isFinite(n) || n <= 0) usage(`${arg} needs a positive number`);
         return n;
      };
      if (arg === '--width') opts.width = num();
      else if (arg === '--scale') opts.scale = num();
      else if (arg === '--font-size') opts.fontSize = num();
      else if (arg === '--theme') opts.theme = value();
      else if (arg.startsWith('-')) usage(`unknown flag ${arg}`);
      else opts.docs.push(basename(arg).replace(/\.slab$/, ''));
   }
   return opts;
}

interface Run {
   code: number;
   stdout: string;
   stderr: string;
}

async function run(cmd: string[]): Promise<Run> {
   const proc = Bun.spawn(cmd, { cwd: ROOT, stdout: 'pipe', stderr: 'pipe' });
   const [stdout, stderr, code] = await Promise.all([
      new Response(proc.stdout).text(),
      new Response(proc.stderr).text(),
      proc.exited,
   ]);
   return { code, stdout, stderr };
}

/** Build the three drivers once and map binary name -> built executable. */
async function buildDrivers(): Promise<Record<string, string>> {
   console.log('compare: cargo build -p slab-cli -p slab-tui -p slab-native');
   const built = await run([
      'cargo',
      'build',
      '--message-format=json-render-diagnostics',
      '-p',
      'slab-cli',
      '-p',
      'slab-tui',
      '-p',
      'slab-native',
   ]);
   if (built.code !== 0) {
      console.error(built.stderr);
      throw new Error('cargo build failed');
   }
   const bins: Record<string, string> = {};
   for (const line of built.stdout.split('\n')) {
      if (!line.startsWith('{')) continue;
      const msg = JSON.parse(line) as { reason?: string; executable?: string | null };
      if (msg.reason === 'compiler-artifact' && msg.executable)
         bins[basename(msg.executable)] = msg.executable;
   }
   for (const name of ['slab', 'slab-tui', 'slab-native'])
      if (!bins[name]) throw new Error(`cargo did not report a ${name} executable`);
   return bins;
}

/** Pixel size straight from the PNG IHDR chunk. */
async function pngSize(path: string): Promise<{ w: number; h: number }> {
   const head = new DataView(await Bun.file(path).slice(0, 24).arrayBuffer());
   return { w: head.getUint32(16), h: head.getUint32(20) };
}

const FONT_FACES = ['Inter', 'JetBrains Mono']
   .flatMap((family) =>
      [
         [400, 'Regular'],
         [500, 'Medium'],
         [600, 'SemiBold'],
         [700, 'Bold'],
      ].map(
         ([weight, style]) =>
            `@font-face{font-family:${JSON.stringify(family)};font-weight:${weight};` +
            `src:url("/fonts/${family.replace(' ', '')}-${style}.ttf")format("truetype")}`,
      ),
   )
   .join('\n');

/** Wait for both families at every weight the documents use. */
const LOAD_FONTS = `
   await Promise.all(
      [400, 500, 600, 700].flatMap((w) => [
         document.fonts.load(w + ' 16px Inter'),
         document.fonts.load(w + ' 16px "JetBrains Mono"'),
      ]),
   );
   await document.fonts.ready;`;

/** Two frames after the last mutation the canvas/DOM paint has landed. */
const SETTLE = 'await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));';

function ghosttyPage(cols: number, rows: number, fontSize: number): string {
   return `<!doctype html><meta charset="utf-8"><title>ghostty</title>
<style>${FONT_FACES}
html,body{margin:0;background:#000}#term{display:inline-block}
#term canvas{display:block}</style>
<div id="term"></div>
<script type="module">
   import { init, Terminal } from '/vendor/ghostty-web.js';${LOAD_FONTS}
   await init();
   const term = new Terminal({
      cols: ${cols},
      rows: ${rows},
      fontFamily: 'JetBrains Mono',
      fontSize: ${fontSize},
      cursorBlink: false,
      disableStdin: true,
      convertEol: true,
      scrollback: 0,
      theme: { background: '#000000', foreground: '#c8ccd4' },
   });
   term.open(document.getElementById('term'));
   // \\e[?25l keeps the block cursor out of the still frame.
   term.write('\\u001b[?25l' + (await (await fetch('./tui.ansi')).text()));
   ${SETTLE}
   window.__ready = true;
</script>`;
}

function webPage(doc: string, width: number, height: number, theme: string | null): string {
   const themeAttr = theme ? ` theme="${theme}"` : '';
   return `<!doctype html><meta charset="utf-8"><title>web</title>
<style>${FONT_FACES}
html,body{margin:0;background:transparent}
#host{display:block;width:${width}px;height:${height}px}</style>
<${TAG} id="host"${themeAttr}></${TAG}>
<script type="module">${LOAD_FONTS}
   await import('./wc/${doc}.js');
   await customElements.whenDefined('${TAG}');
   const host = document.getElementById('host');
   const painted = () => host.shadowRoot?.querySelector('.slab-ops')?.childElementCount > 0;
   while (!painted()) await new Promise((r) => requestAnimationFrame(r));
   ${SETTLE}
   window.__ready = true;
</script>`;
}

interface Panel {
   driver: string;
   engine: string;
   /** Panel image relative to the sheet, or null when the driver failed. */
   src: string | null;
   note: string;
}

function sheetPage(doc: string, head: string, panels: Panel[], panelW: number): string {
   const figures = panels
      .map(
         (p) => `<figure>
      <h2>${p.driver}<span>${p.engine}</span></h2>
      ${p.src ? `<div class="shot"><img src="${p.src}" alt="${p.driver}"></div>` : '<div class="shot fail"></div>'}
      <figcaption${p.src ? '' : ' class="fail"'}>${p.note}</figcaption>
   </figure>`,
      )
      .join('\n   ');
   return `<!doctype html><meta charset="utf-8"><title>${doc}</title>
<style>${FONT_FACES}
:root{color-scheme:dark}
body{margin:0;padding:28px;background:#0b0d12;color:#e8eef6;
   font:500 13px/1.4 Inter,system-ui,sans-serif;width:max-content}
h1{margin:0 0 4px;font-size:19px;font-weight:600;letter-spacing:-0.01em}
p.meta{margin:0 0 22px;color:#8b95a7;font:400 12px/1.4 "JetBrains Mono",monospace}
.row{display:flex;gap:24px;align-items:flex-start}
figure{margin:0;width:${panelW}px}
h2{margin:0 0 8px;font-size:13px;font-weight:600;display:flex;justify-content:space-between;
   align-items:baseline;gap:8px}
h2 span{color:#8b95a7;font:400 11px/1 "JetBrains Mono",monospace;text-align:right}
.shot{border:1px solid #232936;border-radius:6px;overflow:hidden;
   background-color:#12151c;
   background-image:linear-gradient(45deg,#171b24 25%,transparent 25%,transparent 75%,#171b24 75%),
      linear-gradient(45deg,#171b24 25%,transparent 25%,transparent 75%,#171b24 75%);
   background-size:16px 16px;background-position:0 0,8px 8px}
.shot.fail{height:120px;border-color:#5a2230;background:#1b1013}
img{display:block;width:100%;height:auto}
figcaption{margin-top:8px;color:#8b95a7;font:400 11px/1.5 "JetBrains Mono",monospace;
   word-break:break-word}
figcaption.fail{color:#ff8ba0}
</style>
<h1>${doc}</h1>
<p class="meta">${head}</p>
<div class="row">
   ${figures}
</div>
<script type="module">${LOAD_FONTS}
   await Promise.all([...document.images].map((img) => img.decode()));
   ${SETTLE}
   window.__ready = true;
</script>`;
}

/** Screenshot `selector` on a served page once it reports `window.__ready`. */
async function shoot(page: Page, url: string, selector: string, out: string): Promise<void> {
   await page.goto(url, { waitUntil: 'load' });
   await page.waitForFunction('window.__ready === true', null, { timeout: 30_000 });
   await page.locator(selector).screenshot({ path: out, omitBackground: true });
}

/** Run a driver binary, failing with the last line it printed to stderr. */
async function exec(cmd: string[]): Promise<void> {
   const done = await run(cmd);
   if (done.code === 0) return;
   const tail = done.stderr.trim().split('\n').at(-1);
   throw new Error(tail || `${basename(cmd[0])} exited ${done.code}`);
}

/** Render one driver's panel; a failure becomes an error panel, not a stop. */
async function capture(
   driver: string,
   engine: string,
   png: string,
   note: (px: { w: number; h: number }) => string,
   render: () => Promise<void>,
): Promise<Panel> {
   try {
      await render();
      return { driver, engine, src: basename(png), note: note(await pngSize(png)) };
   } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      return { driver, engine, src: null, note: message.split('\n')[0] };
   }
}

const opts = parseArgs(Bun.argv.slice(2));
const all = [...new Bun.Glob('*.slab').scanSync(join(ROOT, 'examples'))]
   .map((f) => f.replace(/\.slab$/, ''))
   .sort();
const docs = opts.docs.length > 0 ? opts.docs : all;
for (const doc of docs) if (!all.includes(doc)) usage(`no examples/${doc}.slab`);

const bins = await buildDrivers();
await mkdir(OUT, { recursive: true });

const server = Bun.serve({
   port: 0,
   async fetch(req) {
      const { pathname } = new URL(req.url);
      const mounts: [string, string][] = [
         ['/vendor/', join(ROOT, 'node_modules/ghostty-web/dist')],
         ['/fonts/', join(ROOT, 'assets/fonts')],
         ['/', OUT],
      ];
      for (const [prefix, dir] of mounts) {
         if (!pathname.startsWith(prefix)) continue;
         const hit = await resolveStatic(dir, pathname.slice(prefix.length - 1));
         if (hit) return new Response(hit.file, { headers: { 'content-type': hit.mime } });
      }
      return new Response('not found', { status: 404 });
   },
});
const origin = `http://127.0.0.1:${server.port}`;

const browser = await chromium.launch();
const page = await browser.newPage({ deviceScaleFactor: opts.scale, colorScheme: 'light' });
page.on('pageerror', (e) => console.error(`  pageerror: ${e.message}`));

let failures = 0;

for (const doc of docs) {
   const file = join('examples', `${doc}.slab`);
   const dir = join(OUT, doc);
   // Only the documents on this run are refreshed; earlier sheets stay put.
   await rm(dir, { recursive: true, force: true });
   await mkdir(dir, { recursive: true });
   const width = CELL_W * Math.floor((opts.width ?? WIDTHS[doc] ?? DEFAULT_WIDTH) / CELL_W);
   const themeArgs = opts.theme ? ['--theme', opts.theme] : [];

   // ── ghostty: cells from the TUI driver, pixels from libghostty-vt ────────
   const dump = await run([
      bins['slab-tui'],
      file,
      '--width',
      String(width),
      '--dump-after',
      '-',
      '--ansi',
      ...themeArgs,
   ]);
   if (dump.code !== 0) {
      console.error(`FAIL ${doc}: slab-tui: ${dump.stderr.trim()}`);
      failures += 1;
      continue;
   }
   const lines = dump.stdout.replace(/\n$/, '').split('\n');
   const cols = width / CELL_W;
   const rows = lines.length;
   const height = rows * CELL_H;
   await writeFile(join(dir, 'tui.ansi'), lines.join('\n'));
   await writeFile(join(dir, 'ghostty.html'), ghosttyPage(cols, rows, opts.fontSize));
   // Panels are captured per element, but a viewport that already fits them
   // keeps chromium from scroll-stitching large documents.
   await page.setViewportSize({
      width: Math.min(width + 64, 2400),
      height: Math.min(height + 64, 2400),
   });

   const panels = [
      await capture(
         'ghostty',
         'ghostty-web (libghostty-vt wasm)',
         join(dir, 'ghostty.png'),
         (px) => `${cols}x${rows} cells at ${opts.fontSize}px · ${px.w}x${px.h}px`,
         () =>
            shoot(page, `${origin}/${doc}/ghostty.html`, '#term canvas', join(dir, 'ghostty.png')),
      ),
      // ── native: offscreen wgpu ────────────────────────────────────────────
      await capture(
         'native',
         'slab-native (wgpu)',
         join(dir, 'native.png'),
         (px) => `${width}x${height} units at ${opts.scale}x · ${px.w}x${px.h}px`,
         () =>
            exec([
               bins['slab-native'],
               file,
               '--headless-frame',
               join(dir, 'native.png'),
               '--width',
               String(width),
               '--height',
               String(height),
               '--scale',
               String(opts.scale),
               ...themeArgs,
            ]),
      ),
      // ── web: generated component, DOM painter ─────────────────────────────
      await capture(
         'web',
         'gen wc + wslab DOM painter',
         join(dir, 'web.png'),
         (px) => `${width}x${height} units at ${opts.scale}x · ${px.w}x${px.h}px`,
         async () => {
            await exec([bins.slab, 'gen', 'wc', file, '-o', join(dir, 'wc'), '--tag', TAG]);
            await writeFile(join(dir, 'web.html'), webPage(doc, width, height, opts.theme));
            await shoot(page, `${origin}/${doc}/web.html`, '#host', join(dir, 'web.png'));
         },
      ),
   ];
   failures += panels.filter((p) => p.src === null).length;

   // ── sheet ────────────────────────────────────────────────────────────────
   const panelW = Math.min(Math.max(width, 320), 620);
   const head =
      `${width}x${height} units · ${cols}x${rows} cells · ${opts.scale}x` +
      `${opts.theme ? ` · theme ${opts.theme}` : ''}`;
   await writeFile(join(dir, 'sheet.html'), sheetPage(doc, head, panels, panelW));
   // A short viewport lets the full-page shot shrink-wrap the sheet.
   await page.setViewportSize({ width: panelW * 3 + 24 * 2 + 28 * 2, height: 200 });
   await page.goto(`${origin}/${doc}/sheet.html`, { waitUntil: 'load' });
   await page.waitForFunction('window.__ready === true', null, { timeout: 30_000 });
   const sheet = join(OUT, `${doc}.png`);
   await page.screenshot({ path: sheet, fullPage: true });
   const px = await pngSize(sheet);
   const ok = panels.filter((p) => p.src).length;
   console.log(`${ok === 3 ? 'ok  ' : 'part'} ${doc}: ${ok}/3 drivers · ${px.w}x${px.h}px`);
}

await browser.close();
server.stop();
console.log(`compare: ${docs.length} document(s) -> out/compare/`);
if (failures > 0) {
   console.error(`compare: ${failures} driver render(s) failed`);
   process.exit(1);
}
