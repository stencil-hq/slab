// Playwright e2e for the generated 00-player web component (player demo).
//
//   bun tools/player-e2e.ts
//
// Prereqs: `slab gen wc examples/00-player.slab -o examples/web-demo/dist
// --tag slab-player` and a chromium from `bun x playwright install chromium`.
// Serves examples/web-demo over Bun.serve on a random port (player.html at /),
// then asserts:
//   (a) component upgraded, shadow box count > 10
//   (b) click the play circle (kernel geometry via __slabDebug) → 'toggle'
//       CustomEvent AND the glyph swaps (|| span visible, |> hidden — opacity)
//   (c) click next → shadow title text becomes playlist[1] and the queue
//       playing-marker row moved from index 0 to 1
//   (d) progress attribute 20% → 80% moves the playhead knob by ≈ 60% of the
//       waveform width (kernel geometry)
//   (e) 4 slotted <slab-track> rows render inside the hole slot
//   (f) keyboard: focus host, Tab×3 + Enter → 'toggle' (kernel tab order
//       shuffle, prev, toggle) — on a fresh page load
//   (g) click queue row 2 → 'pick' CustomEvent whose target is that
//       <slab-track>; player title + queue playing-marker jump to it and the
//       play state survives
//   (h) arrow keys walk the same focus ring: ArrowRight×3 + Enter → 'toggle';
//       ArrowLeft×3 from there wraps past shuffle to loop; fresh page
//       ArrowRight×4 + Enter → 'next'
//   (i) hover ease: pointer over #shuffle → the button's shadow-DOM div bg
//       animates to moss #1B2E22 over 140ms ease-out (sampled ~20ms,
//       rAF-tolerant: at least one intermediate sample differs from both ends)
//
// Playlist contract with examples/web-demo/player.html:
//   [0] Pale Green Things  [1] This Year  [2] Song for Dennis Brown
//   [3] Love Love Love — all by The Mountain Goats.

import { dirname, extname, join } from 'node:path';
import { chromium } from 'playwright';

interface SceneGeom {
   key: string;
   node: number;
   x: number;
   y: number;
   w: number;
   h: number;
}

interface DebugGlobal {
   __slabDebug?: Map<Element, { geom(): SceneGeom[] }>;
   __sig?: Record<string, unknown[]>;
   __pick?: number[];
   __hover?: { t: number; bg: string }[];
   __hoverDone?: boolean;
}

const root = dirname(import.meta.dir);
const demoDir = join(root, 'examples/web-demo');

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
      const rel = path === '/' ? 'player.html' : path;
      const file = Bun.file(join(demoDir, rel));
      if (!(await file.exists())) return new Response('not found', { status: 404 });
      const type = TYPES[extname(rel)] ?? 'application/octet-stream';
      return new Response(file, { headers: { 'content-type': type } });
   },
});

let failures = 0;
function check(name: string, cond: boolean, detail: string): void {
   if (cond) {
      console.log(`PASS ${name} — ${detail}`);
   } else {
      failures += 1;
      console.error(`FAIL ${name} — ${detail}`);
   }
}

const browser = await chromium.launch();
const page = await browser.newPage({
   viewport: { width: 800, height: 900 },
   colorScheme: 'dark',
});
page.on('pageerror', (e) => console.error('pageerror:', e.message));
await page.addInitScript(() => {
   // Read by SlabElement.#registerDebug before elements connect.
   const g = globalThis as { __SLAB_DEBUG__?: boolean };
   g.__SLAB_DEBUG__ = true;
});

/** Wait for the host to upgrade and paint its shadow boxes. */
async function waitUpgrade(): Promise<void> {
   await page.waitForFunction(() => {
      const host = document.getElementById('host');
      if (!host?.shadowRoot) return false;
      return host.shadowRoot.querySelectorAll('.slab-ops div, .slab-ops span').length > 10;
   });
}

