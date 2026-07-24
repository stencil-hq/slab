// Playwright e2e for the generated slab web component (P6 acceptance).
//
//   bun tools/web-e2e.ts
//
// Prereqs:
//   `slab gen wc examples/10-settings.slab -o examples/web-demo/dist --tag slab-settings`
//   `slab gen wc conformance/cases/15-theme.slab -o examples/web-demo/dist --tag slab-theme`
//   `slab gen wc conformance/cases/hole-hug.slab -o examples/web-demo/dist --tag slab-hole-hug`
//   `slab gen wc conformance/cases/edit-multiline.slab -o examples/web-demo/dist --tag slab-edit-multiline`
//   `slab gen wc conformance/cases/16-list.slab -o examples/web-demo/dist --tag slab-list`
//   `slab gen wc conformance/cases/x1-showcase.slab -o examples/web-demo/dist --tag slab-showcase`
//   `slab gen wc conformance/cases/a11y-dynamic.slab -o examples/web-demo/dist --tag slab-a11y-dynamic`
// and a chromium from `bun x playwright install chromium`.
// Serves examples/web-demo over Bun.serve on a random port, then asserts:
//   (a) component upgraded, shadow box count > 10
//   (b) click Save (kernel geometry via __slabDebug) → 'save' CustomEvent
//   (c) click field, type "héllo" → 'draft' events, last text === "héllo",
//       caret div rendered in the shadow DOM
//   (d) narrow host to 500px → `when w<600` patch re-solves client-side
//   (e) emulated dark scheme → `when dark` root background applies
//   (f) 60 slotted rows in the hole, self-sized, scrollable
//   (g) setting the generated theme component's `theme` attribute repaints it
//   (h) a hug hole follows its slotted DOM content after a live resize
//   (i) multiline textarea input inserts once and submit Enter carries text
//   (j) atomic list assignment renders/reflows and signals carry item identity
//   (k) retained semantic DOM values, key identity, rotation, roving focus, click
//   (l) omitted nested list fields atomically clear their child lists

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
   __hugFrames?: { w: number; h: number }[];
}

const root = dirname(import.meta.dir);
const demoDir = join(root, 'examples/web-demo');
const rowMarkup = Array.from(
   { length: 60 },
   (_, index) => `<slab-row slot="rows" label="Row ${index + 1}" tone="#4FC7E0"></slab-row>`,
).join('\n');
const demoHtml = `<!doctype html>
<meta charset="utf-8">
<style>body { margin: 0; } slab-settings { display: block; width: 800px; height: 600px; }</style>
<script type="module" src="/dist/10-settings.js"></script>
<slab-settings id="host">${rowMarkup}</slab-settings>`;

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
      if (path === '/') {
         return new Response(demoHtml, {
            headers: { 'content-type': 'text/html; charset=utf-8' },
         });
      }
      const file = Bun.file(join(demoDir, path));
      if (!(await file.exists())) return new Response('not found', { status: 404 });
      const type = TYPES[extname(path)] ?? 'application/octet-stream';
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
   viewport: { width: 1100, height: 800 },
   colorScheme: 'light',
});
page.on('pageerror', (e) => console.error('pageerror:', e.message));
await page.addInitScript(() => {
   // Read by SlabElement.#registerDebug before elements connect.
   const g = globalThis as { __SLAB_DEBUG__?: boolean };
   g.__SLAB_DEBUG__ = true;
});
await page.goto(`http://127.0.0.1:${server.port}/`);

// ------------------------------------------------------------------- (a)
await page.waitForFunction(() => {
   const host = document.getElementById('host');
   if (!host?.shadowRoot) return false;
   return host.shadowRoot.querySelectorAll('.slab-ops div, .slab-ops span').length > 10;
});
const boxCount = await page.evaluate(() => {
   const host = document.getElementById('host');
   return host?.shadowRoot?.querySelectorAll('.slab-ops div, .slab-ops span').length ?? 0;
});
check('(a) upgrade+paint', boxCount > 10, `${boxCount} shadow-DOM boxes`);

// Record signal CustomEvents.
await page.evaluate(() => {
   const host = document.getElementById('host');
   const g = globalThis as DebugGlobal;
   g.__sig = {};
   for (const name of ['save', 'reset', 'sort', 'draft']) {
      host?.addEventListener(name, (e) => {
         // CustomEvent cast: signal events are always CustomEvents (SlabElement contract).
         const ce = e as CustomEvent;
         const sig = g.__sig ?? {};
         const list = sig[name] ?? [];
         sig[name] = list;
         list.push(ce.detail);
      });
   }
});

/** Viewport-space center of the scene node whose key ends with `suffix`. */
async function nodeCenter(suffix: string, hostId = 'host') {
   return await page.evaluate(
      ({ sfx, id }) => {
         const host = document.getElementById(id);
         if (!host) return null;
         // Debug hook installed by addInitScript + SlabElement.#registerDebug.
         const dbg = globalThis as DebugGlobal;
         const entry = dbg.__slabDebug?.get(host);
         const g = entry?.geom().find((s) => s.key.includes(sfx));
         if (!g) return null;
         const r = host.getBoundingClientRect();
         return { cx: r.left + g.x + g.w / 2, cy: r.top + g.y + g.h / 2, x: g.x, y: g.y, w: g.w };
      },
      { sfx: suffix, id: hostId },
   );
}

