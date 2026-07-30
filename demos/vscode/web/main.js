// Browser host for the Slab VS Code demo.
//
// Tab strips implement multiEditorTabsControl.ts semantics (mousedown switch,
// midpoint reorder, middle-click close, dblclick pin/new). Editor-area drops
// implement editorDropTarget.ts's DropOverlay.positionOverlay verbatim (10%
// edge gates, thirds/halves direction pick, half-rect preview) and grid.ts
// addGroup semantics (same-orientation sibling insert, different-orientation
// wrap, Sizing.Split halves the reference pane). Pane sashes are kernel-owned
// (`splits` container); chrome sashes keep VS Code pixel-pinning on resize.
import './vscode.js'; // registers <slab-vscode> (side effect — keep even if no bindings are used)

const el = document.getElementById('app');

const RED = '#A63D5B';
const GRAY = '#909094';
let uid = 0;
const tab = (name, tint, badge, active = false, kind = '') => ({
   key: name, name, note: 'agentfs', tint, badge, active, hot: false, kind,
});
const leaf = (key, tabs, extra = {}) => ({
   key, leaf: true, horizontal: false, show_mdb: false, show_store: false,
   tabs, children: [], ...extra,
});

/** Editor grid model: one root branch node, VS Code gridview shape.
 * `kind` rides each tab; a leaf's visible content = its active tab's kind. */
let root = {
   key: 'g0', leaf: false, horizontal: true, show_mdb: false, show_store: false, tabs: [],
   children: [
      leaf('gA', [tab('mdb.hpp', RED, '2', true, 'mdb')]),
      leaf('gB', [tab('overlay.hpp', GRAY, ''), tab('store.hpp', RED, '2', true, 'store')]),
   ],
};
let focusedLeaf = 'gB';
let untitled = 0;
let indicator = null; // strip insertion indicator { item, side }
let zone = null; // editor overlay { leaf, dir } — VS Code currentDropOperation

// ------------------------------------------------------------- model ops ---
function* leaves(node = root) {
   if (node.leaf) yield node;
   else for (const c of node.children) yield* leaves(c);
}
function leafOf(item) {
   for (const g of leaves()) if (g.tabs.some((t) => t.key === item)) return g;
   return null;
}
function findLeaf(key, node = root) {
   if (node.leaf) return node.key === key ? node : null;
   for (const c of node.children) { const hit = findLeaf(key, c); if (hit) return hit; }
   return null;
}
function parentOf(target, node = root) {
   for (const c of node.children) {
      if (c === target) return node;
      if (!c.leaf) { const hit = parentOf(target, c); if (hit) return hit; }
   }
   return null;
}

function activate(leafKey, item) {
   // VS Code retains one active editor PER GROUP; activation only changes the
   // target group's active editor and moves group focus.
   const g = findLeaf(leafKey);
   if (!g) return;
   for (const t of g.tabs) t.active = t.key === item;
   focusedLeaf = leafKey;
}

function pruneEmpty() {
   // VS Code closeEmptyGroups: drop empty leaves, collapse single-child branches.
   const walk = (node) => {
      if (node.leaf) return node.tabs.length ? node : null;
      node.children = node.children.map(walk).filter(Boolean);
      if (node.children.length === 1) return node.children[0];
      return node.children.length ? node : null;
   };
   const next = walk(root);
   if (!next) {
      root = { key: 'g0', leaf: false, horizontal: true, show_mdb: false, show_store: false, tabs: [], children: [leaf(`gN${++uid}`, [])] };
   } else if (next.leaf) {
      root = { key: 'g0', leaf: false, horizontal: true, show_mdb: false, show_store: false, tabs: [], children: [next] };
   } else {
      root = next;
   }
}

function closeTab(item) {
   const g = leafOf(item);
   if (!g) return;
   const i = g.tabs.findIndex((t) => t.key === item);
   const wasActive = g.tabs[i].active;
   g.tabs.splice(i, 1);
   if (wasActive && g.tabs.length) activate(g.key, g.tabs[Math.min(i, g.tabs.length - 1)].key);
   pruneEmpty();
}

function takeTab(item) {
   const g = leafOf(item);
   if (!g) return null;
   const i = g.tabs.findIndex((t) => t.key === item);
   return { tab: g.tabs.splice(i, 1)[0], from: g };
}