/** Record signal CustomEvents on the host into globalThis.__sig. */
async function installRecorder(): Promise<void> {
   await page.evaluate(() => {
      const host = document.getElementById('host');
      const g = globalThis as DebugGlobal;
      g.__sig = {};
      for (const name of ['shuffle', 'prev', 'toggle', 'next', 'loop']) {
         host?.addEventListener(name, (e) => {
            // Signal events are always CustomEvents (SlabElement contract).
            const ce = e as CustomEvent;
            const sig = g.__sig ?? {};
            const list = sig[name] ?? [];
            sig[name] = list;
            list.push(ce.detail);
         });
      }
   });
}

/** Viewport-space center of the scene node whose key contains `suffix`. */
async function nodeCenter(suffix: string) {
   return await page.evaluate((sfx) => {
      const host = document.getElementById('host');
      if (!host) return null;
      const dbg = globalThis as DebugGlobal;
      const entry = dbg.__slabDebug?.get(host);
      const g = entry?.geom().find((s) => s.key.endsWith(sfx));
      if (!g) return null;
      const r = host.getBoundingClientRect();
      return { cx: r.left + g.x + g.w / 2, cy: r.top + g.y + g.h / 2, x: g.x, y: g.y, w: g.w };
   }, suffix);
}

/** Computed opacity of the first shadow span whose text equals `text`. */
async function glyphOpacity(text: string): Promise<number | null> {
   return await page.evaluate((t) => {
      const host = document.getElementById('host');
      const spans = [...(host?.shadowRoot?.querySelectorAll('.slab-ops span') ?? [])];
      const el = spans.find((s) => s.textContent === t);
      if (!el) return null;
      // Opacity patches land on wrapper divs, so multiply up the ancestor chain.
      let o = 1;
      for (let n: Element | null = el; n; n = n.parentElement) {
         o *= Number(getComputedStyle(n).opacity);
      }
      return o;
   }, text);
}

/** Doc-space geometry of the 8×8 playhead knob + the full waveform width. */
async function knobGeom() {
   return await page.evaluate(() => {
      const host = document.getElementById('host');
      const dbg = globalThis as DebugGlobal;
      const geom = dbg.__slabDebug?.get(host as Element)?.geom() ?? [];
      const knob = geom.find((s) => s.w === 8 && s.h === 8);
      const waveW = Math.max(0, ...geom.filter((s) => s.h === 44).map((s) => s.w));
      return knob ? { x: knob.x, waveW } : null;
   });
}

await page.goto(`http://127.0.0.1:${server.port}/`);

// ------------------------------------------------------------------- (a)
await waitUpgrade();
const boxCount = await page.evaluate(() => {
   const host = document.getElementById('host');
   return host?.shadowRoot?.querySelectorAll('.slab-ops div, .slab-ops span').length ?? 0;
});
check('(a) upgrade+paint', boxCount > 10, `${boxCount} shadow-DOM boxes`);

await installRecorder();

// ------------------------------------------------------------------- (b)
const play = await nodeCenter('#play');
check('(b) play circle located', play !== null, `kernel geometry ${JSON.stringify(play)}`);
if (play) {
   const before = { pause: await glyphOpacity('||'), play: await glyphOpacity('|>') };
   await page.mouse.click(play.cx, play.cy);
   await page.waitForFunction(() => {
      const dbg = globalThis as DebugGlobal;
      return (dbg.__sig?.toggle?.length ?? 0) > 0;
   });
   const detail = await page.evaluate(() => {
      const dbg = globalThis as DebugGlobal;
      return dbg.__sig?.toggle?.[0];
   });
   check(
      '(b) toggle CustomEvent',
      detail === null,
      `detail=${JSON.stringify(detail)} (Activate → null)`,
   );
   await page.waitForFunction(() => {
      const host = document.getElementById('host');
      const spans = [...(host?.shadowRoot?.querySelectorAll('.slab-ops span') ?? [])];
      const el = spans.find((s) => s.textContent === '||');
      if (!el) return false;
      let o = 1;
      for (let n: Element | null = el; n; n = n.parentElement) {
         o *= Number(getComputedStyle(n).opacity);
      }
      return o === 1;
   });
   const after = { pause: await glyphOpacity('||'), play: await glyphOpacity('|>') };
   check(
      '(b) glyph swap |> → ||',
      before.pause === 0 && before.play === 1 && after.pause === 1 && after.play === 0,
      `opacity |>/|| before ${before.play}/${before.pause} → after ${after.play}/${after.pause}`,
   );
   // Pause again so the app ticker cannot race the (d) geometry probe.
   await page.mouse.click(play.cx, play.cy);
   await page.waitForFunction(() => {
      const dbg = globalThis as DebugGlobal;
      return (dbg.__sig?.toggle?.length ?? 0) === 2;
   });
}

