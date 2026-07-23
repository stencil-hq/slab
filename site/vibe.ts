// vibeviewer design mode — the direct-manipulation layer over the live
// slab element. Geometry and hit order come from the SAME painted scene
// (`sceneSnapshot`, `sceneHitContains`); every manipulation is a SOURCE edit
// routed through CodeMirror, so undo/redo and the text editor
// stay authoritative.
//
// Gestures: click select (hover preview), drag = move (`at`/`offset` on
// canvas/stack children, reorder/reparent in flow containers), 8-handle
// resize (`w`/`h`), arrow-key nudge/reorder, dblclick inline text edit,
// ⌫ delete, ⌘D duplicate, Escape ascends. Wheel scrolls kernel scroll
// containers so clipped children stay reachable while designing.

import type { SlabElement } from '@stencil-hq/wslab';
import {
   type Change,
   CONTAINERS,
   deleteNode,
   duplicateNode,
   insertChild,
   keyForNode,
   moveNode,
   nodeAtLine,
   nodeAtPos,
   resolveKey,
   type SrcDoc,
   type SrcNode,
   type SrcTarget,
   setArg,
   setAttr,
} from './slab-doc.ts';

/** Scene kind ids → surface names (slir K_* order). */
export const KIND_NAMES = [
   'row',
   'col',
   'wrap',
   'grid',
   'stack',
   'canvas',
   'para',
   'group',
   'text',
   'span',
   'rect',
   'img',
   'path',
   'spacer',
   'hole',
   'each',
   'divider',
   'icon',
] as const;

export type Mode = 'design' | 'interact';

export interface Rect {
   x: number;
   y: number;
   w: number;
   h: number;
}

/** What the canvas currently points at, resolved to source. */
export interface Selection {
   key: string;
   /** slir node id (scene identity). */
   node: number;
   /** Scene kind name (resolved tree). */
   kind: string;
   /** Source mapping; null when the node has no source anchor. */
   target: SrcTarget | null;
   rect: Rect;
}

export interface VibeHost {
   view: SlabElement;
   overlay: HTMLElement;
   /** Current parse of the editor buffer. */
   doc(): SrcDoc;
   /** True when the last check produced no errors (design gestures gate on it). */
   clean(): boolean;
   text(): string;
   apply(changes: Change[], history: boolean): void;
   /** Fold everything applied since `text0` into one undo entry. */
   commitDrag(text0: string): void;
   /** Scroll the editor to a node and park the cursor on it. */
   reveal(node: SrcNode): void;
   /** Throttled recompile for live gestures (bypasses the typing debounce). */
   compileSoon(): void;
   onSelect(sel: Selection | null): void;
}

interface DragState {
   pointerId: number;
   startX: number;
   startY: number;
   text0: string;
   key: string;
   grabDX: number;
   grabDY: number;
   rect0: Rect;
   engaged: boolean;
   gesture: 'at' | 'offset' | 'reorder' | 'resize';
   handle: string;
   base: { x: number; y: number };
   /** reorder drop target (recomputed per move). */
   drop: { parentKey: string; index: number; canvas: boolean; at: Rect } | null;
}

const HANDLES = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'] as const;

/** Unescape a slab string literal body (\n \t \" \\ \_ = nbsp). */
function unquote(raw: string): string {
   const body = raw.replace(/^"|"$/g, '');
   return body.replace(/\\([\s\S])/g, (_, c: string) => {
      if (c === 'n') return '\n';
      if (c === 't') return '\t';
      if (c === '_') return '\u00a0';
      return c;
   });
}

function quote(text: string): string {
   return `"${text.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n').replace(/\t/g, '\\t')}"`;
}
/** Canvas-side API main.ts and the inspector drive. */
export interface Vibe {
   mode(): Mode;
   setMode(m: Mode): void;
   /** Select a scene key (null clears); reveals the source line by default. */
   select(key: string | null, opts?: { reveal?: boolean }): void;
   selection(): Selection | null;
   selectedKey(): string | null;
   /** Re-resolve selection/hover against the latest frame and redraw. */
   refresh(): void;
   /** Editor cursor landed at `pos` — mirror it on the canvas. */
   selectAtPos(pos: number): void;
   /** ADD palette: insert a snippet relative to the selection. */
   insertSnippet(snippet: string): void;
   /** Arg edit passthrough (canvas + inspector share one commit path). */
   setNodeArg(node: SrcNode, i: number, text: string): void;
}