/** Scroll a dynamically appended fixture into the browser viewport. */
async function revealHost(hostId: string): Promise<void> {
   await page.evaluate((id) => {
      document.getElementById(id)?.scrollIntoView({ block: 'center' });
   }, hostId);
}

// ------------------------------------------------------------------- (b)
const save = await nodeCenter('#save');
check('(b) save button located', save !== null, `kernel geometry ${JSON.stringify(save)}`);
if (save) {
   await page.mouse.click(save.cx, save.cy);
   await page.waitForFunction(() => {
      const dbg = globalThis as DebugGlobal;
      return (dbg.__sig?.save?.length ?? 0) > 0;
   });
   const detail = await page.evaluate(() => {
      const dbg = globalThis as DebugGlobal;
      return dbg.__sig?.save?.[0];
   });
   const activation = detail as { item?: string; meta?: { key?: string } } | undefined;
   check(
      '(b) save CustomEvent',
      activation?.item === '' && activation.meta?.key?.includes('#save') === true,
      `detail=${JSON.stringify(detail)} (Activate payload)`,
   );
}

// ------------------------------------------------------------------- (c)
const field = await nodeCenter('#field');
check('(c) field located', field !== null, `kernel geometry ${JSON.stringify(field)}`);
if (field) {
   await page.mouse.click(field.cx, field.cy);
   await page.keyboard.type('héllo', { delay: 25 });
   await page.waitForFunction(() => {
      const dbg = globalThis as DebugGlobal;
      const drafts = dbg.__sig?.draft ?? [];
      // Change-signal detail shape per the generated `signals` map.
      const last = drafts[drafts.length - 1] as { text?: string } | undefined;
      return last?.text === 'héllo';
   });
   const drafts = await page.evaluate(() => {
      const dbg = globalThis as DebugGlobal;
      return dbg.__sig?.draft ?? [];
   });
   // Change-signal detail shape per the generated `signals` map.
   const last = drafts[drafts.length - 1] as { text?: string } | undefined;
   check(
      '(c) draft Change signals',
      drafts.length >= 5 && last?.text === 'héllo',
      `${drafts.length} events, last=${JSON.stringify(last)}`,
   );
   const caret = await page.evaluate(() => {
      const host = document.getElementById('host');
      const c = host?.shadowRoot?.querySelector('.slab-caret:not([hidden])');
      return c ? getComputedStyle(c).height : null;
   });
   check('(c) caret rendered', caret !== null, `caret height ${caret}`);
}

// ------------------------------------------------------------------- (d)
const fieldBefore = await nodeCenter('#field');
await page.evaluate(() => {
   const host = document.getElementById('host');
   if (host) host.style.width = '500px';
});
await page.waitForFunction((bx) => {
   const host = document.getElementById('host');
   if (!host) return false;
   const dbg = globalThis as DebugGlobal;
   const entry = dbg.__slabDebug?.get(host);
   const g = entry?.geom().find((s) => s.key.includes('#field'));
   return g !== undefined && g.x !== bx;
}, fieldBefore?.x);
const fieldAfter = await nodeCenter('#field');
check(
   '(d) w<600 patch re-solves',
   fieldBefore !== null && fieldAfter !== null && fieldAfter.x < fieldBefore.x,
   `field x ${fieldBefore?.x} → ${fieldAfter?.x} (pad 24 → 12)`,
);

// ------------------------------------------------------------------- (e)
const bgLight = await page.evaluate(() => {
   const host = document.getElementById('host');
   const rect = host?.shadowRoot?.querySelector('.slab-ops div');
   return rect ? getComputedStyle(rect).backgroundColor : null;
});
await page.emulateMedia({ colorScheme: 'dark' });
await page.waitForFunction((prev) => {
   const host = document.getElementById('host');
   const rect = host?.shadowRoot?.querySelector('.slab-ops div');
   return rect !== null && rect !== undefined && getComputedStyle(rect).backgroundColor !== prev;
}, bgLight);
const bgDark = await page.evaluate(() => {
   const host = document.getElementById('host');
   const rect = host?.shadowRoot?.querySelector('.slab-ops div');
   return rect ? getComputedStyle(rect).backgroundColor : null;
});
check(
   '(e) dark scheme patch',
   bgLight === 'rgb(16, 20, 27)' && bgDark === 'rgb(5, 7, 11)',
   `root bg ${bgLight} → ${bgDark}`,
);

// ------------------------------------------------------------------- (f)
await page.waitForFunction(() => {
   const host = document.getElementById('host');
   const slot = host?.shadowRoot?.querySelector('.slab-hole slot') as HTMLSlotElement | null;
   const first = slot?.assignedElements()[0] as HTMLElement | undefined;
   return first !== undefined && first.offsetHeight > 0 && first.shadowRoot !== null;
});
const rows = await page.evaluate(() => {
   const host = document.getElementById('host');
   const hole = host?.shadowRoot?.querySelector('.slab-hole') as HTMLElement | null;
   const slot = hole?.querySelector('slot') as HTMLSlotElement | null;
   const assigned = slot?.assignedElements() ?? [];
   const first = assigned[0] as HTMLElement | undefined;
   const before = hole ? hole.scrollTop : -1;
   if (hole) hole.scrollTop = 400;
   return {
      count: assigned.length,
      rowH: first?.offsetHeight ?? 0,
      rowPainted: (first?.shadowRoot?.querySelectorAll('.slab-ops *').length ?? 0) > 0,
      holeH: hole?.clientHeight ?? 0,
      scrollH: hole?.scrollHeight ?? 0,
      scrollBefore: before,
      scrollAfter: hole ? hole.scrollTop : -1,
   };
});
check(
   '(f) 60 slotted rows, scrollable',
   rows.count === 60 &&
      rows.rowH > 0 &&
      rows.rowPainted &&
      rows.scrollH > rows.holeH &&
      rows.scrollAfter > rows.scrollBefore,
   JSON.stringify(rows),
);