// ------------------------------------------------------------------- (c)
const markerBefore = await page.evaluate(() => {
   const rows = [...document.querySelectorAll('slab-track')];
   return rows.findIndex((r) => r.hasAttribute('playing'));
});
const next = await nodeCenter('#next');
check('(c) next button located', next !== null, `kernel geometry ${JSON.stringify(next)}`);
if (next) {
   await page.mouse.click(next.cx, next.cy);
   await page.waitForFunction(() => {
      const host = document.getElementById('host');
      const spans = [...(host?.shadowRoot?.querySelectorAll('.slab-ops span') ?? [])];
      return spans.some((s) => s.textContent === 'This Year');
   });
   const markerAfter = await page.evaluate(() => {
      const rows = [...document.querySelectorAll('slab-track')];
      return rows.findIndex((r) => r.hasAttribute('playing'));
   });
   check(
      '(c) next → title + queue marker',
      markerBefore === 0 && markerAfter === 1,
      `shadow title now "This Year"; playing marker row ${markerBefore} → ${markerAfter}`,
   );
}

// ------------------------------------------------------------------- (d)
await page.evaluate(() => {
   document.getElementById('host')?.setAttribute('progress', '20%');
});
await page.waitForFunction(() => {
   const dbg = globalThis as DebugGlobal;
   const host = document.getElementById('host');
   const geom = dbg.__slabDebug?.get(host as Element)?.geom() ?? [];
   const knob = geom.find((s) => s.w === 8 && s.h === 8);
   const waveW = Math.max(0, ...geom.filter((s) => s.h === 44).map((s) => s.w));
   return knob !== undefined && waveW > 0 && knob.x > waveW * 0.1;
});
const at20 = await knobGeom();
await page.evaluate(() => {
   document.getElementById('host')?.setAttribute('progress', '80%');
});
await page.waitForFunction((x20) => {
   const dbg = globalThis as DebugGlobal;
   const host = document.getElementById('host');
   const geom = dbg.__slabDebug?.get(host as Element)?.geom() ?? [];
   const knob = geom.find((s) => s.w === 8 && s.h === 8);
   return knob !== undefined && x20 !== null && knob.x > x20;
}, at20?.x ?? null);
const at80 = await knobGeom();
const dx = at20 && at80 ? at80.x - at20.x : Number.NaN;
const expected = at20 ? at20.waveW * 0.6 : Number.NaN;
check(
   '(d) progress 20% → 80% moves knob',
   at20 !== null && at80 !== null && Math.abs(dx - expected) < 2,
   `knob x ${at20?.x} → ${at80?.x} (Δ ${dx.toFixed(1)}; expected ≈ ${expected.toFixed(1)} = 60% of wave ${at20?.waveW})`,
);

