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
import { FILES, CONTENTS, visibleRows, DEFAULT_OPEN, highlight } from './fsmodel.js';

// Editor line height; must match the doc's 12px/1.5 JetBrains Mono metrics.
const LINE_H = 18;
const el = document.getElementById('app');

const RED = '#A63D5B';
const GRAY = '#909094';
const PROBLEMS = [
   { key: 'include/agentfs/mdb.hpp|47', icon: 'errc', file: 'include/agentfs/mdb.hpp', line: '47', msg: "mdb.hpp(47,18): use of undeclared identifier 'MDB_CREATE'", tint: '#A63D5B' },
   { key: 'include/agentfs/mdb.hpp|68', icon: 'errc', file: 'include/agentfs/mdb.hpp', line: '68', msg: "mdb.hpp(68,11): no matching function for call to 'mdb_txn_begin'", tint: '#A63D5B' },
   { key: 'include/agentfs/store.hpp|92', icon: 'errc', file: 'include/agentfs/store.hpp', line: '92', msg: "store.hpp(92,7): unknown type name 'MDB_cursor'", tint: '#A63D5B' },
   { key: 'include/agentfs/store.hpp|118', icon: 'errc', file: 'include/agentfs/store.hpp', line: '118', msg: 'store.hpp(118,24): member reference base type is not a structure or union', tint: '#A63D5B' },
   { key: 'include/agentfs/mdb.hpp|31', icon: 'warn', file: 'include/agentfs/mdb.hpp', line: '31', msg: "mdb.hpp(31,9): unused variable 'flags'", tint: '#D9A33C' },
   { key: 'include/agentfs/store.hpp|143', icon: 'warn', file: 'include/agentfs/store.hpp', line: '143', msg: 'store.hpp(143,16): implicit conversion changes signedness', tint: '#D9A33C' },
];
let uid = 0;
const tab = (name, tint, badge, active = false, kind = 'edit') => ({
   key: name, name, note: 'agentfs', tint, badge, active, hot: false,
   preview: false, dirty: false, kind,
});
const leaf = (key, tabs, extra = {}) => ({
   key, leaf: true, horizontal: false, show_mdb: false, show_store: false,
   show_edit: false, show_find: false, find_status: 'No results', curline: 0, curline_on: false, gutter: '', tabs, children: [], ...extra,
});

/** Editor grid model: one root branch node, VS Code gridview shape.
 * `kind` rides each tab; a leaf's visible content = its active tab's kind. */
let root = {
   key: 'g0', leaf: false, horizontal: true, show_mdb: false, show_store: false,
   show_edit: false, show_find: false, gutter: '', tabs: [],
   children: [
      leaf('gA', [tab('mdb.hpp', RED, '2', true)], { show_find: true }),
      leaf('gB', [tab('overlay.hpp', GRAY, ''), tab('store.hpp', RED, '2', true)]),
   ],
};
let focusedLeaf = 'gB';
const curlineByLeaf = new Map();
let untitled = 0;
const closedStack = [];
let indicator = null; // strip insertion indicator { item, side }
let zone = null; // editor overlay { leaf, dir } — VS Code currentDropOperation
let draggingTab = null;
const openDirs = new Set(DEFAULT_OPEN);
const lastSeeded = new Map();
let selectedPath = 'include/agentfs/store.hpp';
const navHistory = [{ path: selectedPath }];
let navIndex = 0;
let navigatingHistory = false;

function syncNav() {
   el.setParam('nav.canback', navIndex > 0);
   el.setParam('nav.canfwd', navIndex < navHistory.length - 1);
}

function recordNav(path) {
   if (navigatingHistory || !path || navHistory[navIndex]?.path === path) return;
   navHistory.splice(navIndex + 1);
   navHistory.push({ path });
   navIndex = navHistory.length - 1;
   syncNav();
}

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
const basenameIndex = new Map();
(function indexFiles(nodes) {
   for (const n of nodes) {
      if (n.dir) indexFiles(n.children ?? []);
      else basenameIndex.set(n.name, n.path);
   }
})(FILES);
function pathOfTab(t) {
   if (!t) return '';
   if (t.key.includes('/') || t.key.startsWith('untitled:')) return t.key;
   // Seed tabs use bare basenames; resolve them against the FS model so
   // every editor (not just mdb/store) finds its CONTENTS entry.
   return basenameIndex.get(t.key) ?? t.key;
}

function activePathOf(g) {
   return pathOfTab(g?.tabs.find((t) => t.active));
}
function lineCount(content) {
   return content.split('\n').length;
}

function gutterForPath(path) {
   if (!path) return '';
   return Array.from({ length: lineCount(CONTENTS[path] ?? '') }, (_, i) => `${i + 1}`).join('\n');
}

function languageForPath(path) {
   const name = path.split('/').at(-1) ?? '';
   if (name === 'CMakeLists.txt') return 'CMake';
   if (name.endsWith('.hpp') || name.endsWith('.cpp')) return 'C++';
   if (name.endsWith('.md')) return 'Markdown';
   if (name.endsWith('.sh')) return 'Shell Script';
   return 'Plain Text';
}

function pushTree() {
   el.tree = visibleRows(openDirs).map((r) => ({
      key: r.key, name: r.name, letter: r.letter, icon: r.icon, tint: r.tint,
      badge: r.badge, indent: r.indent, dir: r.dir, open: r.open,
   }));
}