// ------------------------------------------------------------------- (g)
await page.evaluate(async () => {
   // This generated prerequisite exists only in the served browser realm, so
   // a static import from the Bun-run harness cannot load it.
   await import('/dist/15-theme.js');
   const host = document.createElement('slab-theme');
   host.id = 'theme-host';
   host.style.width = '240px';
   host.style.height = '96px';
   document.body.appendChild(host);
});
await page.waitForFunction(() => {
   const host = document.getElementById('theme-host');
   return host?.shadowRoot?.querySelector('.slab-ops div') !== null;
});
const themeBase = await page.evaluate(() => {
   const host = document.getElementById('theme-host');
   const rect = host?.shadowRoot?.querySelector('.slab-ops div');
   return rect ? getComputedStyle(rect).backgroundColor : null;
});
await page.evaluate(() => {
   document.getElementById('theme-host')?.setAttribute('theme', 'dusk');
});
await page.waitForFunction((base) => {
   const host = document.getElementById('theme-host');
   const rect = host?.shadowRoot?.querySelector('.slab-ops div');
   return rect !== null && rect !== undefined && getComputedStyle(rect).backgroundColor !== base;
}, themeBase);
const themeDusk = await page.evaluate(() => {
   const host = document.getElementById('theme-host');
   const rect = host?.shadowRoot?.querySelector('.slab-ops div');
   return rect ? getComputedStyle(rect).backgroundColor : null;
});
check(
   '(g) theme attribute repaint',
   themeBase === 'rgb(16, 32, 48)' && themeDusk === 'rgb(56, 32, 64)',
   `root bg ${themeBase} → ${themeDusk}`,
);

// ------------------------------------------------------------------- (h)
await page.evaluate(async () => {
   // The generated prerequisite exists only in the served browser realm.
   await import('/dist/hole-hug.js');
   const host = document.createElement('slab-hole-hug');
   host.id = 'hug-host';
   const content = document.createElement('div');
   content.id = 'hug-content';
   content.slot = 'content';
   content.style.cssText = 'display:block;width:50px;height:30px';
   host.appendChild(content);

   const dbg = globalThis as DebugGlobal;
   dbg.__hugFrames = [];
   host.addEventListener('slab-frame', () => {
      const hole = dbg.__slabDebug
         ?.get(host)
         ?.geom()
         .find((geometry) => geometry.key.includes('hole@'));
      if (hole) dbg.__hugFrames?.push({ w: hole.w, h: hole.h });
   });
   document.body.appendChild(host);
});
await page.waitForFunction(() => {
   const frames = (globalThis as DebugGlobal).__hugFrames ?? [];
   const last = frames.at(-1);
   return last !== undefined && last.w === 50 && last.h === 30;
});
const hugInitial = await page.evaluate(() => {
   const frames = (globalThis as DebugGlobal).__hugFrames ?? [];
   return { count: frames.length, rect: frames.at(-1) ?? null };
});
await page.evaluate(() => {
   const content = document.getElementById('hug-content');
   if (content) content.style.cssText = 'display:block;width:80px;height:55px';
});
await page.waitForFunction(({ count, rect }) => {
   if (!rect) return false;
   const later = ((globalThis as DebugGlobal).__hugFrames ?? []).slice(count);
   return later.some((hole) => hole.w > rect.w && hole.h > rect.h);
}, hugInitial);
const hugResized = await page.evaluate(() => {
   const frames = (globalThis as DebugGlobal).__hugFrames ?? [];
   return { count: frames.length, rect: frames.at(-1) ?? null };
});
check(
   '(h) hug hole follows slotted content resize',
   hugInitial.rect?.w === 50 &&
      hugInitial.rect.h === 30 &&
      hugResized.count > hugInitial.count &&
      hugResized.rect?.w === 80 &&
      hugResized.rect.h === 55,
   `${JSON.stringify(hugInitial)} → ${JSON.stringify(hugResized)}`,
);