export function createVibe(host: VibeHost): Vibe {
   const { overlay, view } = host;
   let mode: Mode = 'design';
   let selKey: string | null = null;
   let hoverKey: string | null = null;
   let drag: DragState | null = null;
   let inlineFor: string | null = null;

   // ── overlay chrome ────────────────────────────────────────────────
   const el = (cls: string, parent: HTMLElement = overlay): HTMLDivElement => {
      const d = document.createElement('div');
      d.className = cls;
      parent.appendChild(d);
      return d;
   };
   const hoverBox = el('vv-box vv-hover');
   const hoverChip = el('vv-chip vv-chip-hover');
   const targetBox = el('vv-box vv-target');
   const ghostBox = el('vv-box vv-ghost');
   const marker = el('vv-marker');
   const selBox = el('vv-box vv-sel');
   const sizeChip = el('vv-chip vv-chip-size');
   const note = el('vv-note');
   note.textContent = 'SOURCE ERRORS — DESIGN PAUSED';
   const inline = document.createElement('input');
   inline.className = 'vv-inline';
   inline.spellcheck = false;
   overlay.appendChild(inline);
   for (const h of HANDLES) {
      const hd = el(`vv-h vv-h-${h}`, selBox);
      hd.dataset.h = h;
   }
   hide(hoverBox, hoverChip, targetBox, ghostBox, marker, selBox, sizeChip, note, inline);

   function hide(...els: HTMLElement[]): void {
      for (const e of els) e.style.display = 'none';
   }

   function place(e: HTMLElement, r: Rect): void {
      e.style.display = 'block';
      e.style.transform = `translate(${r.x}px, ${r.y}px)`;
      e.style.width = `${Math.max(0, r.w)}px`;
      e.style.height = `${Math.max(0, r.h)}px`;
   }

   function placeChip(e: HTMLElement, x: number, y: number, text: string): void {
      e.textContent = text;
      e.style.display = 'block';
      e.style.transform = `translate(${Math.max(0, x)}px, ${Math.max(0, y)}px)`;
   }

   // ── kernel geometry ───────────────────────────────────────────────

   function sceneIx(key: string): number {
      return view.sceneSnapshot().findIndex((node) => node.key === key);
   }

   function rectAt(ix: number, nodes = view.sceneSnapshot()): Rect {
      const node = nodes[ix];
      if (!node || ix < 0) return { x: 0, y: 0, w: 0, h: 0 };
      return { x: node.x, y: node.y, w: node.w, h: node.h };
   }

   /** Topmost scene node at (x, y) — ignores `inert` (design tools grab
    * everything) but honors clips and rotation via the kernel's `contains`.
    * Among overlapping candidates, grossly larger boxes yield to the
    * smallest one under the cursor: a canvas `path` whose bbox spans the
    * document (ink ≠ box) must not swallow the label painted beneath it. */
   function hitChain(x: number, y: number, excludeKey?: string): number[] {
      const nodes = view.sceneSnapshot();
      const hits: number[] = [];
      let minArea = Number.POSITIVE_INFINITY;
      for (let k = nodes.length - 1; k >= 0; k--) {
         const node = nodes[k];
         if (!node) continue;
         if (
            excludeKey !== undefined &&
            (node.key === excludeKey || node.key.startsWith(`${excludeKey}/`))
         ) {
            continue;
         }
         if (view.sceneHitContains(k, x, y)) {
            hits.push(k);
            const area = node.w * node.h;
            if (area < minArea) minArea = area;
         }
      }
      const pick =
         hits.find((k) => {
            const node = nodes[k];
            return node !== undefined && node.w * node.h <= minArea * 6;
         }) ?? hits[0];
      return pick === undefined ? [] : view.sceneChain(pick);
   }

   /** Deepest hit whose key resolves to source; falls back to node_line. */
   function resolveHit(chain: number[]): Selection | null {
      const nodes = view.sceneSnapshot();
      const doc = host.doc();
      for (let i = chain.length - 1; i >= 0; i--) {
         const sceneNode = nodes[chain[i]];
         if (!sceneNode || sceneNode.key === '') continue;
         let target = resolveKey(doc, sceneNode.key);
         if (!target && sceneNode.src_line > 0) {
            target = nodeAtLine(doc, sceneNode.src_line);
         }
         if (target) {
            return {
               key: sceneNode.key,
               node: sceneNode.node,
               kind: KIND_NAMES[sceneNode.kind] ?? '?',
               target,
               rect: {
                  x: sceneNode.x,
                  y: sceneNode.y,
                  w: sceneNode.w,
                  h: sceneNode.h,
               },
            };
         }
      }
      return null;
   }

   function selectionFor(key: string | null): Selection | null {
      if (!key) return null;
      const nodes = view.sceneSnapshot();
      const ix = nodes.findIndex((node) => node.key === key);
      const sceneNode = nodes[ix];
      if (!sceneNode) return null;
      const doc = host.doc();
      let target = resolveKey(doc, key);
      if (!target && sceneNode.src_line > 0) {
         target = nodeAtLine(doc, sceneNode.src_line);
      }
      return {
         key,
         node: sceneNode.node,
         kind: KIND_NAMES[sceneNode.kind] ?? '?',
         target,
         rect: {
            x: sceneNode.x,
            y: sceneNode.y,
            w: sceneNode.w,
            h: sceneNode.h,
         },
      };
   }
   /** Resolve an edit target through retained scene provenance before key-only lookup. */
   function sourceTargetFor(key: string): SrcTarget | null {
      return selectionFor(key)?.target ?? resolveKey(host.doc(), key);
   }

   // ── rendering ─────────────────────────────────────────────────────

   /** Last `key|resolved` signature pushed to host.onSelect — refresh()
    * re-notifies when a compile makes a pending selection resolvable
    * (e.g. a just-inserted node appearing in the fresh scene). */
   let notified = '';

   function notify(sel: Selection | null): void {
      notified = `${selKey ?? ''}|${sel ? 1 : 0}`;
      host.onSelect(sel);
   }

   function refresh(): void {
      overlay.classList.toggle('stale', !host.clean());
      note.style.display = mode === 'design' && !host.clean() ? 'block' : 'none';
      if (mode !== 'design') {
         hide(hoverBox, hoverChip, selBox, sizeChip, marker, ghostBox, targetBox);
         return;
      }
      // selection
      const ix = selKey ? sceneIx(selKey) : -1;
      if (selKey && ix >= 0) {
         const r = rectAt(ix);
         place(selBox, r);
         if (host.clean()) {
            placeChip(
               sizeChip,
               r.x + r.w / 2 - 30,
               r.y + r.h + 6,
               `${Math.round(r.w)} × ${Math.round(r.h)}`,
            );
         } else {
            hide(sizeChip);
         }
      } else {
         hide(selBox, sizeChip);
      }
      if (`${selKey ?? ''}|${ix >= 0 ? 1 : 0}` !== notified && !drag) {
         notify(selKey && ix >= 0 ? selectionFor(selKey) : null);
      }
      // hover
      const hix = hoverKey && hoverKey !== selKey ? sceneIx(hoverKey) : -1;
      if (hoverKey && hix >= 0 && !drag) {
         const r = rectAt(hix);
         place(hoverBox, r);
         const seg = hoverKey.split('/').pop() ?? hoverKey;
         placeChip(hoverChip, r.x, Math.max(0, r.y - 18), seg.toUpperCase());
      } else {
         hide(hoverBox, hoverChip);
      }
   }

   function select(key: string | null, opts: { reveal?: boolean } = {}): void {
      selKey = key;
      const sel = selectionFor(key);
      if (sel?.target && opts.reveal !== false) host.reveal(sel.target.node);
      notify(sel);
      refresh();
   }

   // ── gestures ──────────────────────────────────────────────────────

   function xy(e: PointerEvent | MouseEvent): { x: number; y: number } {
      const bounds = overlay.getBoundingClientRect();
      const scaleX = bounds.width > 0 ? overlay.offsetWidth / bounds.width : 1;
      const scaleY = bounds.height > 0 ? overlay.offsetHeight / bounds.height : 1;
      return {
         x: (e.clientX - bounds.left) * scaleX,
         y: (e.clientY - bounds.top) * scaleY,
      };
   }

   /** Current attr value as an `x,y` pair (missing/partial → zeros). */
   function pairOf(node: SrcNode, name: string): { x: number; y: number } {
      const a = node.attrs.find((v) => v.name === name);
      if (!a) return { x: 0, y: 0 };
      const parts = a.value.split(',').map((s) => Number.parseFloat(s.trim()));
      return { x: parts[0] || 0, y: parts[1] || 0 };
   }

   function gestureFor(sel: Selection): 'at' | 'offset' | 'reorder' | null {
      const src = sel.target?.node;
      if (!src) return null;
      const pk = src.parent?.kind;
      if (pk === 'canvas') return 'at';
      if (pk === 'stack') return 'offset';
      if (src.parent || sel.target?.def === null) return 'reorder';
      return null;
   }

   overlay.addEventListener('pointerdown', (e) => {
      if (mode !== 'design' || e.button !== 0 || !host.clean()) return;
      commitInline();
      const { x, y } = xy(e);
      const handle = (e.target as HTMLElement).dataset?.h;
      if (handle && selKey) {
         const sel = selectionFor(selKey);
         if (sel?.target) {
            drag = {
               pointerId: e.pointerId,
               startX: x,
               startY: y,
               text0: host.text(),
               key: selKey,
               grabDX: 0,
               grabDY: 0,
               rect0: sel.rect,
               engaged: true,
               gesture: 'resize',
               handle,
               base: pairOf(sel.target.node, 'at'),
               drop: null,
            };
            overlay.setPointerCapture(e.pointerId);
            return;
         }
      }
      const chain = hitChain(x, y);
      let sel = resolveHit(chain);
      if (!sel) {
         select(null);
         return;
      }
      // pressing inside the CURRENT selection drags it (not its deepest
      // descendant) — crumb up once, then move the container as a unit
      if (selKey && sel.key !== selKey) {
         const nodes = view.sceneSnapshot();
         const chainKeys = chain.map((ix) => nodes[ix]?.key ?? '');
         if (chainKeys.includes(selKey)) {
            const kept = selectionFor(selKey);
            if (kept) sel = kept;
         }
      }
      if (sel.key !== selKey) select(sel.key);
      const gesture = gestureFor(sel);
      if (!gesture || !sel.target) return;
      drag = {
         pointerId: e.pointerId,
         startX: x,
         startY: y,
         text0: host.text(),
         key: sel.key,
         grabDX: x - sel.rect.x,
         grabDY: y - sel.rect.y,
         rect0: sel.rect,
         engaged: false,
         gesture,
         handle: '',
         base: pairOf(sel.target.node, gesture === 'offset' ? 'offset' : 'at'),
         drop: null,
      };
      overlay.setPointerCapture(e.pointerId);
   });

   overlay.addEventListener('pointermove', (e) => {
      if (mode !== 'design') return;
      const { x, y } = xy(e);
      if (!drag) {
         if (!host.clean()) return;
         const sel = resolveHit(hitChain(x, y));
         const key = sel?.key ?? null;
         if (key !== hoverKey) {
            hoverKey = key;
            refresh();
         }
         return;
      }
      const dx = x - drag.startX;
      const dy = y - drag.startY;
      if (!drag.engaged && Math.hypot(dx, dy) < 3) return;
      drag.engaged = true;
      hoverKey = null;
      hide(hoverBox, hoverChip);
      if (drag.gesture === 'resize') {
         liveResize(dx, dy, e.shiftKey);
      } else if (drag.gesture === 'reorder') {
         liveReorder(x, y);
      } else {
         livePosition(dx, dy, e.shiftKey);
      }
   });

   const endDrag = (e: PointerEvent): void => {
      if (!drag) return;
      const d = drag;
      drag = null;
      hide(ghostBox, marker, targetBox);
      overlay.releasePointerCapture?.(d.pointerId);
      if (!d.engaged) return;
      if (d.gesture === 'reorder') {
         dropReorder(d, xy(e));
      } else {
         host.commitDrag(d.text0);
      }
      refresh();
   };
   overlay.addEventListener('pointerup', endDrag);
   overlay.addEventListener('pointercancel', endDrag);

   /** Re-resolve the dragged node against the CURRENT parse. */
   function dragTarget(d: DragState): SrcNode | null {
      return sourceTargetFor(d.key)?.node ?? null;
   }

   function livePosition(dx: number, dy: number, snap: boolean): void {
      const d = drag;
      if (!d) return;
      const node = dragTarget(d);
      if (!node) return;
      const step = snap ? 8 : 1;
      const nx = Math.round((d.base.x + dx) / step) * step;
      const ny = Math.round((d.base.y + dy) / step) * step;
      const name = d.gesture === 'offset' ? 'offset' : 'at';
      host.apply(setAttr(node, name, `${nx},${ny}`), false);
      host.compileSoon();
   }

   function liveResize(dx: number, dy: number, snap: boolean): void {
      const d = drag;
      if (!d) return;
      const node = dragTarget(d);
      if (!node) return;
      const h = d.handle;
      const sx = h.includes('e') ? 1 : h.includes('w') ? -1 : 0;
      const sy = h.includes('s') ? 1 : h.includes('n') ? -1 : 0;
      const step = snap ? 8 : 1;
      const w = Math.max(1, Math.round((d.rect0.w + sx * dx) / step) * step);
      const hh = Math.max(1, Math.round((d.rect0.h + sy * dy) / step) * step);
      const changes: Change[] = [];
      if (sx !== 0) changes.push(...setAttr(node, 'w', String(w)));
      if (sy !== 0) changes.push(...setAttr(node, 'h', String(hh)));
      // canvas children: west/north handles pull `at` along (top-left anchor)
      if (node.parent?.kind === 'canvas' && !node.attrs.some((a) => a.name === 'anchor')) {
         const ax = d.base.x + (h.includes('w') ? d.rect0.w - w : 0);
         const ay = d.base.y + (h.includes('n') ? d.rect0.h - hh : 0);
         if (h.includes('w') || h.includes('n')) {
            changes.push(...setAttr(node, 'at', `${Math.round(ax)},${Math.round(ay)}`));
         }
      }
      if (changes.length > 0) {
         host.apply(dedupe(changes), false);
         host.compileSoon();
         placeChip(
            sizeChip,
            d.rect0.x + d.rect0.w / 2 - 30,
            d.rect0.y + d.rect0.h + 6,
            `${sx !== 0 ? w : Math.round(d.rect0.w)} × ${sy !== 0 ? hh : Math.round(d.rect0.h)}`,
         );
      }
   }

   /** setAttr against one node can emit two inserts at the SAME header
    * offset (w then h on a bare node); merge them so ranges stay unique. */
   function dedupe(changes: Change[]): Change[] {
      const out: Change[] = [];
      for (const c of changes) {
         const prev = out.find((p) => p.from === c.from && p.to === c.to && p.to === p.from);
         if (prev) prev.insert += c.insert;
         else out.push(c);
      }
      return out;
   }

   function liveReorder(x: number, y: number): void {
      const d = drag;
      if (!d) return;
      const nodes = view.sceneSnapshot();
      // ghost follows the pointer
      place(ghostBox, { x: x - d.grabDX, y: y - d.grabDY, w: d.rect0.w, h: d.rect0.h });
      // deepest container under the pointer, excluding the dragged subtree
      const chain = hitChain(x, y, d.key);
      const doc = host.doc();
      d.drop = null;
      hide(marker, targetBox);
      for (let i = chain.length - 1; i >= 0; i--) {
         const ix = chain[i];
         const sceneNode = nodes[ix];
         if (!sceneNode) continue;
         const kindName = KIND_NAMES[sceneNode.kind] ?? '';
         if (CONTAINERS[kindName] !== true) continue;
         const key = sceneNode.key;
         const target = key === '' ? null : sourceTargetFor(key);
         if (key !== '' && (!target || target.def !== null || target.node.isCall)) continue;
         const parentRect = rectAt(ix, nodes);
         if (kindName === 'canvas') {
            d.drop = { parentKey: key, index: -1, canvas: true, at: parentRect };
            place(targetBox, parentRect);
            return;
         }
         // insertion index among scene children, by main-axis midpoints
         const isRow = kindName === 'row' || kindName === 'wrap' || sceneNode.is_row;
         let index = 0;
         let markerPos: Rect | null = null;
         let lastRect: Rect | null = null;
         for (let c = 0; c < nodes.length; c++) {
            const child = nodes[c];
            if (!child || child.parent !== ix) continue;
            const ckey = child.key;
            if (ckey === d.key || ckey.startsWith(`${d.key}/`)) continue;
            const cr = rectAt(c, nodes);
            lastRect = cr;
            const mid = isRow ? cr.x + cr.w / 2 : cr.y + cr.h / 2;
            const p = isRow ? x : y;
            if (p < mid && markerPos === null) {
               markerPos = isRow
                  ? { x: cr.x - 2, y: cr.y, w: 2, h: cr.h }
                  : { x: cr.x, y: cr.y - 2, w: cr.w, h: 2 };
               // index = source position of this child
               const csrc = ckey === '' ? null : sourceTargetFor(ckey);
               if (csrc?.node.parent) {
                  index = csrc.node.parent.children.indexOf(csrc.node);
               } else if (csrc) {
                  index = doc.roots.indexOf(csrc.node);
               }
            }
         }
         if (markerPos === null) {
            index = Number.MAX_SAFE_INTEGER; // append
            markerPos = lastRect
               ? isRow
                  ? { x: lastRect.x + lastRect.w, y: lastRect.y, w: 2, h: lastRect.h }
                  : { x: lastRect.x, y: lastRect.y + lastRect.h, w: lastRect.w, h: 2 }
               : isRow
                 ? {
                      x: parentRect.x + 4,
                      y: parentRect.y + 4,
                      w: 2,
                      h: Math.max(8, parentRect.h - 8),
                   }
                 : {
                      x: parentRect.x + 4,
                      y: parentRect.y + 4,
                      w: Math.max(8, parentRect.w - 8),
                      h: 2,
                   };
         }
         d.drop = { parentKey: key, index, canvas: false, at: parentRect };
         place(targetBox, parentRect);
         place(marker, markerPos);
         return;
      }
   }

   function dropReorder(d: DragState, p: { x: number; y: number }): void {
      if (!d.drop) return;
      const doc = host.doc();
      const node = sourceTargetFor(d.key)?.node;
      if (!node) return;
      const parent =
         d.drop.parentKey === '' ? null : (sourceTargetFor(d.drop.parentKey)?.node ?? null);
      const index =
         d.drop.index === -1 || d.drop.index === Number.MAX_SAFE_INTEGER
            ? parent
               ? parent.children.length
               : doc.roots.length
            : d.drop.index;
      const edit = moveNode(doc, node, parent, index);
      if (edit.changes.length === 0) return;
      host.apply(edit.changes, false);
      if (d.drop.canvas) {
         // position where the ghost landed, relative to the canvas box
         const after = host.doc();
         const moved = nodeAtPos(after, edit.caret);
         if (moved) {
            const ax = Math.round(p.x - d.grabDX - d.drop.at.x);
            const ay = Math.round(p.y - d.grabDY - d.drop.at.y);
            host.apply(setAttr(moved, 'at', `${ax},${ay}`), false);
         }
      }
      host.commitDrag(d.text0);
      const after = host.doc();
      const moved = nodeAtPos(after, edit.caret);
      if (moved) select(keyForNode(moved), { reveal: true });
      host.compileSoon();
   }

   // ── wheel: kernel scroll containers stay reachable ────────────────

   overlay.addEventListener(
      'wheel',
      (e) => {
         if (mode !== 'design') return;
         const nodes = view.sceneSnapshot();
         const { x, y } = xy(e);
         const chain = hitChain(x, y);
         const bounds = overlay.getBoundingClientRect();
         const scale = bounds.height > 0 ? bounds.height / overlay.offsetHeight : 1;
         const dx = e.deltaX / scale;
         const dy = e.deltaY / scale;
         const mainDelta = e.shiftKey ? dx : dy;
         const crossDelta = e.shiftKey ? dy : dx;
         let handled = false;
         for (const [axis, delta, flag] of [
            [0, mainDelta, 4],
            [1, crossDelta, 512],
         ] as const) {
            if (delta === 0) continue;
            for (let i = chain.length - 1; i >= 0; i--) {
               const node = nodes[chain[i]];
               if (!node || (node.flags & flag) === 0 || node.key === '') continue;
               handled =
                  view.setScroll(node.key, axis, view.getScroll(node.key, axis) + delta) || handled;
               break;
            }
         }
         if (handled) e.preventDefault();
      },
      { passive: false },
   );

   // ── inline text editing ───────────────────────────────────────────

   overlay.addEventListener('dblclick', (e) => {
      if (mode !== 'design' || !host.clean()) return;
      const { x, y } = xy(e);
      const sel = resolveHit(hitChain(x, y));
      const src = sel?.target?.node;
      if (!sel || !src) return;
      const argIx = src.args.findIndex((a) => a.kind === 'string');
      if (argIx === -1) return;
      select(sel.key);
      inlineFor = sel.key;
      inline.value = unquote(src.args[argIx].text);
      inline.dataset.arg = String(argIx);
      inline.style.display = 'block';
      inline.style.transform = `translate(${sel.rect.x}px, ${Math.max(0, sel.rect.y)}px)`;
      inline.style.width = `${Math.max(120, sel.rect.w)}px`;
      inline.style.height = `${Math.max(22, Math.min(sel.rect.h, 40))}px`;
      inline.focus();
      inline.select();
   });

   function commitInline(): void {
      if (!inlineFor || inline.style.display === 'none') {
         hide(inline);
         inlineFor = null;
         return;
      }
      const target = sourceTargetFor(inlineFor);
      const argIx = Number(inline.dataset.arg ?? '0');
      if (target) {
         const arg = target.node.args[argIx];
         if (arg && quote(inline.value) !== arg.text) {
            host.apply(
               [{ from: arg.span.from, to: arg.span.to, insert: quote(inline.value) }],
               true,
            );
            host.compileSoon();
         }
      }
      hide(inline);
      inlineFor = null;
   }

   inline.addEventListener('keydown', (e) => {
      e.stopPropagation();
      if (e.key === 'Enter') commitInline();
      if (e.key === 'Escape') {
         hide(inline);
         inlineFor = null;
      }
   });
   inline.addEventListener('blur', commitInline);

   // ── keyboard ──────────────────────────────────────────────────────

   function editable(t: EventTarget | null): boolean {
      const n = t as HTMLElement | null;
      if (!n) return false;
      return (
         n.tagName === 'INPUT' ||
         n.tagName === 'TEXTAREA' ||
         n.tagName === 'SELECT' ||
         n.isContentEditable ||
         n.closest?.('.cm-editor') !== null
      );
   }

   document.addEventListener('keydown', (e) => {
      if (mode !== 'design' || editable(e.target) || !selKey) return;
      const doc = host.doc();
      const target = sourceTargetFor(selKey);
      const node = target?.node;
      if (e.key === 'Escape') {
         const up = selKey.includes('/') ? selKey.slice(0, selKey.lastIndexOf('/')) : null;
         select(up);
         e.preventDefault();
         return;
      }
      if (!node) return;
      if (e.key === 'Backspace' || e.key === 'Delete') {
         const up = selKey.includes('/') ? selKey.slice(0, selKey.lastIndexOf('/')) : null;
         host.apply(deleteNode(doc, node), true);
         select(up, { reveal: false });
         host.compileSoon();
         e.preventDefault();
         return;
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'd') {
         const edit = duplicateNode(doc, node);
         host.apply(edit.changes, true);
         const copy = nodeAtPos(host.doc(), edit.caret);
         if (copy) select(keyForNode(copy), { reveal: true });
         host.compileSoon();
         e.preventDefault();
         return;
      }
      if (e.key.startsWith('Arrow')) {
         const step = e.shiftKey ? 10 : 1;
         const dx = e.key === 'ArrowLeft' ? -step : e.key === 'ArrowRight' ? step : 0;
         const dy = e.key === 'ArrowUp' ? -step : e.key === 'ArrowDown' ? step : 0;
         const pk = node.parent?.kind;
         if (pk === 'canvas' || pk === 'stack') {
            const name = pk === 'stack' ? 'offset' : 'at';
            const base = pairOf(node, name);
            host.apply(setAttr(node, name, `${base.x + dx},${base.y + dy}`), true);
            host.compileSoon();
            e.preventDefault();
            return;
         }
         // flow: arrows reorder among siblings
         const sibs = node.parent ? node.parent.children : doc.roots;
         const ix = sibs.indexOf(node);
         const dir = dx + dy;
         if (dir !== 0 && ix >= 0) {
            const to = dir < 0 ? ix - 1 : ix + 2; // moveNode index is a gap position
            const edit = moveNode(doc, node, node.parent, to);
            if (edit.changes.length > 0) {
               host.apply(edit.changes, true);
               const moved = nodeAtPos(host.doc(), edit.caret);
               if (moved) select(keyForNode(moved), { reveal: false });
               host.compileSoon();
            }
            e.preventDefault();
         }
      }
   });

   // ── public api ────────────────────────────────────────────────────

   return {
      mode: () => mode,
      setMode(m: Mode): void {
         mode = m;
         overlay.classList.toggle('off', m !== 'design');
         hoverKey = null;
         if (m !== 'design') commitInline();
         refresh();
      },
      select,
      selection: () => selectionFor(selKey),
      selectedKey: () => selKey,
      refresh,
      /** Editor cursor landed at `pos` — mirror it on the canvas. */
      selectAtPos(pos: number): void {
         const node = nodeAtPos(host.doc(), pos);
         if (!node) return;
         const key = keyForNode(node);
         if (key !== selKey) {
            selKey = key;
            host.onSelect(selectionFor(key));
            refresh();
         }
      },
      /** ADD palette: insert a snippet relative to the selection. */
      insertSnippet(snippet: string): void {
         const doc = host.doc();
         let parent: SrcNode | null = null;
         let index = 0;
         const node = selKey ? sourceTargetFor(selKey)?.node : null;
         const divider = /^\s*divider(?:\s|$)/.test(snippet);
         if (divider) {
            if (node && (node.kind === 'row' || node.kind === 'col') && node.children.length >= 2) {
               parent = node;
               index = Math.max(1, Math.floor(node.children.length / 2));
            } else if (
               node?.parent &&
               (node.parent.kind === 'row' || node.parent.kind === 'col') &&
               node.parent.children.length >= 2
            ) {
               const childIndex = node.parent.children.indexOf(node);
               if (childIndex >= 0) {
                  parent = node.parent;
                  index = childIndex === 0 ? 1 : childIndex;
               }
            } else if (
               !node &&
               doc.roots.length === 1 &&
               (doc.roots[0].kind === 'row' || doc.roots[0].kind === 'col') &&
               doc.roots[0].children.length >= 2
            ) {
               parent = doc.roots[0];
               index = Math.max(1, Math.floor(parent.children.length / 2));
            }
            if (!parent) return;
         } else if (node && CONTAINERS[node.kind] === true && !node.isCall) {
            parent = node;
            index = node.children.length;
         } else if (node?.parent) {
            parent = node.parent;
            index = parent.children.indexOf(node) + 1;
         } else if (doc.roots.length === 1 && CONTAINERS[doc.roots[0].kind] === true) {
            parent = doc.roots[0];
            index = parent.children.length;
         }
         let body = snippet;
         if (parent?.kind === 'canvas' && !/(^|\s)at=/.test(snippet)) {
            body = snippet.replace(/^(\S+)/, '$1 at=16,16');
         }
         const edit = insertChild(doc, parent, body, index);
         host.apply(edit.changes, true);
         const added = nodeAtPos(host.doc(), edit.caret);
         if (added) select(keyForNode(added), { reveal: true });
         host.compileSoon();
      },
      /** Arg edit passthrough for the inspector (kept here so canvas +
       * inspector share one commit path). */
      setNodeArg(node: SrcNode, i: number, text: string): void {
         host.apply(setArg(node, i, text), true);
         host.compileSoon();
      },
   };
}