function activate(leafKey, item) {
   // VS Code retains one active editor PER GROUP; activation only changes the
   // target group's active editor and moves group focus.
   const g = findLeaf(leafKey);
   if (!g) return;
   for (const t of g.tabs) t.active = t.key === item;
   focusedLeaf = leafKey;
}
function revealLine(leafKey, path, line, selection = null) {
   if (line === null || !Number.isFinite(line)) return;
   const targetLine = Math.max(1, Math.trunc(line));
   // pushModel seeds newly activated editors in its first settled callback.
   // Wait once more so caret and scroll offsets target the new field content.
   el.whenSettled().then(() => el.whenSettled()).then(() => {
      if (activePathOf(findLeaf(leafKey)) !== path) return;
      const snapshot = el.sceneSnapshot();
      const field = snapshot.find((n) => n.editable === true
         && n.key.includes('/#edscroll/')
         && leafKeyFromEditorKey(n.key) === leafKey);
      const scroll = snapshot.find((n) => n.key.endsWith('/#edscroll')
         && leafKeyFromEditorKey(n.key) === leafKey);
      const text = CONTENTS[path] ?? '';
      let offset = 0;
      for (let current = 1; current < targetLine && offset < text.length; current += 1) {
         const newline = text.indexOf('\n', offset);
         offset = newline < 0 ? text.length : newline + 1;
      }
      if (field && el.setCaret) {
         const sel = selection && typeof selection === 'object'
            ? selection
            : { caret: selection ?? offset, anchor: selection ?? offset };
         el.setCaret(field.key, sel.caret, sel.anchor ?? sel.caret);
      }
      if (scroll) el.setScroll(scroll.key, 0, (targetLine - 1) * LINE_H);
      el.setParam('status.caret', `Ln ${targetLine}, Col 1`);
   });
}

function openFile(path, { preview = false, line = null } = {}) {
   const kind = 'edit';
   const key = path.endsWith('include/agentfs/mdb.hpp')
      ? 'mdb.hpp'
      : path.endsWith('include/agentfs/store.hpp') ? 'store.hpp' : path;
   for (const g of leaves()) {
      const existing = g.tabs.find((t) => t.key === key);
      if (existing) {
         if (!preview) existing.preview = false;
         selectedPath = path;
         activate(g.key, key);
         pushModel();
         revealLine(g.key, path, line);
         return;
      }
   }

   const g = findLeaf(focusedLeaf) ?? leaves().next().value;
   if (!g) return;
   const parts = path.split('/');
   const untitledNumber = key.startsWith('untitled:') ? key.slice('untitled:'.length) : '';
   const name = untitledNumber ? `Untitled-${untitledNumber}` : parts.at(-1) ?? path;
   const note = untitledNumber ? '' : parts.length > 1 ? parts.at(-2) : '';
   const previewTab = preview ? g.tabs.find((t) => t.preview) : null;
   if (previewTab) {
      Object.assign(previewTab, {
         key, name, note, tint: GRAY, badge: '', kind, preview: true, dirty: false,
      });
   } else {
      g.tabs.push({
         key, name, note, tint: GRAY, badge: '', active: true, hot: false,
         preview, dirty: false, kind,
      });
   }
   selectedPath = path;
   activate(g.key, key);
   pushModel();
   revealLine(g.key, path, line);
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
      root = { key: 'g0', leaf: false, horizontal: true, show_mdb: false, show_store: false, show_edit: false, show_find: false, gutter: '', tabs: [], children: [leaf(`gN${++uid}`, [])] };
   } else if (next.leaf) {
      root = { key: 'g0', leaf: false, horizontal: true, show_mdb: false, show_store: false, show_edit: false, show_find: false, gutter: '', tabs: [], children: [next] };
   } else {
      root = next;
   }
}

function closeTab(item) {
   const g = leafOf(item);
   if (!g) return;
   const i = g.tabs.findIndex((t) => t.key === item);
   const closed = g.tabs[i];
   closedStack.push({ path: pathOfTab(closed), leafKey: g.key, index: i });
   const wasActive = g.tabs[i].active;
   g.tabs.splice(i, 1);
   if (wasActive && g.tabs.length) activate(g.key, g.tabs[Math.min(i, g.tabs.length - 1)].key);
   pruneEmpty();
}

function reopenLastClosed() {
   const closed = closedStack.pop();
   if (!closed) return;
   openFile(closed.path, { preview: false });
   const reopenedLeaf = [...leaves()].find((g) =>
      g.tabs.some((t) => pathOfTab(t) === closed.path));
   const reopened = reopenedLeaf?.tabs.find((t) => pathOfTab(t) === closed.path);
   const target = findLeaf(closed.leafKey);
   if (target && reopened && moveTabToLeaf(reopened.key, target.key, closed.index)) pushModel();
   recordNav(closed.path);
}

function takeTab(item) {
   const g = leafOf(item);
   if (!g) return null;
   const index = g.tabs.findIndex((t) => t.key === item);
   if (index < 0) return null;
   return { tab: g.tabs.splice(index, 1)[0], from: g, index };
}

function moveTabToLeaf(item, leafKey, slot) {
   const target = findLeaf(leafKey);
   if (!target) return false;
   const taken = takeTab(item);
   if (!taken) return false;
   // The drop slot is measured in the pre-removal strip. Moving a tab right
   // within its own group shifts every later slot left by one.
   if (taken.from === target && taken.index < slot) slot -= 1;
   target.tabs.splice(Math.max(0, Math.min(slot, target.tabs.length)), 0, taken.tab);
   activate(target.key, item);
   pruneEmpty();
   return true;
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
         key: `gW${++uid}`, leaf: false, horizontal, show_mdb: false, show_store: false, show_edit: false, show_find: false, gutter: '', tabs: [],
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
   show_mdb: node.show_mdb, show_store: node.show_store, show_edit: node.show_edit,
   show_find: node.show_find, find_status: node.find_status ?? 'No results', curline: node.curline ?? 0, curline_on: node.curline_on ?? false, gutter: node.gutter,
   crumbs: (node.crumbs ?? []).map(({ key, seg, letter, last }) => ({ key, seg, letter, last })),
   tabs: node.tabs.map(({ key, name, note, tint, badge, active, hot, preview, dirty }) => ({ key, name, note, tint, badge, active, hot, preview, dirty })),
   children: node.children.map(strip),
});
let sceneIndex = { ed: new Map(), zoneRects: new Map(), pane: new Map(), body: new Map(), indl: new Map(), indr: new Map() };