// ------------------------------------------------------------------- (i)
await page.evaluate(async () => {
   await import('/dist/edit-multiline.js');
   const host = document.createElement('slab-edit-multiline');
   host.id = 'edit-multiline-host';
   host.style.cssText = 'display:block;width:420px;height:220px';
   const g = globalThis as DebugGlobal;
   g.__sig ??= {};
   const sig = g.__sig;
   for (const name of ['draft', 'message', 'send']) {
      sig[`edit-${name}`] = [];
      host.addEventListener(name, (event) => {
         sig[`edit-${name}`].push((event as CustomEvent).detail);
      });
   }
   document.body.appendChild(host);
});
await page.waitForFunction(() => {
   const host = document.getElementById('edit-multiline-host');
   if (!host) return false;
   return (globalThis as DebugGlobal).__slabDebug?.get(host)?.geom().length;
});
await revealHost('edit-multiline-host');
const draftField = await nodeCenter('/draft', 'edit-multiline-host');
check('(i) multiline draft located', draftField !== null, JSON.stringify(draftField));
if (draftField) {
   await page.mouse.click(draftField.cx, draftField.cy);
   await page.keyboard.type('hello');
   await page.keyboard.press('Enter');
   await page.keyboard.type('world');
}
await page.waitForFunction(() => {
   const events = (globalThis as DebugGlobal).__sig?.['edit-draft'] ?? [];
   return (events.at(-1) as { text?: string } | undefined)?.text === 'hello\nworld';
});
const multilineDraft = await page.evaluate(() => {
   const events = (globalThis as DebugGlobal).__sig?.['edit-draft'] ?? [];
   return events.at(-1);
});
check(
   '(i) textarea inserts one newline',
   (multilineDraft as { text?: string } | undefined)?.text === 'hello\nworld',
   JSON.stringify(multilineDraft),
);
const submitField = await nodeCenter('/message', 'edit-multiline-host');
check('(i) multiline submit field located', submitField !== null, JSON.stringify(submitField));
if (submitField) {
   await page.mouse.click(submitField.cx, submitField.cy);
   await page.keyboard.type('ready');
   await page.keyboard.press('Enter');
}
await page.waitForFunction(() => {
   const events = (globalThis as DebugGlobal).__sig?.['edit-send'] ?? [];
   return (events.at(-1) as { text?: string } | undefined)?.text === 'ready';
});
const submitSignals = await page.evaluate(() => {
   const sig = (globalThis as DebugGlobal).__sig ?? {};
   return { changes: sig['edit-message'] ?? [], submits: sig['edit-send'] ?? [] };
});
check(
   '(i) Enter submits without inserting',
   (submitSignals.changes.at(-1) as { text?: string } | undefined)?.text === 'ready' &&
      (submitSignals.submits.at(-1) as { text?: string } | undefined)?.text === 'ready',
   JSON.stringify(submitSignals),
);
const semanticEdit = await page.evaluate(() => {
   const host = document.getElementById('edit-multiline-host');
   const shadow = host?.shadowRoot;
   const ime = shadow?.querySelector<HTMLTextAreaElement>('.slab-ime');
   const field = [
      ...(shadow?.querySelectorAll<HTMLElement>('.slab-a11y-node[tabindex]') ?? []),
   ].find((element) => element.dataset.slabKey?.includes('/draft'));
   field?.focus();
   return { imeTabIndex: ime?.tabIndex, fieldKey: field?.dataset.slabKey ?? '' };
});
await page.waitForFunction(() => {
   const host = document.getElementById('edit-multiline-host');
   const active = host?.shadowRoot?.activeElement as HTMLElement | null | undefined;
   return active?.dataset.slabEditor === 'true' && active.dataset.slabKey?.includes('/draft');
});
await page.keyboard.type('!');
await page.evaluate(() => {
   const host = document.getElementById('edit-multiline-host');
   const active = host?.shadowRoot?.activeElement;
   for (const type of ['compositionstart', 'compositionupdate', 'compositionend']) {
      active?.dispatchEvent(
         new CompositionEvent(type, {
            data: type === 'compositionstart' ? '' : 'Ω',
            bubbles: true,
            composed: true,
         }),
      );
   }
});
await page.waitForFunction(() => {
   const events = (globalThis as DebugGlobal).__sig?.['edit-draft'] ?? [];
   return (events.at(-1) as { text?: string } | undefined)?.text?.endsWith('!Ω') === true;
});
const semanticEditResult = await page.evaluate(() => {
   const host = document.getElementById('edit-multiline-host');
   const active = host?.shadowRoot?.activeElement as HTMLElement | null | undefined;
   const events = (globalThis as DebugGlobal).__sig?.['edit-draft'] ?? [];
   const sends = (globalThis as DebugGlobal).__sig?.['edit-send'] ?? [];
   const before = sends.length;
   const message = [
      ...(host?.shadowRoot?.querySelectorAll<HTMLElement>('.slab-a11y-node[tabindex]') ?? []),
   ].find((element) => element.dataset.slabKey?.includes('/message'));
   message?.click();
   return {
      activeKey: active?.dataset.slabKey ?? '',
      text: (events.at(-1) as { text?: string } | undefined)?.text,
      sendsBefore: before,
      sendsAfter: sends.length,
   };
});
check(
   '(i) semantic field keeps AT focus and forwards text/composition',
   semanticEdit.imeTabIndex === -1 &&
      semanticEdit.fieldKey.includes('/draft') &&
      semanticEditResult.activeKey.includes('/draft') &&
      semanticEditResult.text?.endsWith('!Ω') === true,
   `${JSON.stringify(semanticEdit)} ${JSON.stringify(semanticEditResult)}`,
);
check(
   '(i) semantic field click never synthesizes Enter submit',
   semanticEditResult.sendsAfter === semanticEditResult.sendsBefore,
   JSON.stringify(semanticEditResult),
);