// ------------------------------------------------------------------- (e)
await page.waitForFunction(() => {
   const host = document.getElementById('host');
   const slot = host?.shadowRoot?.querySelector('.slab-hole slot') as HTMLSlotElement | null;
   const first = slot?.assignedElements()[0] as HTMLElement | undefined;
   return first !== undefined && first.offsetHeight > 0 && first.shadowRoot !== null;
});
const queue = await page.evaluate(() => {
   const host = document.getElementById('host');
   const slot = host?.shadowRoot?.querySelector('.slab-hole slot') as HTMLSlotElement | null;
   const assigned = (slot?.assignedElements() ?? []) as HTMLElement[];
   return {
      count: assigned.length,
      tags: assigned.map((el) => el.tagName.toLowerCase()),
      heights: assigned.map((el) => el.offsetHeight),
      painted: assigned.every(
         (el) => (el.shadowRoot?.querySelectorAll('.slab-ops *').length ?? 0) > 0,
      ),
   };
});
check(
   '(e) 4 slotted <slab-track> rows',
   queue.count === 4 &&
      queue.tags.every((t) => t === 'slab-track') &&
      queue.heights.every((h) => h > 0) &&
      queue.painted,
   JSON.stringify(queue),
);

// ------------------------------------------------------------------- (f)
// Fresh load: kernel focus starts empty, so Tab order is shuffle, prev, toggle.
await page.goto(`http://127.0.0.1:${server.port}/`);
await waitUpgrade();
await installRecorder();
await page.evaluate(() => {
   document.getElementById('host')?.focus();
});
await page.keyboard.press('Tab');
await page.keyboard.press('Tab');
await page.keyboard.press('Tab');
await page.keyboard.press('Enter');
await page.waitForFunction(() => {
   const dbg = globalThis as DebugGlobal;
   return (dbg.__sig?.toggle?.length ?? 0) > 0;
});
const kb = await page.evaluate(() => {
   const dbg = globalThis as DebugGlobal;
   const sig = dbg.__sig ?? {};
   return {
      toggle: sig.toggle?.length ?? 0,
      others: ['shuffle', 'prev', 'next', 'loop'].filter((n) => (sig[n]?.length ?? 0) > 0),
   };
});
check(
   '(f) keyboard Tab×3 + Enter → toggle',
   kb.toggle === 1 && kb.others.length === 0,
   `toggle ×${kb.toggle}, stray signals: ${JSON.stringify(kb.others)}`,
);

// ------------------------------------------------------------------- (g)
// Continue on the (f) page: toggle fired once there, so the app is playing
// with the marker on row 0 — clicking row 2 must move it without pausing.
await page.evaluate(() => {
   const g = globalThis as DebugGlobal;
   g.__pick = [];
   document.getElementById('host')?.addEventListener('pick', (e) => {
      const rows = [...document.querySelectorAll('slab-track')];
      g.__pick?.push(rows.indexOf(e.target as Element));
   });
});
await page.waitForFunction(() =>
   [...document.querySelectorAll('slab-track')].every(
      (row) => (row.shadowRoot?.querySelectorAll('.slab-ops *').length ?? 0) > 0,
   ),
);
const row2 = await page.evaluate(() => {
   const r = [...document.querySelectorAll('slab-track')][2]?.getBoundingClientRect();
   return r ? { cx: r.left + r.width / 2, cy: r.top + r.height / 2 } : null;
});
check('(g) queue row 2 located', row2 !== null, `light-DOM rect center ${JSON.stringify(row2)}`);
if (row2) {
   await page.mouse.click(row2.cx, row2.cy);
   await page.waitForFunction(() => {
      const g = globalThis as DebugGlobal;
      const host = document.getElementById('host');
      const spans = [...(host?.shadowRoot?.querySelectorAll('.slab-ops span') ?? [])];
      return (
         (g.__pick?.length ?? 0) > 0 && spans.some((s) => s.textContent === 'Song for Dennis Brown')
      );
   });
   const picked = await page.evaluate(() => {
      const g = globalThis as DebugGlobal;
      const host = document.getElementById('host');
      const rows = [...document.querySelectorAll('slab-track')];
      return {
         picks: g.__pick ?? [],
         title: host?.getAttribute('title'),
         marker: rows.findIndex((r) => r.hasAttribute('playing')),
         playing: host?.hasAttribute('playing') ?? false,
      };
   });
   check(
      '(g) click row 2 → pick + current track',
      picked.picks.length === 1 &&
         picked.picks[0] === 2 &&
         picked.title === 'Song for Dennis Brown' &&
         picked.marker === 2 &&
         picked.playing,
      `picks ${JSON.stringify(picked.picks)}, title "${picked.title}", marker row ${picked.marker}, still playing ${picked.playing}`,
   );
}