function lastItemSeg(key) {
   const m = [...key.matchAll(/~([^/]+)\//g)];
   return m.length ? decodeURIComponent(m[m.length - 1][1].replace(/%2F/g, '/').replace(/%7E/g, '~').replace(/%25/g, '%')) : null;
}
function leafKeyFromEditorKey(key) {
   const edIdx = key.indexOf('/#ed');
   return edIdx >= 0 ? lastItemSeg(key.slice(0, edIdx + 1)) : null;
}

function trailingKey(key) {
   return decodeURIComponent(key.slice(key.lastIndexOf('/') + 1));
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
const dirtyPaths = new Set();
let scmRows = [];

function pushScmChanges() {
   dirtyPaths.clear();
   for (const g of leaves()) {
      for (const t of g.tabs) {
         if (t.dirty) dirtyPaths.add(pathOfTab(t));
      }
   }
   const rows = [...dirtyPaths].map((path) => ({
      key: path,
      file: path.split('/').at(-1) ?? path,
      badge: 'M',
   }));
   const unchanged = rows.length === scmRows.length
      && rows.every((row, i) =>
         row.key === scmRows[i].key
         && row.file === scmRows[i].file
         && row.badge === scmRows[i].badge);
   if (unchanged) return;
   scmRows = rows;
   el.setList('scm.changes', '', rows);
}


function pushModel() {
   for (const g of leaves()) {
      const active = g.tabs.find((t) => t.active);
      if (!active && g.tabs.length) g.tabs[0].active = true;
      const shown = g.tabs.find((t) => t.active);
      const path = pathOfTab(shown);
      const segments = path ? path.split('/') : [];
      g.crumbs = segments.map((seg, i) => ({
         key: i,
         seg,
         letter: i === segments.length - 1
            ? (seg.endsWith('.hpp') ? 'h' : (seg.endsWith('.cpp') || seg.endsWith('.c') ? 'C' : ''))
            : '',
         last: i === segments.length - 1,
      }));
      g.show_mdb = path.endsWith('include/agentfs/mdb.hpp');
      g.show_store = path.endsWith('include/agentfs/store.hpp');
      g.show_edit = !!shown;
      g.gutter = gutterForPath(path);
      for (const t of g.tabs) t.hot = t.active && g.key === focusedLeaf;
   }
   el.setParam('status.lang', languageForPath(activePathOf(findLeaf(focusedLeaf))));
   el.root = [strip(root)];
   pushScmChanges();
   el.whenSettled().then(() => {
      reindex();
      const snapshot = el.sceneSnapshot();
      const editorFields = snapshot.filter((n) => n.editable === true && n.key.includes('/#ed'));
      const editorScrolls = snapshot.filter((n) => n.key.endsWith('/#edscroll'));
      for (const g of leaves()) {
         const path = activePathOf(g);
         if (!path || lastSeeded.get(g.key) === path) continue;
         const field = editorFields.find((n) => leafKeyFromEditorKey(n.key) === g.key);
         if (!field) continue;
         const content = CONTENTS[path] ?? '';
         el.setFieldText(field.key, content);
         if (el.setFieldStyles) el.setFieldStyles(field.key, highlight(content));
         lastSeeded.set(g.key, path);
         // Scroll only after the reseeded field has been measured, or the
         // offset clamps against the previous content extent.
         const scroll = editorScrolls.find((n) => leafKeyFromEditorKey(n.key) === g.key);
         if (scroll) {
            const off = path === 'include/agentfs/mdb.hpp' ? 38 * LINE_H : 0;
            el.whenSettled().then(() => el.setScroll(scroll.key, 0, off));
         }
      }
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
   const active = g?.tabs.find((t) => t.active);
   recordNav(pathOfTab(active));
   // VS Code switches editors on mousedown. The root-list write is keyed and
   // item keys are stable, so the kernel re-solve preserves pointer capture
   // and the armed drag source (verified: reorder/cross-group/split drags).
   pushModel();
});

el.addEventListener('tab_up', (e) => {
   const { item, meta } = e.detail;
   if (meta.button === 1 && item && (meta.hit_key ?? '').includes('~')) {
      closeTab(item);
      pushModel();
   } else if (meta.button === 0 && item && !draggingTab) {
      pushModel();
   }
});

el.addEventListener('tab_close', (e) => {
   closeTab(e.detail.item);
   pushModel();
});

el.addEventListener('tab_drag', (e) => {
   draggingTab = e.detail.item;
   // A cancelled prior drag must never leak chrome into the next operation.
   setIndicator(null);
   setZone(null);
});

el.addEventListener('tab_move', (e) => {
   const { item, meta } = e.detail;
   const hit = meta.hit_key ?? '';
   // over a tab body: strip insertion indicator (midpoint rule)
   const tabMarker = ['/#body', '/#indl', '/#indr'].find((marker) => hit.includes(marker));
   const overTabItem = tabMarker
      ? lastItemSeg(hit.slice(0, hit.indexOf(tabMarker) + 1)) : null;
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
   if (i < 0) return;
   if (moveTabToLeaf(src, g.key, before ? i : i + 1)) pushModel();
});

el.addEventListener('strip_drop', (e) => {
   const { meta } = e.detail;
   const stripIdx = meta.key.indexOf('/#strip');
   if (!meta.src_item || stripIdx < 0) return;
   const leafKey = lastItemSeg(meta.key.slice(0, stripIdx + 1));
   if (leafKey && moveTabToLeaf(meta.src_item, leafKey, Number.MAX_SAFE_INTEGER)) pushModel();
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

el.addEventListener('tab_end', (e) => {
   setIndicator(null);
   setZone(null);
   draggingTab = null;
   if (!e.detail.meta.dropped) pushModel();
});

el.addEventListener('strip_dbl', (e) => {
   const { meta } = e.detail;
   const hit = meta.hit_key ?? meta.key;
   if (!hit.endsWith('/#strip')) return;
   const stripIdx = meta.key.indexOf('/#strip');
   if (stripIdx < 0) return;
   const leafKey = lastItemSeg(meta.key.slice(0, stripIdx + 1));
   const g = findLeaf(leafKey);
   if (!g) return;
   untitled += 1;
   const key = `untitled:${untitled}`;
   const t = tab(`Untitled-${untitled}`, GRAY, '', false, 'edit');
   t.key = key;
   CONTENTS[key] = '';
   g.tabs.push(t);
   activate(g.key, key);
   pushModel();
});

el.addEventListener('tab_dbl', (e) => {
   const g = leafOf(e.detail.item);
   const t = g?.tabs.find((candidate) => candidate.key === e.detail.item);
   if (!t || !t.preview) return;
   t.preview = false;
   pushModel();
});

const quickOpenFiles = [];
for (const entry of FILES) {
   const visit = (node) => {
      if (node.dir) {
         for (const child of node.children) visit(child);
         return;
      }
      const slash = node.path.lastIndexOf('/');
      quickOpenFiles.push({
         key: node.path,
         name: node.name,
         dir: slash < 0 ? '' : node.path.slice(0, slash),
         letter: node.letter,
      });
   };
   visit(entry);
}
let quickOpenRows = [];

function rankedQuickOpenFiles() {
   const byPath = new Map(quickOpenFiles.map((file) => [file.key, file]));
   const byName = new Map(quickOpenFiles.map((file) => [file.name, file]));
   const ranked = [];
   const seen = new Set();
   for (const group of leaves()) {
      for (const openTab of group.tabs) {
         const file = byPath.get(pathOfTab(openTab)) ?? byName.get(openTab.name);
         if (!file || seen.has(file.key)) continue;
         seen.add(file.key);
         ranked.push(file);
      }
   }
   for (const file of quickOpenFiles) {
      if (seen.has(file.key)) continue;
      seen.add(file.key);
      ranked.push(file);
   }
   return ranked;
}

function isSubsequence(needle, candidate) {
   let index = 0;
   for (const char of candidate) {
      if (char === needle[index]) index += 1;
      if (index === needle.length) return true;
   }
   return needle.length === 0;
}

function setQuickOpenQuery(query) {
   const needle = query.toLocaleLowerCase();
   quickOpenRows = rankedQuickOpenFiles()
      .filter((file) => isSubsequence(
         needle,
         `${file.name} ${file.dir}`.toLocaleLowerCase(),
      ))
      .slice(0, 12)
      .map((file, index) => ({ ...file, selected: index === 0 }));
   el.setParam('qo.sel', 0);
   el.setList('qo.rows', '', quickOpenRows);
}

function closeQuickOpen() {
   el.setParam('qo.open', false);
}

function openQuickOpen() {
   menuTarget = null;
   el.setParam('menu.open', false);
   const existingField = el.sceneSnapshot().find((node) =>
      node.editable === true && node.key.includes('/#quickopen/'));
   if (existingField) el.setFieldText(existingField.key, '');
   el.setParam('qo.open', true);
   setQuickOpenQuery('');
   el.whenSettled().then(() => requestAnimationFrame(() => {
      // Keyboard text lands only when the element itself holds DOM focus
      // (a fresh page still has focus on <body>).
      el.focus();
      const field = el.sceneSnapshot().find((node) =>
         node.editable === true && node.key.includes('/#quickopen/'));
      if (field) el.setFocus(field.key, false);
   }));
}

function moveQuickOpenSelection(delta) {
   if (!quickOpenRows.length) return;
   const current = Math.max(0, Math.min(
      quickOpenRows.length - 1,
      Number(el.getParam('qo.sel')) || 0,
   ));
   const next = (current + delta + quickOpenRows.length) % quickOpenRows.length;
   quickOpenRows = quickOpenRows.map((row, index) => ({
      ...row,
      selected: index === next,
   }));
   el.setParam('qo.sel', next);
   el.setList('qo.rows', '', quickOpenRows);
}

function pickQuickOpen(path) {
   if (!path) return;
   openFile(path, { preview: false });
   recordNav(path);
   closeQuickOpen();
}

el.addEventListener('qo_change', (e) => setQuickOpenQuery(e.detail.text));
el.addEventListener('qo_pick', (e) => pickQuickOpen(e.detail.item));

let menuTarget = null;
el.addEventListener('tab_menu', (e) => {
   const { item, meta } = e.detail;
   const stripIndex = meta.key.indexOf('/#strip');
   const leafKey = stripIndex >= 0 ? lastItemSeg(meta.key.slice(0, stripIndex + 1)) : null;
   const targetTab = findLeaf(leafKey)?.tabs.find((candidate) => candidate.key === item);
   if (!leafKey || !targetTab) return;
   menuTarget = { leafKey, tabKey: item };
   el.setParam('menu.anchor', meta.key);
   el.setList('menu.items', '', [
      { key: 'close', label: 'Close' },
      { key: 'closeOthers', label: 'Close Others' },
      { key: 'closeSaved', label: 'Close Saved' },
      { key: 'pin', label: targetTab.preview ? 'Keep Open' : 'Pin' },
      { key: 'closeAll', label: 'Close All' },
   ]);
   el.setParam('menu.open', true);
});

el.addEventListener('menu_pick', (e) => {
   const target = menuTarget;
   const g = target ? findLeaf(target.leafKey) : null;
   const targetTab = g?.tabs.find((candidate) => candidate.key === target.tabKey);
   if (g && targetTab) {
      switch (e.detail.item) {
         case 'close':
            closeTab(target.tabKey);
            break;
         case 'closeOthers':
            g.tabs = [targetTab];
            activate(g.key, target.tabKey);
            break;
         case 'closeSaved':
            g.tabs = g.tabs.filter((candidate) => candidate.dirty);
            if (g.tabs.length && !g.tabs.some((candidate) => candidate.active)) {
               activate(g.key, g.tabs[0].key);
            }
            pruneEmpty();
            break;
         case 'pin':
            targetTab.preview = false;
            break;
         case 'closeAll':
            g.tabs = [];
            pruneEmpty();
            break;
      }
      pushModel();
   }
   menuTarget = null;
   el.setParam('menu.open', false);
});

window.addEventListener('pointerdown', (e) => {
   const snapshot = el.sceneSnapshot();
   if (el.getParam('menu.open')) {
      const menu = snapshot.find((node) => node.key.endsWith('/#menu'));
      const insideMenu = menu
         && e.clientX >= menu.x && e.clientX <= menu.x + menu.w
         && e.clientY >= menu.y && e.clientY <= menu.y + menu.h;
      if (!insideMenu) {
         menuTarget = null;
         el.setParam('menu.open', false);
      }
   }
   if (el.getParam('qo.open')) {
      const quickOpen = snapshot.find((node) => node.key.endsWith('/#quickopen'));
      const insideQuickOpen = quickOpen
         && e.clientX >= quickOpen.x && e.clientX <= quickOpen.x + quickOpen.w
         && e.clientY >= quickOpen.y && e.clientY <= quickOpen.y + quickOpen.h;
      if (!insideQuickOpen) closeQuickOpen();
   }
}, true);

window.addEventListener('keydown', (e) => {
   const key = e.key.toLocaleLowerCase();
   // Handled global chords must not reach SlabElement: the kernel binds its
   // own editing commands (e.g. Cmd+W word_back) and dispatches even
   // default-prevented keydowns.
   const consume = () => { e.preventDefault(); e.stopImmediatePropagation(); };
   if (e.ctrlKey && key === 'tab') {
      consume();
      const g = findLeaf(focusedLeaf);
      if (!g?.tabs.length) return;
      const activeIndex = g.tabs.findIndex((t) => t.active);
      const next = g.tabs[(activeIndex + 1) % g.tabs.length];
      selectedPath = pathOfTab(next);
      activate(g.key, next.key);
      recordNav(selectedPath);
      pushModel();
      return;
   }
   const reopenShortcut = (e.metaKey || e.ctrlKey)
      && e.shiftKey && !e.altKey && key === 't';
   if (reopenShortcut) {
      consume();
      reopenLastClosed();
      return;
   }
   const quickOpenShortcut = (e.metaKey || e.ctrlKey)
      && !e.altKey && !e.shiftKey && key === 'p';
   if (quickOpenShortcut) {
      consume();
      openQuickOpen();
      return;
   }
   if (el.getParam('qo.open')) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
         consume();
         moveQuickOpenSelection(e.key === 'ArrowDown' ? 1 : -1);
      } else if (e.key === 'Enter') {
         consume();
         pickQuickOpen(quickOpenRows[Number(el.getParam('qo.sel')) || 0]?.key);
      } else if (e.key === 'Escape') {
         consume();
         closeQuickOpen();
      }
      return;
   }
   if (e.key !== 'Escape' || !el.getParam('menu.open')) return;
   consume();
   menuTarget = null;
   el.setParam('menu.open', false);
}, true);

// --------------------------------------------------- chrome selection bits ---
let selectedTreeRow = null;
let lastTreePick = { path: '', at: 0 };
el.addEventListener('tree_pick', (e) => {
   const path = e.detail.item;
   const row = visibleRows(openDirs).find((r) => r.key === path);
   if (!row) return;
   if (row.dir) {
      if (openDirs.has(path)) openDirs.delete(path);
      else openDirs.add(path);
      pushTree();
      return;
   }
   selectedPath = path;
   if (selectedTreeRow) el.setNodeState(selectedTreeRow, 'selected', false);
   selectedTreeRow = e.detail.meta.key;
   el.setNodeState(selectedTreeRow, 'selected', true);
   const now = performance.now();
   const preview = lastTreePick.path !== path || now - lastTreePick.at > 450;
   lastTreePick = { path, at: now };
   openFile(path, { preview });
   recordNav(path);
});

el.addEventListener('sidebar_search_change', (e) => {
   const query = e.detail.text;
   if (query.length < 2) {
      el.setList('search.results', '', []);
      return;
   }
   const needle = query.toLocaleLowerCase();
   const rows = [];
   for (const [path, content] of Object.entries(CONTENTS)) {
      const lines = content.split('\n');
      for (let index = 0; index < lines.length; index += 1) {
         if (!lines[index].toLocaleLowerCase().includes(needle)) continue;
         const line = index + 1;
         rows.push({
            key: `${path}|${line}`,
            file: path.split('/').at(-1) ?? path,
            line: String(line),
            preview: lines[index].trim().slice(0, 60),
         });
         if (rows.length === 40) break;
      }
      if (rows.length === 40) break;
   }
   el.setList('search.results', '', rows);
});

const EXTENSIONS = [
   { name: 'C/C++', publisher: 'ms-vscode.cpptools' },
   { name: 'CMake Tools', publisher: 'ms-vscode.cmake-tools' },
   { name: 'GitLens', publisher: 'eamodio.gitlens' },
];

el.addEventListener('ext_search_change', (e) => {
   const needle = e.detail.text.toLocaleLowerCase();
   const filtered = EXTENSIONS.filter(({ name, publisher }) =>
      `${name} ${publisher}`.toLocaleLowerCase().includes(needle));
   el.setList('ext.rows', '', filtered);
});

el.addEventListener('problems_filter_change', (e) => {
   el.setParam('problems.filtering', e.detail.text.length > 0);
   const needle = e.detail.text.toLocaleLowerCase();
   const filtered = PROBLEMS.filter(({ file, msg }) =>
      `${file} ${msg}`.toLocaleLowerCase().includes(needle));
   el.setList('problems.rows', '', filtered);
});

el.addEventListener('search_pick', (e) => {
   const split = e.detail.item.lastIndexOf('|');
   if (split < 0) return;
   const path = e.detail.item.slice(0, split);
   const line = Number(e.detail.item.slice(split + 1));
   openFile(path, { line });
   recordNav(path);
});

el.addEventListener('scm_pick', (e) => {
   openFile(e.detail.item);
});

let selectedActivityItem = null;
const sidebarViews = ['explorer', 'search', 'scm', 'debug', 'ext'];
function selectActivity(key, itemKey = sceneKey(`/${key}`)) {
   if (!sidebarViews.includes(key)) return;
   if (selectedActivityItem) el.setNodeState(selectedActivityItem, 'selected', false);
   selectedActivityItem = itemKey;
   if (selectedActivityItem) el.setNodeState(selectedActivityItem, 'selected', true);
   for (const view of sidebarViews) el.setParam(`sidebar.${view}`, view === key);
   restoreSidebar();
}

el.addEventListener('activity_pick', (e) => {
   selectActivity(trailingKey(e.detail.meta.key), e.detail.meta.key);
});

let selectedPanelTab = null;
const panelTabs = ['problems', 'output', 'debugc', 'terminal', 'ports'];
function selectPanel(key, itemKey = sceneKey(`/${key}`)) {
   if (!panelTabs.includes(key)) return;
   if (selectedPanelTab) el.setNodeState(selectedPanelTab, 'selected', false);
   selectedPanelTab = itemKey;
   if (selectedPanelTab) el.setNodeState(selectedPanelTab, 'selected', true);
   for (const panel of panelTabs) el.setParam(`panel.${panel}`, panel === key);
}

el.addEventListener('panel_pick', (e) => {
   showPanel(trailingKey(e.detail.meta.key), e.detail.meta.key);
});
el.addEventListener('status_problems', showProblemsPanel);

el.addEventListener('problem_pick', (e) => {
   const item = String(e.detail.item ?? trailingKey(e.detail.meta.key));
   const split = item.lastIndexOf('|');
   if (split < 0) return;
   const path = item.slice(0, split);
   const line = Number(item.slice(split + 1));
   openFile(path, { line });
   recordNav(path);
});

el.addEventListener('nav_back', () => {
   if (navIndex <= 0) return;
   navIndex -= 1;
   navigatingHistory = true;
   try {
      openFile(navHistory[navIndex].path);
   } finally {
      navigatingHistory = false;
   }
   syncNav();
});

el.addEventListener('nav_fwd', () => {
   if (navIndex >= navHistory.length - 1) return;
   navIndex += 1;
   navigatingHistory = true;
   try {
      openFile(navHistory[navIndex].path);
   } finally {
      navigatingHistory = false;
   }
   syncNav();

});
el.addEventListener('code_change', (e) => {
   const leafKey = leafKeyFromEditorKey(e.detail.meta.key);
   const g = findLeaf(leafKey);
   const activeTab = g?.tabs.find((t) => t.active);
   const path = pathOfTab(activeTab);
   if (!path) return;
   // Seeding via setFieldText echoes a kernel Change with identical text;
   // only real user edits may pin the preview tab or mark it dirty.
   if (e.detail.text === (CONTENTS[path] ?? '')) return;
   const previousLines = lineCount(CONTENTS[path] ?? '');
   CONTENTS[path] = e.detail.text;
   if (el.setFieldStyles) el.setFieldStyles(e.detail.meta.key, highlight(e.detail.text));
   const tabChanged = activeTab.preview || !activeTab.dirty;
   activeTab.preview = false;
   activeTab.dirty = true;
   if (tabChanged || lineCount(e.detail.text) !== previousLines) pushModel();
});

function updateCaretStatus() {
   const key = el.focusedKey();
   if (!key?.includes('/#edscroll/')) return;
   const state = el.getCaret(key);
   const text = el.fieldText(key);
   if (!state || text === undefined) return;
   const before = text.slice(0, state.caret);
   const lastNewline = before.lastIndexOf('\n');
   const line = before.split('\n').length;
   const col = state.caret - lastNewline;
   el.setParam('status.caret', `Ln ${line}, Col ${col}`);

   const leafKey = leafKeyFromEditorKey(key);
   if (!leafKey) return;
   // Group focus follows editor focus (VS Code: clicking into an editor
   // makes its group the target of Cmd+W/S/F and the hot-tab underline).
   if (focusedLeaf !== leafKey) {
      focusedLeaf = leafKey;
      pushModel();
   }
   const curline = (line - 1) * LINE_H;
   let changed = false;
   for (const g of leaves()) {
      const curlineOn = g.key === leafKey;
      if (curlineOn) {
         const cached = curlineByLeaf.get(g.key);
         if (cached !== curline || g.curline !== curline) {
            curlineByLeaf.set(g.key, curline);
            g.curline = curline;
            changed = true;
         }
      }
      if (g.curline_on !== curlineOn) {
         g.curline_on = curlineOn;
         changed = true;
      }
   }
   if (changed) pushModel();
}

el.addEventListener('pointerup', updateCaretStatus);
el.addEventListener('keyup', updateCaretStatus);
el.addEventListener('term_send', (e) => {
   const text = e.detail.text;
   const command = text.trim();
   const firstWord = command.split(/\s+/, 1)[0];
   let response;
   if (command === 'ls') response = FILES.map((entry) => entry.name).join('\n');
   else if (command === 'pwd') response = '/work/agentfs-cxx';
   else response = `zsh: command not found: ${firstWord}`;
   const current = String(el.getParam('panel.termlog') ?? '');
   el.setParam('panel.termlog', `${current}\ncan@mac slab-lang % ${text}\n${response}`);
   el.setFieldText(e.detail.meta.key, '');
});


const findByLeaf = new Map();

function findOwnerKey(metaKey) {
   const ed = metaKey.indexOf('/#ed');
   return lastItemSeg(metaKey.slice(0, ed + 1)) ?? focusedLeaf;
}

function collectMatches(content, query) {
   if (!content || !query) return [];
   const matches = [];
   const haystack = content.toLocaleLowerCase();
   const needle = query.toLocaleLowerCase();
   for (let at = 0; (at = haystack.indexOf(needle, at)) >= 0; at += needle.length) {
      matches.push(at);
   }
   return matches;
}

function setFindState(ownerKey, query) {
   const path = activePathOf(findLeaf(ownerKey));
   const content = CONTENTS[path] ?? '';
   const matches = collectMatches(content, query);
   const state = { query, path, content, matches, index: 0 };
   findByLeaf.set(ownerKey, state);
   return state;
}

el.addEventListener('find_change', (e) => {
   const ownerKey = findOwnerKey(e.detail.meta.key);
   const g = findLeaf(ownerKey);
   if (!g) return;
   const state = setFindState(ownerKey, e.detail.text);
   g.find_status = state.matches.length ? `1 of ${state.matches.length}` : 'No results';
   pushModel();
});

function advanceFind(ownerKey, query, backwards) {
   const g = findLeaf(ownerKey);
   if (!g) return;
   const path = activePathOf(g);
   const content = CONTENTS[path] ?? '';
   let state = findByLeaf.get(ownerKey);
   if (!state || state.query !== query || state.path !== path || state.content !== content) {
      state = setFindState(ownerKey, query);
   }
   const count = state.matches.length;
   if (!count) {
      g.find_status = 'No results';
      pushModel();
      return;
   }
   state.index = (state.index + (backwards ? count - 1 : 1)) % count;
   const matchOffset = state.matches[state.index];
   g.find_status = `${state.index + 1} of ${count}`;
   pushModel();
   let line = 1;
   for (let at = 0; at < matchOffset; at += 1) {
      if (content.charCodeAt(at) === 10) line += 1;
   }
   revealLine(ownerKey, path, line, {
      caret: matchOffset,
      anchor: matchOffset + query.length,
   });
}

// Plain Enter arrives via the field's submit= binding; the kernel
// deliberately bypasses submit for modified Enter, so Shift+Enter
// (find previous) is routed through the window keydown layer instead.
el.addEventListener('find_submit', (e) => {
   advanceFind(findOwnerKey(e.detail.meta.key), e.detail.text, false);
});

el.addEventListener('find_close', (e) => {
   const ed = e.detail.meta.key.indexOf('/#ed');
   const ownerKey = lastItemSeg(e.detail.meta.key.slice(0, ed + 1)) ?? focusedLeaf;
   const g = findLeaf(ownerKey);
   if (!g) return;
   g.show_find = false;
   pushModel();
});

const chatSessions = [];

el.addEventListener('chat_send', (e) => {
   const text = e.detail.text;
   if (!text.trim()) return;
   console.log(`chat_send: ${text}`);
   chatSessions.push({ key: String(chatSessions.length + 1), title: text });
   el.setList('chat.sessions', '', chatSessions);
   el.setFieldText(e.detail.meta.key, '');
   el.setParam('chat.typing', false);
});

el.addEventListener('chat_change', (e) => {
   el.setParam('chat.typing', e.detail.text.length > 0);
});

// ------------------------------------------------------------ chrome sashes ---
const CHROME_W = 55 + 4 + 4;
const CHROME_H = 28 + 4 + 22 + 3;
const panePx = { sidebar: 195, chat: 198, panel: 178 };
const PANEL_FLOOR = 77;
let panelRestorePx = panePx.panel;
let sidebarRestorePx = panePx.sidebar;
let panelPreMaxPx = panePx.panel;
let panelMaximized = false;

function sceneKey(suffix) {
   for (const n of el.sceneSnapshot()) if (n.key.endsWith(suffix)) return n.key;
   return null;
}

function maxPanelPx() {
   return Math.max(PANEL_FLOOR, el.clientHeight - CHROME_H - 200);
}

function closePanel() {
   if (panePx.panel > PANEL_FLOOR) panelRestorePx = panePx.panel;
   panePx.panel = 0;
   panelMaximized = false;
   applyLayout();
}

function restorePanel() {
   if (panePx.panel > PANEL_FLOOR) return;
   panePx.panel = Math.max(PANEL_FLOOR, panelRestorePx);
   panelMaximized = false;
   applyLayout();
}

function showPanel(key, itemKey) {
   selectPanel(key, itemKey);
   restorePanel();
}

function showProblemsPanel() {
   showPanel('problems');
}

function toggleMaxPanel() {
   if (panelMaximized) {
      panePx.panel = panelPreMaxPx;
      if (panelPreMaxPx > PANEL_FLOOR) panelRestorePx = panelPreMaxPx;
      panelMaximized = false;
   } else {
      panelPreMaxPx = panePx.panel;
      panePx.panel = maxPanelPx();
      panelMaximized = true;
   }
   applyLayout();
}

function restoreSidebar() {
   if (panePx.sidebar > 0) return;
   panePx.sidebar = Math.max(1, sidebarRestorePx);
   applyLayout();
}

function toggleSidebar() {
   if (panePx.sidebar > 0) {
      sidebarRestorePx = panePx.sidebar;
      panePx.sidebar = 0;
      applyLayout();
   } else {
      restoreSidebar();
   }
}

function applyLayout() {
   const w = el.clientWidth;
   const h = el.clientHeight;
   const sashL = sceneKey('#sashL');
   const sashR = sceneKey('#sashR');
   const sashP = sceneKey('#sashP');
   if (!sashL || !sashR || !sashP) return;
   if (panelMaximized) panePx.panel = maxPanelPx();
   el.setDivider(sashL, panePx.sidebar);
   el.setDivider(sashR, w - CHROME_W - panePx.sidebar - panePx.chat);
   el.setDivider(sashP, h - CHROME_H - panePx.panel);
}

el.addEventListener('sash_sidebar', (e) => {
   panePx.sidebar = Number.parseFloat(e.detail.text);
   if (panePx.sidebar > 0) sidebarRestorePx = panePx.sidebar;
});
el.addEventListener('sash_center', (e) => {
   panePx.chat = el.clientWidth - CHROME_W - panePx.sidebar - Number.parseFloat(e.detail.text);
});
el.addEventListener('sash_panel', (e) => {
   panePx.panel = el.clientHeight - CHROME_H - Number.parseFloat(e.detail.text);
   panelMaximized = false;
   if (panePx.panel > PANEL_FLOOR) panelRestorePx = panePx.panel;
});
el.addEventListener('panel_max', toggleMaxPanel);
el.addEventListener('panel_close', closePanel);
window.addEventListener('keydown', (e) => {
   const primary = e.metaKey || e.ctrlKey;
   const key = e.key.toLowerCase();
   let handled = primary;

   if (!primary && e.shiftKey && e.key === 'Enter') {
      const fk = el.focusedKey();
      if (fk && fk.includes('/#ed/') && !fk.includes('/#edscroll/')) {
         e.preventDefault();
         e.stopImmediatePropagation();
         advanceFind(findOwnerKey(fk), el.fieldText(fk) ?? '', true);
         return;
      }
   }
   if (primary && !e.shiftKey && key === 'w') {
      const activeTab = findLeaf(focusedLeaf)?.tabs.find((tab) => tab.active);
      if (activeTab) {
         closeTab(activeTab.key);
         pushModel();
      }
   } else if (primary && !e.shiftKey && key === 's') {
      const activeTab = findLeaf(focusedLeaf)?.tabs.find((tab) => tab.active);
      if (activeTab?.dirty) {
         activeTab.dirty = false;
         pushModel();
      }
   } else if (primary && !e.shiftKey && key === 'b') {
      toggleSidebar();
   } else if (primary && e.shiftKey && ['e', 'f', 'x'].includes(key)) {
      selectActivity({ e: 'explorer', f: 'search', x: 'ext' }[key]);
   } else if (primary && !e.shiftKey && key === 'f') {
      const leafKey = focusedLeaf;
      const g = findLeaf(leafKey);
      if (g) {
         g.show_find = true;
         pushModel();
         el.whenSettled().then(() => {
            const findField = el.sceneSnapshot().find((n) =>
               n.editable === true
               && n.key.includes(`~${leafKey}/`)
               && !n.key.includes('/#edscroll/'));
            if (!findField) return;
            el.focus();
            el.setFocus(findField.key, false);
            // Select the retained query so typing replaces it (VS Code).
            const existing = el.fieldText(findField.key) ?? '';
            if (existing.length) el.setCaret(findField.key, existing.length, 0);
         });
      }
   } else if (primary && !e.shiftKey && key === 'j') {
      if (panePx.panel <= PANEL_FLOOR) restorePanel();
      else closePanel();
   } else if (e.ctrlKey && e.code === 'Backquote') {
      selectPanel('terminal');
      if (panePx.panel <= PANEL_FLOOR) restorePanel();
      else closePanel();
   } else {
      handled = false;
   }

   if (handled) {
      e.preventDefault();
      e.stopImmediatePropagation();
   }
}, true);
new ResizeObserver(applyLayout).observe(el);

// Model pushes run synchronously from signal handlers: signals dispatch
// after inst_dispatch returns, and keyed re-solves preserve the kernel's
// pointer capture and armed drag (synthetic item ids are key-stable), so a
// mousedown switch repaints immediately — VS Code's switch-on-mousedown.

el.whenSettled().then(() => {
   reindex();
   applyLayout();
   pushTree();
   el.setList('problems.rows', '', PROBLEMS);
   el.setParam('status.errs', String(PROBLEMS.filter(({ icon }) => icon === 'errc').length));
   el.setParam('status.warns', String(PROBLEMS.filter(({ icon }) => icon === 'warn').length));
   el.whenSettled().then(() => {
      // initial chrome selection: Explorer store.hpp row + OUTPUT panel tab
      const encodedStorePath = encodeURIComponent(selectedPath);
      for (const n of el.sceneSnapshot()) {
         if (n.key.includes(`#treerows~${encodedStorePath}/row@0`)) {
            selectedTreeRow = n.key;
            el.setNodeState(n.key, 'selected', true);
         }
         if (n.key.endsWith('/output')) {
            selectedPanelTab = n.key;
            el.setNodeState(n.key, 'selected', true);
         }
      }
   });
});
pushModel();
window.__model = { get root() { return root; }, leaves: () => [...leaves()].map((g) => `${g.key}[${g.tabs.map((t) => t.key + (t.active ? '*' : '')).join('|')}]`) };
window.__panes = panePx;
