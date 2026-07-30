#!/usr/bin/env bun
/**
 * SDP host for demos/vscode/vscode.slab — the scripted twin of web/main.js.
 *
 * Drives `slab drive` over stdio with the same VS Code contracts:
 *   - multiEditorTabsControl.ts strip semantics (mousedown switch, midpoint
 *     reorder, middle-click close, dblclick pin/new-tab, X close)
 *   - editorDropTarget.ts DropOverlay.positionOverlay zones (10% edge gates,
 *     thirds/halves, half-rect preview via zone-* node states)
 *   - grid.ts addGroup splits (sibling insert / orientation wrap,
 *     Sizing.Split via inst_set_split on pane scene keys)
 *
 * Run: bun demos/vscode/host.ts   (renders /tmp/slab-grid-step-*.png)
 */
import { DriveClient } from '../../packages/dslab/src/index.ts';

type Tab = {
   key: string; name: string; note: string; tint: string; badge: string;
   active: boolean; hot: boolean; kind: string;
};
type Group = {
   key: string; leaf: boolean; horizontal: boolean;
   show_mdb: boolean; show_store: boolean; tabs: Tab[]; children: Group[];
};

const RED = '#A63D5B';
const GRAY = '#909094';
let uid = 0;
const tab = (name: string, tint: string, badge: string, active = false, kind = ''): Tab =>
   ({ key: name, name, note: 'agentfs', tint, badge, active, hot: false, kind });
const leaf = (key: string, tabs: Tab[]): Group =>
   ({ key, leaf: true, horizontal: false, show_mdb: false, show_store: false, tabs, children: [] });

let root: Group = {
   key: 'g0', leaf: false, horizontal: true, show_mdb: false, show_store: false, tabs: [],
   children: [
      leaf('gA', [tab('mdb.hpp', RED, '2', true, 'mdb')]),
      leaf('gB', [tab('overlay.hpp', GRAY, ''), tab('store.hpp', RED, '2', true, 'store')]),
   ],
};
let focusedLeaf = 'gB';
let indicator: { item: string; side: 'before' | 'after' } | null = null;
let zone: { leaf: string; dir: string } | null = null;
let pendingSplits: Array<{ leafKey: string; half: number }> = [];

const client = DriveClient.launch({
   // The prebuilt binary keeps the NDJSON stream clean; `cargo run` can
   // interleave build chatter into stdout and corrupt framing.
   executable: './target/debug/slab',
   args: ['drive', 'demos/vscode/vscode.slab'],
   cwd: new URL('../..', import.meta.url).pathname,
});

// ------------------------------------------------------------- model ops ---
function* leaves(node: Group = root): Generator<Group> {
   if (node.leaf) yield node;
   else for (const c of node.children) yield* leaves(c);
}
const leafOf = (item: string) => [...leaves()].find((g) => g.tabs.some((t) => t.key === item)) ?? null;
const findLeaf = (key: string) => [...leaves()].find((g) => g.key === key) ?? null;
function parentOf(target: Group, node: Group = root): Group | null {
   for (const c of node.children) {
      if (c === target) return node;
      if (!c.leaf) { const hit = parentOf(target, c); if (hit) return hit; }
   }
   return null;
}
function activate(leafKey: string, item: string) {
   const g = findLeaf(leafKey);
   if (!g) return;
   for (const t of g.tabs) t.active = t.key === item;
   focusedLeaf = leafKey;
}
function pruneEmpty() {
   const walk = (node: Group): Group | null => {
      if (node.leaf) return node.tabs.length ? node : null;
      node.children = node.children.map(walk).filter((c): c is Group => !!c);
      if (node.children.length === 1) return node.children[0];
      return node.children.length ? node : null;
   };
   const next = walk(root);
   root = !next || next.leaf
      ? { ...root, children: next ? [next] : [leaf(`gN${++uid}`, [])] }
      : next;
}
function takeTab(item: string) {
   const g = leafOf(item);
   if (!g) return null;
   return { tab: g.tabs.splice(g.tabs.findIndex((t) => t.key === item), 1)[0], from: g };
}
function moveTabToLeaf(item: string, leafKey: string, slot: number) {
   const target = findLeaf(leafKey);
   const taken = takeTab(item);
   if (!target || !taken) return;
   target.tabs.splice(Math.min(slot, target.tabs.length), 0, taken.tab);
   activate(target.key, item);
   pruneEmpty();
}
function closeTab(item: string) {
   const g = leafOf(item);
   if (!g) return;
   const i = g.tabs.findIndex((t) => t.key === item);
   const wasActive = g.tabs[i].active;
   g.tabs.splice(i, 1);
   if (wasActive && g.tabs.length) activate(g.key, g.tabs[Math.min(i, g.tabs.length - 1)].key);
   pruneEmpty();
}
/** grid.ts addGroup semantics. Returns the fresh leaf key. */
function splitLeaf(targetKey: string, dir: string, item: string): string | null {
   const target = findLeaf(targetKey);
   const taken = takeTab(item);
   if (!target || !taken) return null;
   const horizontal = dir === 'left' || dir === 'right';
   const before = dir === 'left' || dir === 'up';
   const fresh = leaf(`gN${++uid}`, [taken.tab]);
   const parent = parentOf(target);
   if (parent && parent.horizontal === horizontal) {
      const i = parent.children.indexOf(target);
      parent.children.splice(before ? i : i + 1, 0, fresh);
   } else {
      const wrap: Group = {
         key: `gW${++uid}`, leaf: false, horizontal, show_mdb: false, show_store: false, tabs: [],
         children: before ? [fresh, target] : [target, fresh],
      };
      if (parent) parent.children.splice(parent.children.indexOf(target), 1, wrap);
      else root = wrap;
   }
   activate(fresh.key, item);
   pruneEmpty();
   return fresh.key;
}