// ------------------------------------------------------------------- (j)
await page.evaluate(async () => {
   await import('/dist/16-list.js');
   const host = document.createElement('slab-list') as HTMLElement & {
      items: { key?: string; label?: string; tone?: string }[];
      setList(name: string, path: string, value: unknown): boolean;
   };
   host.id = 'list-host';
   host.style.cssText = 'display:block;width:360px;height:320px';
   host.items = [
      { key: 'a', label: 'Alpha', tone: '#ff0000' },
      { key: 'b', label: 'Beta', tone: '#00ff00' },
   ];
   const g = globalThis as DebugGlobal;
   g.__sig ??= {};
   const sig = g.__sig;
   sig['list-pick'] = [];
   host.addEventListener('pick', (event) => {
      sig['list-pick'].push((event as CustomEvent).detail);
   });
   document.body.appendChild(host);
});
await page.waitForFunction(() => {
   const host = document.getElementById('list-host');
   if (!host) return false;
   const geom = (globalThis as DebugGlobal).__slabDebug?.get(host)?.geom() ?? [];
   return geom.some((g) => g.key.includes('~a/')) && geom.some((g) => g.key.includes('~b/'));
});
await revealHost('list-host');
const firstItem = await nodeCenter('~a/', 'list-host');
check('(j) assigned list rendered', firstItem !== null, JSON.stringify(firstItem));
if (firstItem) await page.mouse.click(firstItem.cx, firstItem.cy);
await page.waitForFunction(() => {
   const events = (globalThis as DebugGlobal).__sig?.['list-pick'] ?? [];
   return (events.at(-1) as { item?: string } | undefined)?.item === 'a';
});
const listPick = await page.evaluate(() =>
   (globalThis as DebugGlobal).__sig?.['list-pick']?.at(-1),
);
check(
   '(j) list signal carries item',
   (listPick as { item?: string } | undefined)?.item === 'a',
   JSON.stringify(listPick),
);
const listMutation = await page.evaluate(() => {
   const host = document.getElementById('list-host') as HTMLElement & {
      items: { key?: string; label?: string; tone?: string }[];
      setList(name: string, path: string, value: unknown): boolean;
   };
   const rejected = !host.setList('items', '', [
      { key: 'bad', label: 'Bad' },
      { key: 'broken', unknown: true },
   ]);
   host.items = [
      { key: 'a', label: 'Alpha' },
      { key: 'b', label: 'Beta' },
      { key: 'c', label: 'Gamma' },
      { key: 'd', label: 'Delta' },
   ];
   return rejected;
});
await page.waitForFunction(() => {
   const host = document.getElementById('list-host');
   if (!host) return false;
   const geom = (globalThis as DebugGlobal).__slabDebug?.get(host)?.geom() ?? [];
   return ['a', 'b', 'c', 'd'].every((key) => geom.some((g) => g.key.includes(`~${key}/`)));
});
const listLayout = await page.evaluate(() => {
   const host = document.getElementById('list-host');
   if (!host) return [];
   const geom = (globalThis as DebugGlobal).__slabDebug?.get(host)?.geom() ?? [];
   return ['a', 'b', 'c', 'd'].map((key) => geom.find((g) => g.key.includes(`~${key}/`))?.y);
});
check(
   '(j) atomic reject and reassignment relayout',
   listMutation &&
      listLayout.every((y) => typeof y === 'number') &&
      new Set(listLayout).size === listLayout.length,
   JSON.stringify(listLayout),
);