// ------------------------------------------------------------------- (h)
// Fresh load: arrow keys walk the focus ring (shuffle, prev, toggle, next,
// loop) whenever kernel focus is not an edit field.
await page.goto(`http://127.0.0.1:${server.port}/`);
await waitUpgrade();
await installRecorder();
await page.evaluate(() => {
   document.getElementById('host')?.focus();
});
for (let k = 0; k < 3; k++) await page.keyboard.press('ArrowRight');
await page.keyboard.press('Enter');
await page.waitForFunction(() => {
   const dbg = globalThis as DebugGlobal;
   return (dbg.__sig?.toggle?.length ?? 0) > 0;
});
const ar3 = await page.evaluate(() => {
   const sig = (globalThis as DebugGlobal).__sig ?? {};
   return {
      toggle: sig.toggle?.length ?? 0,
      others: ['shuffle', 'prev', 'next', 'loop'].filter((n) => (sig[n]?.length ?? 0) > 0),
   };
});
check(
   '(h) ArrowRight×3 + Enter → toggle',
   ar3.toggle === 1 && ar3.others.length === 0,
   `toggle ×${ar3.toggle}, stray signals: ${JSON.stringify(ar3.others)}`,
);

// From toggle, ArrowLeft×3 walks prev, shuffle, then WRAPS to loop (back from
// ring start lands on the last focusable).
for (let k = 0; k < 3; k++) await page.keyboard.press('ArrowLeft');
await page.keyboard.press('Enter');
await page.waitForFunction(() => {
   const dbg = globalThis as DebugGlobal;
   return (dbg.__sig?.loop?.length ?? 0) > 0;
});
const alw = await page.evaluate(() => {
   const sig = (globalThis as DebugGlobal).__sig ?? {};
   return {
      loop: sig.loop?.length ?? 0,
      others: ['shuffle', 'prev', 'next'].filter((n) => (sig[n]?.length ?? 0) > 0),
   };
});
check(
   '(h) ArrowLeft×3 wraps shuffle → loop',
   alw.loop === 1 && alw.others.length === 0,
   `loop ×${alw.loop}, stray signals: ${JSON.stringify(alw.others)}`,
);

// Extra: fresh page, ArrowRight×4 + Enter lands one past toggle — next.
await page.goto(`http://127.0.0.1:${server.port}/`);
await waitUpgrade();
await installRecorder();
await page.evaluate(() => {
   document.getElementById('host')?.focus();
});
for (let k = 0; k < 4; k++) await page.keyboard.press('ArrowRight');
await page.keyboard.press('Enter');
await page.waitForFunction(() => {
   const dbg = globalThis as DebugGlobal;
   return (dbg.__sig?.next?.length ?? 0) > 0;
});
const ar4 = await page.evaluate(() => {
   const sig = (globalThis as DebugGlobal).__sig ?? {};
   return {
      next: sig.next?.length ?? 0,
      others: ['shuffle', 'prev', 'toggle', 'loop'].filter((n) => (sig[n]?.length ?? 0) > 0),
   };
});
check(
   '(h) ArrowRight×4 + Enter → next',
   ar4.next === 1 && ar4.others.length === 0,
   `next ×${ar4.next}, stray signals: ${JSON.stringify(ar4.others)}`,
);