// ------------------------------------------------------------ scene index ---
type Rect = { x: number; y: number; w: number; h: number };
let scene: Array<{ key: string; x: number; y: number; w: number; h: number }> = [];
const lastItemSeg = (key: string) => {
   const m = [...key.matchAll(/~([^/]+)\//g)];
   return m.length ? m[m.length - 1][1] : null;
};
async function refreshScene() {
   scene = (await client.call('scene.tree')).nodes as typeof scene;
}
const findRect = (pred: (k: string) => boolean): (Rect & { key: string }) | null => {
   for (const n of scene) if (n.key && pred(n.key)) return n;
   return null;
};
const bodyRect = (item: string) => findRect((k) => k.includes(`~${item}/`) && k.endsWith('/#body'));
const edRect = (leafKey: string) => findRect((k) => k.includes(`~${leafKey}/`) && k.endsWith('/#ed'));
const paneRect = (leafKey: string) => findRect((k) => new RegExp(`~${leafKey.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}/stack@0$`).test(k));

/** Serialize to the document schemas (drops host-only `kind`). */
const serialize = (n: Group): Record<string, unknown> => ({
   key: n.key, leaf: n.leaf, horizontal: n.horizontal,
   show_mdb: n.show_mdb, show_store: n.show_store,
   tabs: n.tabs.map(({ key, name, note, tint, badge, active, hot }) => ({ key, name, note, tint, badge, active, hot })),
   children: n.children.map(serialize),
});

async function pushModel() {
   for (const g of leaves()) {
      if (!g.tabs.some((t) => t.active) && g.tabs.length) g.tabs[0].active = true;
      const shown = g.tabs.find((t) => t.active);
      g.show_mdb = shown?.kind === 'mdb';
      g.show_store = shown?.kind === 'store';
      for (const t of g.tabs) t.hot = t.active && g.key === focusedLeaf;
   }
   await client.call('param.set', { sets: { root: [serialize(root) as never] } });
   await refreshScene();
   const splits = pendingSplits;
   pendingSplits = [];
   for (const { leafKey, half } of splits) {
      const pane = paneRect(leafKey);
      // `split.set` is the SDP mirror of SlabElement.setSplit; absent on older
      // servers, in which case the kernel's equal-share insertion seeding holds.
      if (pane) await client.request('split.set', { key: pane.key, size: half }).catch(() => null);
   }
}

// ------------------------------------------------------------- indicators ---
async function setNodeState(key: string, name: string, on: boolean) {
   await client.call('state.node', { key, name, on });
}
async function setIndicator(next: typeof indicator) {
   const rectKey = (ind: NonNullable<typeof indicator>) =>
      findRect((k) => k.includes(`~${ind.item}/`) && k.endsWith(ind.side === 'before' ? '/#indl' : '/#indr'))?.key ?? null;
   if (indicator && (!next || next.item !== indicator.item || next.side !== indicator.side)) {
      const key = rectKey(indicator);
      if (key) await setNodeState(key, indicator.side === 'before' ? 'insert-before' : 'insert-after', false);
      indicator = null;
   }
   if (next && !indicator) {
      const key = rectKey(next);
      if (key) await setNodeState(key, next.side === 'before' ? 'insert-before' : 'insert-after', true);
      indicator = next;
   }
}
async function setZone(next: typeof zone) {
   const zoneKey = (leafKey: string) =>
      findRect((k) => k.includes(`~${leafKey}/`) && k.endsWith('/#zone'))?.key ?? null;
   if (zone && (!next || next.leaf !== zone.leaf || next.dir !== zone.dir)) {
      const key = zoneKey(zone.leaf);
      if (key) await setNodeState(key, `zone-${zone.dir}`, false);
      zone = null;
   }
   if (next && !zone) {
      const key = zoneKey(next.leaf);
      if (key) await setNodeState(key, `zone-${next.dir}`, true);
      zone = next;
   }
}

/** editorDropTarget.ts positionOverlay thresholds, verbatim. */
function zoneFor(rect: Rect, x: number, y: number): string {
   const relX = x - rect.x;
   const relY = y - rect.y;
   const { w: W, h: H } = rect;
   if (relX > W * 0.1 && relX < W * 0.9 && relY > H * 0.1 && relY < H * 0.9) return 'merge';
   if (relX < W / 3) return 'left';
   if (relX > (W / 3) * 2) return 'right';
   return relY < H / 2 ? 'up' : 'down';
}

// --------------------------------------------------------------- signals ----
type Sig = { name: string; item: string; meta: Record<string, unknown> };
async function onSignals(signals: Sig[]) {
   for (const sig of signals) {
      const item = sig.item;
      const hit = String(sig.meta.hit_key ?? '');
      const button = Number(sig.meta.button ?? 0);
      const metaKey = String(sig.meta.key ?? '');
      switch (sig.name) {
         case 'tab_press': {
            if (hit.includes('/#body/icon') || !item) break;
            const g = leafOf(item);
            if (g) { activate(g.key, item); await pushModel(); }
            break;
         }
         case 'tab_up':
            if (button === 1 && item && hit.includes('~')) { closeTab(item); await pushModel(); }
            break;
         case 'tab_close':
            closeTab(item);
            await pushModel();
            break;
         case 'tab_move': {
            await refreshScene(); // zones need live geometry, not last-push geometry
            const overTab = /#body|#indl|#indr/.test(hit) ? lastItemSeg(`${hit.slice(0, hit.indexOf('/#'))}/`) : null;
            if (overTab && overTab !== item) {
               await setZone(null);
               const rect = bodyRect(overTab);
               if (rect) await setIndicator({ item: overTab, side: Number(sig.meta.x) < rect.x + rect.w / 2 ? 'before' : 'after' });
               break;
            }
            await setIndicator(null);
            const edIdx = hit.indexOf('/#ed');
            if (edIdx >= 0) {
               const leafKey = lastItemSeg(hit.slice(0, edIdx + 1));
               const ed = leafKey ? edRect(leafKey) : null;
               const src = leafOf(item);
               if (ed && src && leafKey) {
                  const dir = zoneFor(ed, Number(sig.meta.x), Number(sig.meta.y));
                  if (src.key === leafKey && (src.tabs.length < 2 || dir === 'merge')) { await setZone(null); break; }
                  await setZone({ leaf: leafKey, dir });
                  break;
               }
            }
            await setZone(null);
            break;
         }
         case 'tab_drop': {
            const src = String(sig.meta.src_item ?? '');
            if (!src || src === item) break;
            const g = leafOf(item);
            if (!g) break;
            const rect = bodyRect(item);
            const before = rect ? Number(sig.meta.x) < rect.x + rect.w / 2 : false;
            const i = g.tabs.findIndex((t) => t.key === item);
            moveTabToLeaf(src, g.key, before ? i : i + 1);
            await pushModel();
            break;
         }
         case 'strip_drop': {
            const src = String(sig.meta.src_item ?? '');
            const leafKey = lastItemSeg(metaKey.slice(0, metaKey.indexOf('/#strip') + 1));
            if (src && leafKey) { moveTabToLeaf(src, leafKey, Number.MAX_SAFE_INTEGER); await pushModel(); }
            break;
         }
         case 'editor_drop': {
            const src = String(sig.meta.src_item ?? '');
            const leafKey = lastItemSeg(metaKey.slice(0, metaKey.indexOf('/#ed') + 1));
            const op = zone;
            await setZone(null);
            if (!src || !leafKey || !op || op.leaf !== leafKey) break;
            if (op.dir === 'merge') {
               if (leafOf(src)?.key !== leafKey) moveTabToLeaf(src, leafKey, Number.MAX_SAFE_INTEGER);
            } else {
               const pane = paneRect(leafKey);
               const horizontal = op.dir === 'left' || op.dir === 'right';
               const half = pane ? (horizontal ? pane.w : pane.h) / 2 : 0;
               const freshKey = splitLeaf(leafKey, op.dir, src);
               if (freshKey && half > 0) pendingSplits.push({ leafKey: freshKey, half }, { leafKey, half });
            }
            await pushModel();
            break;
         }
         case 'tab_end':
            await setIndicator(null);
            await setZone(null);
            break;
         case 'strip_new': {
            const leafKey = lastItemSeg(metaKey.slice(0, metaKey.indexOf('/#strip') + 1));
            const g = leafKey ? findLeaf(leafKey) : null;
            if (!g) break;
            const t = tab(`Untitled-${++uid}`, GRAY, '');
            g.tabs.push(t);
            activate(g.key, t.key);
            await pushModel();
            break;
         }
         default:
            break;
      }
   }
}

type PointerType = 'move' | 'down' | 'up';
async function pointer(type: PointerType, x: number, y: number, button = 0, clicks = 1) {
   const result = await client.call('input.pointer', { type, x, y, button, clicks });
   const effects = result.effects as { signals?: Sig[] };
   await onSignals(effects.signals ?? []);
   return result;
}
async function shot(step: string) {
   const path = `/tmp/slab-grid-${step}.png`;
   await client.call('render.png', { path });
   console.log(`  -> ${path}`);
}
const order = () => [...leaves()].map((g) => `${g.key}[${g.tabs.map((t) => t.key + (t.active ? '*' : '')).join('|')}]`).join('  ');
const center = (r: Rect) => ({ x: r.x + r.w / 2, y: r.y + r.h / 2 });
async function dragTo(from: { x: number; y: number }, to: { x: number; y: number }) {
   await refreshScene();
   await pointer('down', from.x, from.y);
   for (let i = 1; i <= 8; i++) await pointer('move', from.x + ((to.x - from.x) * i) / 8, from.y + ((to.y - from.y) * i) / 8);
   await pointer('up', to.x, to.y);
}

// --------------------------------------------------------------- scenario ---
await client.call('env.set', { width: 1600, height: 900 });
await pushModel();
console.log('baseline:', order());
await shot('0-baseline');

console.log('\n1. split RIGHT (overlay.hpp -> right band of gB)');
{
   await refreshScene();
   const ed = edRect('gB');
   const from = center(bodyRect('overlay.hpp')!);
   if (ed) await dragTo(from, { x: ed.x + ed.w * 0.96, y: ed.y + ed.h / 2 });
   console.log('  ', order());
   await shot('1-split-right');
}

console.log('\n2. split UP (mdb.hpp -> top band of gB) — vertical wrap');
{
   await refreshScene();
   const ed = edRect('gB');
   const from = center(bodyRect('mdb.hpp')!);
   if (ed) await dragTo(from, { x: ed.x + ed.w / 2, y: ed.y + ed.h * 0.04 });
   console.log('  ', order());
   await shot('2-split-up');
}

console.log('\n3. merge (store.hpp -> center of the overlay group)');
{
   await refreshScene();
   const target = leafOf('overlay.hpp');
   const ed = target ? edRect(target.key) : null;
   const from = center(bodyRect('store.hpp')!);
   if (ed) await dragTo(from, { x: ed.x + ed.w / 2, y: ed.y + ed.h / 2 });
   console.log('  ', order());
   await shot('3-merge');
}

await client.call('protocol.quit').catch(() => {});
await client.close();
console.log('\ndone');