// ------------------------------------------------------------------- (k)
await page.evaluate(async () => {
   // Generated fixtures only exist in the served browser output, not the Bun-run harness.
   await import('/dist/a11y-dynamic.js');
   const host = document.createElement('slab-a11y-dynamic') as HTMLElement & {
      active: string;
      items: Record<string, unknown>[];
      setList(name: string, path: string, value: unknown): boolean;
      getList(name: string, path: string): unknown;
   };
   host.id = 'a11y-host';
   host.style.cssText = 'display:block;width:240px;height:180px';
   host.active = '#list/items~alpha/item';
   host.items = [
      {
         key: 'alpha',
         label: 'Runtime Alpha',
         check: 'mixed',
         chosen: true,
         open: true,
         now: 3,
         announcement: 'assertive',
         position: 1,
         total: 2,
      },
      {
         key: 'beta',
         label: 'Runtime Beta',
         check: 'false',
         chosen: false,
         open: false,
         now: 9,
         announcement: 'off',
         position: 2,
         total: 2,
      },
   ];
   const global = globalThis as DebugGlobal;
   global.__sig ??= {};
   global.__sig['a11y-choose'] = [];
   host.addEventListener('choose', (event) => {
      global.__sig?.['a11y-choose'].push((event as CustomEvent).detail);
   });
   const before = document.createElement('button');
   before.id = 'a11y-before';
   before.textContent = 'before semantics';
   document.body.append(before, host);
});
await page.waitForFunction(() => {
   const host = document.getElementById('a11y-host');
   return host?.shadowRoot?.querySelectorAll('.slab-a11y-node[role="option"]').length === 2;
});
await revealHost('a11y-host');
const semanticInitial = await page.evaluate(() => {
   const host = document.getElementById('a11y-host');
   const shadow = host?.shadowRoot;
   const root = shadow?.querySelector<HTMLElement>('.slab-a11y-node[role="listbox"]');
   const options = [
      ...(shadow?.querySelectorAll<HTMLElement>('.slab-a11y-node[role="option"]') ?? []),
   ];
   const alpha = options.find((element) => element.dataset.slabKey?.includes('~alpha/'));
   const beta = options.find((element) => element.dataset.slabKey?.includes('~beta/'));
   const detail = shadow?.querySelector<HTMLElement>('.slab-a11y-node[role="region"]');
   const rootRect = root?.getBoundingClientRect();
   return {
      optionCount: options.length,
      entry: alpha?.tabIndex,
      other: beta?.tabIndex,
      label: alpha?.getAttribute('aria-label'),
      desc: alpha?.getAttribute('aria-description'),
      checked: alpha?.getAttribute('aria-checked'),
      expanded: alpha?.getAttribute('aria-expanded'),
      selected: alpha?.getAttribute('aria-selected'),
      valueNow: alpha?.getAttribute('aria-valuenow'),
      valueMin: alpha?.getAttribute('aria-valuemin'),
      valueMax: alpha?.getAttribute('aria-valuemax'),
      valueText: alpha?.getAttribute('aria-valuetext'),
      live: alpha?.getAttribute('aria-live'),
      atomic: alpha?.getAttribute('aria-atomic'),
      level: alpha?.getAttribute('aria-level'),
      position: alpha?.getAttribute('aria-posinset'),
      size: alpha?.getAttribute('aria-setsize'),
      activeTarget:
         root?.getAttribute('aria-activedescendant') === alpha?.id && alpha?.id !== undefined,
      controlsTarget:
         root?.getAttribute('aria-controls') === detail?.id && detail?.id !== undefined,
      modal: detail?.getAttribute('aria-modal'),
      rootTransform: root ? getComputedStyle(root).transform : 'none',
      rootOrigin: root ? getComputedStyle(root).transformOrigin : '',
      rotatedWidth: rootRect?.width ?? 0,
      alphaTransform: alpha ? getComputedStyle(alpha).transform : 'none',
      nested: alpha !== undefined && root?.contains(alpha) === true,
   };
});
check(
   '(k) semantic DOM maps typed scene values',
   semanticInitial.optionCount === 2 &&
      semanticInitial.entry === 0 &&
      semanticInitial.other === -1 &&
      semanticInitial.label === 'Runtime Alpha' &&
      semanticInitial.desc === 'Runtime Alpha' &&
      semanticInitial.checked === 'mixed' &&
      semanticInitial.expanded === 'true' &&
      semanticInitial.selected === 'true' &&
      semanticInitial.valueNow === '3' &&
      semanticInitial.valueMin === '0' &&
      semanticInitial.valueMax === '10' &&
      semanticInitial.valueText === 'Runtime Alpha' &&
      semanticInitial.live === 'assertive' &&
      semanticInitial.atomic === 'true' &&
      semanticInitial.level === '1' &&
      semanticInitial.position === '1' &&
      semanticInitial.size === '2' &&
      semanticInitial.activeTarget &&
      semanticInitial.controlsTarget &&
      semanticInitial.modal === 'true',
   JSON.stringify(semanticInitial),
);
check(
   '(k) semantic geometry follows rotation hierarchy',
   semanticInitial.rootTransform !== 'none' &&
      semanticInitial.rootOrigin !== '0px 0px' &&
      semanticInitial.rotatedWidth > 180 &&
      semanticInitial.alphaTransform === 'none' &&
      semanticInitial.nested,
   JSON.stringify(semanticInitial),
);
await page.evaluate(() => {
   const host = document.getElementById('a11y-host') as HTMLElement & {
      instance: { scene_json(): string } | null;
   };
   const inst = host.instance;
   if (!inst) return;
   const parsed: unknown = JSON.parse(inst.scene_json());
   if (!Array.isArray(parsed)) return;
   const option = parsed.find(
      (node) =>
         node !== null && typeof node === 'object' && 'role' in node && node.role === 'option',
   );
   if (!option || typeof option !== 'object') return;
   Object.defineProperty(inst, 'scene_json', {
      configurable: true,
      value: () => JSON.stringify([...parsed, { ...option, label: 'Duplicate occurrence' }]),
   });
   host.style.width = '241px';
});
await page.waitForFunction(() => {
   const host = document.getElementById('a11y-host');
   return host?.shadowRoot?.querySelectorAll('.slab-a11y-node[role="option"]').length === 3;
});
const duplicateSemantics = await page.evaluate(() => {
   const host = document.getElementById('a11y-host');
   const shadow = host?.shadowRoot;
   const root = shadow?.querySelector<HTMLElement>('.slab-a11y-node[role="listbox"]');
   const alpha = [
      ...(shadow?.querySelectorAll<HTMLElement>('.slab-a11y-node[role="option"]') ?? []),
   ].filter((element) => element.dataset.slabKey?.includes('~alpha/'));
   return {
      count: alpha.length,
      ids: alpha.map((element) => element.id),
      canonical: root?.getAttribute('aria-activedescendant') === alpha[0]?.id,
   };
});
check(
   '(k) duplicate scene keys retain distinct canonical semantic nodes',
   duplicateSemantics.count === 2 &&
      new Set(duplicateSemantics.ids).size === 2 &&
      duplicateSemantics.canonical,
   JSON.stringify(duplicateSemantics),
);
await page.evaluate(() => {
   const host = document.getElementById('a11y-host') as HTMLElement & {
      instance: { scene_json(): string } | null;
   };
   if (host.instance) Reflect.deleteProperty(host.instance, 'scene_json');
   host.style.width = '240px';
});
await page.waitForFunction(() => {
   const host = document.getElementById('a11y-host');
   return host?.shadowRoot?.querySelectorAll('.slab-a11y-node[role="option"]').length === 2;
});
await page.evaluate(() => document.getElementById('a11y-before')?.focus());
await page.keyboard.press('Tab');
await page.waitForFunction(() => {
   const host = document.getElementById('a11y-host');
   const active = host?.shadowRoot?.activeElement as HTMLElement | null | undefined;
   return active?.dataset.slabKey?.includes('~alpha/') === true;
});
check('(k) keyboard enters the initial semantic tab stop', true, 'alpha focused');