function moveTabToLeaf(item, leafKey, slot) {
   const target = findLeaf(leafKey);
   const taken = takeTab(item);
   if (!target || !taken) return;
   if (taken.from === target && slot > target.tabs.length) slot = target.tabs.length;
   target.tabs.splice(Math.min(slot, target.tabs.length), 0, taken.tab);
   activate(target.key, item);
   pruneEmpty();
}

/** grid.ts addGroup: LEFT/UP insert before, RIGHT/DOWN after; same-orientation
 * parents take a sibling, different-orientation targets get wrapped. */
function splitLeaf(targetKey, dir, item) {
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
      const wrap = {
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
/** Serialize the model tree to the document's exact list schemas (drops
 * host-only fields like tab.kind, which bulk validation would reject). */
const strip = (node) => ({
   key: node.key, leaf: node.leaf, horizontal: node.horizontal,
   show_mdb: node.show_mdb, show_store: node.show_store,
   tabs: node.tabs.map(({ key, name, note, tint, badge, active, hot }) => ({ key, name, note, tint, badge, active, hot })),
   children: node.children.map(strip),
});
let sceneIndex = { ed: new Map(), zoneRects: new Map(), pane: new Map(), body: new Map(), indl: new Map(), indr: new Map() };

function lastItemSeg(key) {
   const m = [...key.matchAll(/~([^/]+)\//g)];
   return m.length ? decodeURIComponent(m[m.length - 1][1].replace(/%2F/g, '/').replace(/%7E/g, '~').replace(/%25/g, '%')) : null;
}

function reindex() {
   const idx = { ed: new Map(), zoneRects: new Map(), pane: new Map(), body: new Map(), indl: new Map(), indr: new Map() };
   for (const n of el.sceneSnapshot()) {
      const item = lastItemSeg(`${n.key}/`);
      if (!item) continue;
      if (n.key.endsWith('/#ed')) { idx.ed.set(item, { key: n.key, ...n }); }
      else if (n.key.endsWith('/#zone')) idx.zoneRects.set(item, n.key);
      else if (n.key.endsWith('/#body')) idx.body.set(item, { key: n.key, ...n });
      else if (n.key.endsWith('/#indl')) idx.indl.set(item, n.key);
      else if (n.key.endsWith('/#indr')) idx.indr.set(item, n.key);
      else if (/~[^/]+\/stack@0$/.test(n.key)) idx.pane.set(item, { key: n.key, ...n });
   }
   sceneIndex = idx;
}

/** Sizing.Split requests applied only after the model flush has settled, so
 * the fresh pane exists in the scene (drops fire while pushes are deferred). */
let pendingSplits = [];

function pushModel() {
   for (const g of leaves()) {
      const active = g.tabs.find((t) => t.active);
      if (!active && g.tabs.length) g.tabs[0].active = true;
      const shown = g.tabs.find((t) => t.active);
      g.show_mdb = shown?.kind === 'mdb';
      g.show_store = shown?.kind === 'store';
      for (const t of g.tabs) t.hot = t.active && g.key === focusedLeaf;
   }
   el.root = [strip(root)];
   el.whenSettled().then(() => {
      reindex();
      const splits = pendingSplits;
      pendingSplits = [];
      for (const { leafKey, half } of splits) {
         const pane = sceneIndex.pane.get(leafKey);
         if (pane) el.setSplit(pane.key, half);
      }
   });
}

// -------------------------------------------------- strip indicator + zone ---
function setIndicator(next) {
   const rectKey = (ind) => (ind.side === 'before' ? sceneIndex.indl : sceneIndex.indr).get(ind.item);
   const state = (ind) => (ind.side === 'before' ? 'insert-before' : 'insert-after');
   if (indicator && (!next || next.item !== indicator.item || next.side !== indicator.side)) {
      const key = rectKey(indicator);
      if (key) el.setNodeState(key, state(indicator), false);
      indicator = null;
   }
   if (next && !indicator) {
      const key = rectKey(next);
      if (key) el.setNodeState(key, state(next), true);
      indicator = next;
   }
}

function setZone(next) {
   if (zone && (!next || next.leaf !== zone.leaf || next.dir !== zone.dir)) {
      const key = sceneIndex.zoneRects.get(zone.leaf);
      if (key) el.setNodeState(key, `zone-${zone.dir}`, false);
      zone = null;
   }
   if (next && !zone) {
      const key = sceneIndex.zoneRects.get(next.leaf);
      if (key) el.setNodeState(key, `zone-${next.dir}`, true);
      zone = next;
   }
}

/** editorDropTarget.ts positionOverlay, verbatim thresholds.
 * preferSplitVertically = true (openSideBySideDirection 'right' default). */
function zoneFor(rect, x, y) {
   const relX = x - rect.x;
   const relY = y - rect.y;
   const W = rect.w;
   const H = rect.h;
   const edgeW = W * 0.1; // 10% edge threshold: editors dragging
   const edgeH = H * 0.1;
   if (relX > edgeW && relX < W - edgeW && relY > edgeH && relY < H - edgeH) return 'merge';
   if (relX < W / 3) return 'left';
   if (relX > (W / 3) * 2) return 'right';
   if (relY < H / 2) return 'up';
   return 'down';
}

// ------------------------------------------------------------- signals ------
el.addEventListener('tab_press', (e) => {
   const { item, meta } = e.detail;
   if ((meta.hit_key ?? '').includes('/#body/icon') || !item) return;
   const g = leafOf(item);
   if (g) activate(g.key, item);
   pushModel();
});

el.addEventListener('tab_up', (e) => {
   const { item, meta } = e.detail;
   if (meta.button === 1 && item && (meta.hit_key ?? '').includes('~')) {
      closeTab(item);
      pushModel();
   }
});

el.addEventListener('tab_close', (e) => {
   closeTab(e.detail.item);
   pushModel();
});

el.addEventListener('tab_move', (e) => {
   const { item, meta } = e.detail;
   const hit = meta.hit_key ?? '';
   // over a tab body: strip insertion indicator (midpoint rule)
   const overTabItem = hit.includes('/#body') || hit.includes('/#indl') || hit.includes('/#indr')
      ? lastItemSeg(`${hit.slice(0, hit.indexOf('/#')) }/`) : null;
   if (overTabItem && overTabItem !== item) {
      setZone(null);
      const rect = sceneIndex.body.get(overTabItem);
      if (!rect) return setIndicator(null);
      setIndicator({ item: overTabItem, side: meta.x < rect.x + rect.w / 2 ? 'before' : 'after' });
      return;
   }
   setIndicator(null);
   // over an editor body: DropOverlay zones
   const edIdx = hit.indexOf('/#ed');
   if (edIdx >= 0) {
      const leafKey = lastItemSeg(hit.slice(0, edIdx + 1));
      const ed = sceneIndex.ed.get(leafKey);
      const src = leafOf(item);
      if (ed && src) {
         const dir = zoneFor(ed, meta.x, meta.y);
         // VS Code: no drop of an editor onto itself if the source group would empty
         if (src.key === leafKey && src.tabs.length < 2) return setZone(null);
         if (dir === 'merge' && src.key === leafKey) return setZone(null);
         return setZone({ leaf: leafKey, dir });
      }
   }
   setZone(null);
});

el.addEventListener('tab_drop', (e) => {
   const { item, meta } = e.detail;
   const src = meta.src_item;
   if (!src || src === item) return;
   const g = leafOf(item);
   if (!g) return;
   const rect = sceneIndex.body.get(item);
   const before = rect ? meta.x < rect.x + rect.w / 2 : false;
   const i = g.tabs.findIndex((t) => t.key === item);
   moveTabToLeaf(src, g.key, before ? i : i + 1);
   pushModel();
});

el.addEventListener('strip_drop', (e) => {
   const { meta } = e.detail;
   if (!meta.src_item) return;
   const leafKey = lastItemSeg(meta.key.slice(0, meta.key.indexOf('/#strip') + 1));
   if (leafKey) moveTabToLeaf(meta.src_item, leafKey, Number.MAX_SAFE_INTEGER);
   pushModel();
});

el.addEventListener('editor_drop', (e) => {
   const { meta } = e.detail;
   const src = meta.src_item;
   if (!src) return;
   const edIdx = meta.key.indexOf('/#ed');
   const leafKey = lastItemSeg(meta.key.slice(0, edIdx + 1));
   const op = zone; // VS Code currentDropOperation at drop time
   setZone(null);
   if (!leafKey) return;
   const srcLeaf = leafOf(src);
   if (!op || op.leaf !== leafKey) return;
   if (op.dir === 'merge') {
      if (srcLeaf?.key === leafKey) return;
      moveTabToLeaf(src, leafKey, Number.MAX_SAFE_INTEGER);
   } else {
      const ed = sceneIndex.ed.get(leafKey);
      const pane = sceneIndex.pane.get(leafKey);
      const horizontal = op.dir === 'left' || op.dir === 'right';
      const reference = pane ?? ed;
      const half = reference ? (horizontal ? reference.w : reference.h) / 2 : 0;
      const freshKey = splitLeaf(leafKey, op.dir, src);
      if (freshKey && half > 0) {
         // Sizing.Split: both panes take half the reference extent.
         pendingSplits.push({ leafKey: freshKey, half }, { leafKey, half });
      }
   }
   pushModel();
});

el.addEventListener('tab_end', () => { setIndicator(null); setZone(null); });

el.addEventListener('strip_new', (e) => {
   const leafKey = lastItemSeg(e.detail.meta.key.slice(0, e.detail.meta.key.indexOf('/#strip') + 1));
   if (!leafKey) return;
   const g = findLeaf(leafKey);
   untitled += 1;
   const t = tab(`Untitled-${untitled}`, GRAY, '');
   g.tabs.push(t);
   activate(g.key, t.key);
   pushModel();
});

el.addEventListener('tab_dbl', (e) => {
   console.log(`dblclick on ${e.detail.item}: VS Code would pin the tab`);
});

// --------------------------------------------------- chrome selection bits ---
let selectedTreeRow = null;
el.addEventListener('tree_pick', (e) => {
   if (selectedTreeRow) el.setNodeState(selectedTreeRow, 'selected', false);
   selectedTreeRow = e.detail.meta.key;
   el.setNodeState(selectedTreeRow, 'selected', true);
});
let selectedPanelTab = null;
el.addEventListener('panel_pick', (e) => {
   if (selectedPanelTab) el.setNodeState(selectedPanelTab, 'selected', false);
   selectedPanelTab = e.detail.meta.key;
   el.setNodeState(selectedPanelTab, 'selected', true);
});
for (const sig of ['find_change', 'chat_change', 'filter_change']) {
   el.addEventListener(sig, (e) => console.log(`${sig}:`, e.detail.text));
}

// ------------------------------------------------------------ chrome sashes ---
const CHROME_W = 55 + 4 + 4;
const CHROME_H = 28 + 4 + 22 + 3;
const panePx = { sidebar: 195, chat: 198, panel: 178 };

function sceneKey(suffix) {
   for (const n of el.sceneSnapshot()) if (n.key.endsWith(suffix)) return n.key;
   return null;
}

function applyLayout() {
   const w = el.clientWidth;
   const h = el.clientHeight;
   const sashL = sceneKey('#sashL');
   const sashR = sceneKey('#sashR');
   const sashP = sceneKey('#sashP');
   if (!sashL || !sashR || !sashP) return;
   el.setDivider(sashL, panePx.sidebar);
   el.setDivider(sashR, w - CHROME_W - panePx.sidebar - panePx.chat);
   el.setDivider(sashP, h - CHROME_H - panePx.panel);
}

el.addEventListener('sash_sidebar', (e) => { panePx.sidebar = Number.parseFloat(e.detail.text); });
el.addEventListener('sash_center', (e) => {
   panePx.chat = el.clientWidth - CHROME_W - panePx.sidebar - Number.parseFloat(e.detail.text);
});
el.addEventListener('sash_panel', (e) => {
   panePx.panel = el.clientHeight - CHROME_H - Number.parseFloat(e.detail.text);
});
new ResizeObserver(applyLayout).observe(el);

// Model pushes run synchronously from signal handlers: signals dispatch
// after inst_dispatch returns, and keyed re-solves preserve the kernel's
// pointer capture and armed drag (synthetic item ids are key-stable), so a
// mousedown switch repaints immediately — VS Code's switch-on-mousedown.

el.whenSettled().then(() => {
   reindex();
   applyLayout();
   // initial chrome selection: Explorer store.hpp row + OUTPUT panel tab
   for (const n of el.sceneSnapshot()) {
      if (n.key.endsWith('/store.hpp') && n.key.includes('#sidebar')) { selectedTreeRow = n.key; el.setNodeState(n.key, 'selected', true); }
      if (n.key.endsWith('/output')) { selectedPanelTab = n.key; el.setNodeState(n.key, 'selected', true); }
   }
});
pushModel();
window.__model = { get root() { return root; }, leaves: () => [...leaves()].map((g) => `${g.key}[${g.tabs.map((t) => t.key + (t.active ? '*' : '')).join('|')}]`) };
window.__panes = panePx;