// ------------------------------------------------------------------- (i)
// Fresh load, mouse parked at 0,0: hovering #shuffle eases its bg to moss
// over 140ms ease-out. Sample the button's shadow div bg every ~20ms;
// rAF-tolerant — anchor timing on the first flip, require settling and a
// genuine intermediate value.
await page.goto(`http://127.0.0.1:${server.port}/`);
await waitUpgrade();
const shuf = await nodeCenter('#shuffle');
check('(i) shuffle button located', shuf !== null, `kernel geometry ${JSON.stringify(shuf)}`);
if (shuf) {
   await page.evaluate(
      (geo) => {
         const g = globalThis as DebugGlobal;
         g.__hover = [];
         g.__hoverDone = false;
         const host = document.getElementById('host');
         const t0 = performance.now();
         const sample = () => {
            let bg = 'transparent';
            const hr = host?.getBoundingClientRect();
            if (hr && host?.shadowRoot) {
               // The shuffle row's rect div is identified by kernel geometry:
               // same viewport box as the #shuffle scene node.
               for (const div of host.shadowRoot.querySelectorAll('.slab-ops div')) {
                  const r = div.getBoundingClientRect();
                  if (
                     Math.abs(r.left - (hr.left + geo.x)) < 1.5 &&
                     Math.abs(r.top - (hr.top + geo.y)) < 1.5 &&
                     Math.abs(r.width - geo.w) < 1.5
                  ) {
                     const c = getComputedStyle(div).backgroundColor;
                     if (c && c !== 'rgba(0, 0, 0, 0)') {
                        bg = c;
                        break;
                     }
                  }
               }
            }
            g.__hover?.push({ t: performance.now() - t0, bg });
            if (performance.now() - t0 < 800) setTimeout(sample, 20);
            else g.__hoverDone = true;
         };
         sample();
      },
      { x: shuf.x, y: shuf.y, w: shuf.w },
   );
   await page.mouse.move(shuf.cx, shuf.cy);
   await page.waitForFunction(() => (globalThis as DebugGlobal).__hoverDone === true);
   const hover = await page.evaluate(() => (globalThis as DebugGlobal).__hover ?? []);
   const start = hover[0]?.bg ?? 'missing';
   const last = hover[hover.length - 1]?.bg ?? 'missing';
   const flipAt = hover.find((s) => s.bg !== start)?.t ?? Number.NaN;
   const nearest = (t: number) =>
      hover.reduce((a, b) => (Math.abs(b.t - t) < Math.abs(a.t - t) ? b : a));
   const s40 = nearest(flipAt + 40);
   const s400 = nearest(flipAt + 400);
   const settled = hover.length >= 2 && hover[hover.length - 2]?.bg === last;
   const mid = hover.some((s) => s.bg !== start && s.bg !== last);
   check(
      '(i) hover eases shuffle bg → moss',
      Number.isFinite(flipAt) && s40.bg !== s400.bg && last === 'rgb(27, 46, 34)' && settled && mid,
      `start ${start}; +40ms ${s40.bg}; +400ms ${s400.bg}; final ${last} (moss #1B2E22); flip at ${flipAt.toFixed(0)}ms, ${hover.length} samples, intermediate differs: ${mid}`,
   );
   await page.mouse.move(0, 0);
   await page.waitForFunction(
      (geo) => {
         const host = document.getElementById('host');
         const hostRect = host?.getBoundingClientRect();
         if (!hostRect || !host?.shadowRoot) return false;
         for (const div of host.shadowRoot.querySelectorAll('.slab-ops div')) {
            const rect = div.getBoundingClientRect();
            if (
               Math.abs(rect.left - (hostRect.left + geo.x)) < 1.5 &&
               Math.abs(rect.top - (hostRect.top + geo.y)) < 1.5 &&
               Math.abs(rect.width - geo.w) < 1.5
            ) {
               return getComputedStyle(div).backgroundColor === 'rgba(0, 0, 0, 0)';
            }
         }
         return true;
      },
      { x: shuf.x, y: shuf.y, w: shuf.w },
   );
   check('(i) pointer exit clears hover', true, 'shuffle background returned to transparent');
}

await browser.close();
server.stop();

if (failures > 0) {
   console.error(`\n${failures} assertion(s) FAILED`);
   process.exit(1);
}
console.log('\nall player e2e assertions passed');