const emptyKeyResult = await page.evaluate(() => {
   const host = document.getElementById('a11y-host') as HTMLElement & {
      setList(name: string, path: string, value: unknown): boolean;
      getList(name: string, path: string): unknown;
   };
   const before = JSON.stringify(host.getList('items', ''));
   const emptyAccepted = host.setList('items', '', [{ key: '', label: 'Invalid' }]);
   const duplicateAccepted = host.setList('items', '', [
      { key: 'same', label: 'First' },
      { key: 'same', label: 'Second' },
   ]);
   return {
      emptyAccepted,
      duplicateAccepted,
      unchanged: JSON.stringify(host.getList('items', '')) === before,
   };
});
check(
   '(k) invalid list keys reject atomically',
   !emptyKeyResult.emptyAccepted && !emptyKeyResult.duplicateAccepted && emptyKeyResult.unchanged,
   JSON.stringify(emptyKeyResult),
);

const firstSemanticIdentity = await page.evaluate(() => {
   const host = document.getElementById('a11y-host');
   const semantic = [
      ...(host?.shadowRoot?.querySelectorAll<HTMLElement>('.slab-a11y-node[role="option"]') ?? []),
   ].find((element) => element.dataset.slabKey?.includes('~alpha/'));
   const geometry = (globalThis as DebugGlobal).__slabDebug
      ?.get(host as Element)
      ?.geom()
      .find((node) => node.key.includes('~alpha/'));
   semantic?.click();
   return { id: semantic?.id ?? '', node: geometry?.node ?? -1 };
});
await page.waitForFunction(() => {
   const events = (globalThis as DebugGlobal).__sig?.['a11y-choose'] ?? [];
   return (events.at(-1) as { item?: string } | undefined)?.item === 'alpha';
});
check(
   '(k) semantic programmatic click activates',
   firstSemanticIdentity.id !== '',
   JSON.stringify(firstSemanticIdentity),
);
const rejectedSemanticClick = await page.evaluate(() => {
   const host = document.getElementById('a11y-host');
   const detail = host?.shadowRoot?.querySelector<HTMLElement>('.slab-a11y-node[role="region"]');
   const events = (globalThis as DebugGlobal).__sig?.['a11y-choose'] ?? [];
   const before = events.length;
   detail?.click();
   return { before, after: events.length };
});
check(
   '(k) semantic click aborts when keyed focus is rejected',
   rejectedSemanticClick.after === rejectedSemanticClick.before,
   JSON.stringify(rejectedSemanticClick),
);

await page.evaluate(() => {
   const host = document.getElementById('a11y-host') as HTMLElement & {
      items: Record<string, unknown>[];
   };
   host.items = [{ key: 'beta', label: 'Runtime Beta' }];
});
await page.waitForFunction(() => {
   const host = document.getElementById('a11y-host');
   return ![...(host?.shadowRoot?.querySelectorAll<HTMLElement>('.slab-a11y-node') ?? [])].some(
      (element) => element.dataset.slabKey?.includes('~alpha/'),
   );
});
await page.evaluate(() => {
   const host = document.getElementById('a11y-host') as HTMLElement & {
      items: Record<string, unknown>[];
   };
   host.items = [
      { key: 'alpha', label: 'Runtime Alpha', check: 'mixed', chosen: true, open: true },
      { key: 'beta', label: 'Runtime Beta' },
   ];
});
await page.waitForFunction(() => {
   const host = document.getElementById('a11y-host');
   return [...(host?.shadowRoot?.querySelectorAll<HTMLElement>('.slab-a11y-node') ?? [])].some(
      (element) => element.dataset.slabKey?.includes('~alpha/'),
   );
});
const secondSemanticIdentity = await page.evaluate(() => {
   const host = document.getElementById('a11y-host');
   const semantic = [
      ...(host?.shadowRoot?.querySelectorAll<HTMLElement>('.slab-a11y-node[role="option"]') ?? []),
   ].find((element) => element.dataset.slabKey?.includes('~alpha/'));
   const geometry = (globalThis as DebugGlobal).__slabDebug
      ?.get(host as Element)
      ?.geom()
      .find((node) => node.key.includes('~alpha/'));
   return { id: semantic?.id ?? '', node: geometry?.node ?? -1 };
});
check(
   '(k) stable key owns DOM id across synthetic id change',
   secondSemanticIdentity.id === firstSemanticIdentity.id &&
      secondSemanticIdentity.node !== firstSemanticIdentity.node,
   `${JSON.stringify(firstSemanticIdentity)} -> ${JSON.stringify(secondSemanticIdentity)}`,
);

await page.evaluate(() => {
   const host = document.getElementById('a11y-host');
   const beta = [
      ...(host?.shadowRoot?.querySelectorAll<HTMLElement>('.slab-a11y-node[role="option"]') ?? []),
   ].find((element) => element.dataset.slabKey?.includes('~beta/'));
   beta?.focus();
});
await page.waitForFunction(() => {
   const host = document.getElementById('a11y-host');
   const options = [
      ...(host?.shadowRoot?.querySelectorAll<HTMLElement>('.slab-a11y-node[role="option"]') ?? []),
   ];
   const alpha = options.find((element) => element.dataset.slabKey?.includes('~alpha/'));
   const beta = options.find((element) => element.dataset.slabKey?.includes('~beta/'));
   return alpha?.tabIndex === -1 && beta?.tabIndex === 0;
});
check('(k) semantic focus uses one roving tab stop', true, 'beta=0, alpha=-1');

// ------------------------------------------------------------------- (l)
await page.evaluate(async () => {
   await import('/dist/x1-showcase.js');
   const host = document.createElement('slab-showcase');
   host.id = 'showcase-host';
   host.style.width = '800px';
   host.style.height = '600px';
   document.body.appendChild(host);
});
await page.waitForFunction(() => {
   const host = document.getElementById('showcase-host');
   return (host?.shadowRoot?.childElementCount ?? 0) > 0;
});
const showcaseLoaded = await page.evaluate(() => {
   const host = document.getElementById('showcase-host');
   return host !== null;
});
check('(l) showcase loaded', showcaseLoaded, 'showcase element rendered');
const nestedReplay = await page.evaluate(() => {
   interface ShowcaseHost extends HTMLElement {
      setList(name: string, path: string, value: unknown): boolean;
      getList(name: string, path: string): unknown;
      loadSlir(bytes: Uint8Array): boolean;
   }
   const host = document.getElementById('showcase-host') as ShowcaseHost;
   const rootAccepted = host.setList('commits', '', [
      {
         key: '0',
         hash: 'a1b2c3d',
         msg: 'Replay root',
         author: 'alice',
         active: true,
         tags: [{ key: 'root', label: 'Stale root child', tone: '#9ca3af' }],
         lines: [],
      },
   ]);
   const childAccepted = host.setList('commits', '0.tags', [
      { key: 'direct-a', label: 'Direct child A', tone: '#22c55e' },
      { key: 'direct-b', label: 'Direct child B', tone: '#3b82f6' },
   ]);
   const ctor = host.constructor;
   if (!('slir' in ctor)) return { rootAccepted, childAccepted, loaded: false, cached: null };
   const encoded = ctor.slir;
   if (typeof encoded !== 'string' && !(encoded instanceof Uint8Array)) {
      return { rootAccepted, childAccepted, loaded: false, cached: null };
   }
   const bytes =
      typeof encoded === 'string'
         ? Uint8Array.from(atob(encoded), (character) => character.charCodeAt(0))
         : encoded;
   const loaded = host.loadSlir(bytes);
   return { rootAccepted, childAccepted, loaded, cached: host.getList('commits', '0.tags') };
});
await page.waitForFunction(() => {
   const host = document.getElementById('showcase-host');
   const text = host?.shadowRoot?.querySelector('.slab-ops')?.textContent ?? '';
   return text.includes('Direct child A') && text.includes('Direct child B');
});
check(
   '(l) live swap replays parent before direct nested writes',
   nestedReplay.rootAccepted &&
      nestedReplay.childAccepted &&
      nestedReplay.loaded &&
      JSON.stringify(nestedReplay.cached).includes('Direct child B'),
   JSON.stringify(nestedReplay),
);
const nestedListResult = await page.evaluate(() => {
   const host = document.getElementById('showcase-host') as HTMLElement & {
      setList(name: string, path: string, value: unknown): boolean;
   };
   const before = [
      ...(host.shadowRoot?.querySelectorAll<HTMLElement>('.slab-a11y-node') ?? []),
   ].filter((element) => {
      const key = element.dataset.slabKey ?? '';
      return key.includes('commits~0/') && key.includes('/tags~');
   }).length;
   const accepted = host.setList('commits', '', [
      {
         key: '0',
         hash: 'a1b2c3d',
         msg: 'Omitted nested lists clear',
         author: 'alice',
         active: true,
      },
   ]);
   return { accepted, before };
});
await page.waitForFunction(() => {
   const host = document.getElementById('showcase-host');
   const nodes = [...(host?.shadowRoot?.querySelectorAll<HTMLElement>('.slab-a11y-node') ?? [])];
   return !nodes.some((element) => {
      const key = element.dataset.slabKey ?? '';
      return key.includes('commits~0/') && key.includes('/tags~');
   });
});
const nestedTagsAfter = await page.evaluate(
   () =>
      [
         ...(document
            .getElementById('showcase-host')
            ?.shadowRoot?.querySelectorAll<HTMLElement>('.slab-a11y-node') ?? []),
      ].filter((element) => {
         const key = element.dataset.slabKey ?? '';
         return key.includes('commits~0/') && key.includes('/tags~');
      }).length,
);
check(
   '(l) omitted nested lists clear existing children',
   nestedListResult.accepted && nestedListResult.before >= 2 && nestedTagsAfter === 0,
   `${JSON.stringify(nestedListResult)} -> ${nestedTagsAfter}`,
);

await browser.close();
server.stop();

if (failures > 0) {
   console.error(`\n${failures} assertion(s) FAILED`);
   process.exit(1);
}
console.log('\nall web e2e assertions passed');
