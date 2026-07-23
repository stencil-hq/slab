// vibeviewer inspector — schema-driven attribute editing for the selected
// node. Fields resolve their node FRESH by key at commit time (spans shift
// under every edit), so the panel never writes through stale spans. Values
// route through slab-doc edit builders into CodeMirror; params (host inputs,
// not source) route to the live element.

import type { SlabElement } from '@stencil-hq/wslab';
import {
   type Change,
   CONTAINERS,
   deleteNode,
   duplicateNode,
   keyForNode,
   nodeAtLine,
   nodeAtPos,
   parseListDefault,
   removeAttr,
   resolveKey,
   type SrcDoc,
   type SrcListRowSchema,
   type SrcNode,
   type SrcParam,
   setAttr,
   setFlag,
   setId,
} from './slab-doc.ts';
import type { Selection, Vibe } from './vibe.ts';

export interface InspectorHost {
   body: HTMLElement;
   meta: HTMLElement;
   crumbs: HTMLElement;
   view: SlabElement;
   vibe: Vibe;
   doc(): SrcDoc;
   text(): string;
   apply(changes: Change[], history: boolean): void;
   commitDrag(text0: string): void;
   compileSoon(): void;
   /** Scroll the editor to a text offset (GOTO affordances). */
   revealPos(pos: number): void;
}

type FieldKind = 'size' | 'num' | 'color' | 'enum' | 'text';

interface Field {
   name: string;
   kind: FieldKind;
   section: string;
   options?: string[];
   /** Shown even when unset (schema core for the node kind). */
   core?: (n: SrcNode) => boolean;
   /** Available at all for the node kind (ADD ATTR + set values). */
   applies?: (n: SrcNode) => boolean;
}

const TEXTY: Record<string, true> = { text: true, span: true, para: true };
const BOXY: Record<string, true> = {
   ...CONTAINERS,
   rect: true,
   spacer: true,
   img: true,
   hole: true,
};

const NINE = [
   'top-start',
   'top',
   'top-end',
   'start',
   'center',
   'end',
   'bottom-start',
   'bottom',
   'bottom-end',
];

const isBox = (n: SrcNode) => BOXY[n.kind] === true || n.isCall;
const isText = (n: SrcNode) => TEXTY[n.kind] === true;
const inCanvas = (n: SrcNode) => n.parent?.kind === 'canvas';
const inStack = (n: SrcNode) => n.parent?.kind === 'stack';
const always = () => true;

type ScrollMode = 'off' | 'main' | 'cross' | 'both';

function nodeScrollMode(node: SrcNode): ScrollMode {
   const value = node.attrs.find((attr) => attr.name === 'scroll')?.value;
   if (value === 'cross' || value === 'both') return value;
   return node.flags.some((flag) => flag.name === 'scroll') ? 'main' : 'off';
}

function hasMainScroll(node: SrcNode): boolean {
   const mode = nodeScrollMode(node);
   return mode === 'main' || mode === 'both';
}

const supportsScroll = (node: SrcNode) => CONTAINERS[node.kind] === true && !node.isCall;
const supportsVirtual = (node: SrcNode) =>
   node.kind === 'each' && node.parent !== null && hasMainScroll(node.parent);
const supportsSticky = (node: SrcNode) => node.parent !== null && hasMainScroll(node.parent);
const supportsDragGhost = (node: SrcNode) => node.attrs.some((attr) => attr.name === 'drag');

const FIELDS: Field[] = [
   { name: 'at', kind: 'text', section: 'LAYOUT', core: inCanvas, applies: inCanvas },
   { name: 'anchor', kind: 'enum', section: 'LAYOUT', options: NINE, applies: inCanvas },
   { name: 'offset', kind: 'text', section: 'LAYOUT', core: inStack, applies: inStack },
   {
      name: 'self',
      kind: 'enum',
      section: 'LAYOUT',
      options: ['start', 'center', 'end', 'baseline', ...NINE],
      core: inStack,
   },
   { name: 'w', kind: 'size', section: 'LAYOUT', core: always },
   { name: 'h', kind: 'size', section: 'LAYOUT', core: always },
   { name: 'min-w', kind: 'num', section: 'LAYOUT' },
   { name: 'max-w', kind: 'num', section: 'LAYOUT' },
   { name: 'min-h', kind: 'num', section: 'LAYOUT' },
   { name: 'max-h', kind: 'num', section: 'LAYOUT' },
   {
      name: 'pad',
      kind: 'text',
      section: 'LAYOUT',
      core: (n) => CONTAINERS[n.kind] === true || n.kind === 'rect',
      applies: isBox,
   },
   {
      name: 'gap',
      kind: 'text',
      section: 'LAYOUT',
      core: (n) => CONTAINERS[n.kind] === true && n.kind !== 'canvas' && n.kind !== 'stack',
      applies: (n) => CONTAINERS[n.kind] === true,
   },
   {
      // 2-axis placement for flow containers lives in the ALIGN matrix
      // (writes pack= main + align= cross); stack keeps the 9-position
      // enum — its align is the default child slot (§6.6).
      name: 'align',
      kind: 'enum',
      section: 'LAYOUT',
      options: NINE,
      core: (n) => n.kind === 'stack',
      applies: (n) => n.kind === 'stack' || n.kind === 'grid' || n.kind === 'para',
   },
   {
      name: 'cols',
      kind: 'text',
      section: 'LAYOUT',
      core: (n) => n.kind === 'grid',
      applies: (n) => n.kind === 'grid',
   },
   { name: 'span', kind: 'num', section: 'LAYOUT', applies: (n) => n.parent?.kind === 'grid' },
   {
      name: 'item-extent',
      kind: 'num',
      section: 'LAYOUT',
      core: (n) => n.flags.some((flag) => flag.name === 'virtual'),
      applies: supportsVirtual,
   },
   {
      name: 'overscan',
      kind: 'num',
      section: 'LAYOUT',
      core: (n) => n.flags.some((flag) => flag.name === 'virtual'),
      applies: supportsVirtual,
   },
   { name: 'attach', kind: 'text', section: 'LAYOUT', applies: (n) => inStack(n) || inCanvas(n) },
   {
      name: 'gravity',
      kind: 'enum',
      section: 'LAYOUT',
      options: [
         'below-start',
         'below-center',
         'below-end',
         'above-start',
         'above-center',
         'above-end',
         'left-start',
         'left-center',
         'left-end',
         'right-start',
         'right-center',
         'right-end',
      ],
      applies: (n) => inStack(n) || inCanvas(n),
   },
   {
      name: 'collide',
      kind: 'enum',
      section: 'LAYOUT',
      options: ['auto', 'none'],
      applies: (n) => inStack(n) || inCanvas(n),
   },
   {
      name: 'd',
      kind: 'text',
      section: 'STYLE',
      core: (n) => n.kind === 'path',
      applies: (n) => n.kind === 'path',
   },
   {
      name: 'src',
      kind: 'text',
      section: 'STYLE',
      core: (n) => n.kind === 'img',
      applies: (n) => n.kind === 'img',
   },
   { name: 'bg', kind: 'color', section: 'STYLE', core: (n) => n.kind === 'rect', applies: isBox },
   { name: 'stroke', kind: 'color', section: 'STYLE', applies: isBox },
   { name: 'stroke-w', kind: 'num', section: 'STYLE', applies: isBox },
   {
      name: 'stroke-align',
      kind: 'enum',
      section: 'STYLE',
      options: ['inside', 'center', 'outside'],
      applies: isBox,
   },
   { name: 'stroke-dash', kind: 'text', section: 'STYLE', applies: isBox },
   { name: 'stroke-sides', kind: 'text', section: 'STYLE', applies: isBox },
   {
      name: 'radius',
      kind: 'num',
      section: 'STYLE',
      core: (n) => n.kind === 'rect' || n.kind === 'img',
      applies: isBox,
   },
   { name: 'smooth', kind: 'num', section: 'STYLE', applies: isBox },
   { name: 'opacity', kind: 'num', section: 'STYLE' },
   { name: 'shadow', kind: 'text', section: 'STYLE', applies: isBox },
   { name: 'blur', kind: 'num', section: 'STYLE' },
   { name: 'backdrop', kind: 'text', section: 'STYLE', applies: isBox },
   { name: 'backdrop-mask', kind: 'text', section: 'STYLE', applies: isBox },
   { name: 'grain', kind: 'text', section: 'STYLE', applies: isBox },
   { name: 'mask', kind: 'text', section: 'STYLE', applies: isBox },
   { name: 'rotate', kind: 'num', section: 'STYLE' },
   { name: 'scale', kind: 'text', section: 'STYLE' },
   { name: 'tilt', kind: 'text', section: 'STYLE' },
   {
      name: 'fit',
      kind: 'enum',
      section: 'STYLE',
      options: ['cover', 'contain', 'stretch'],
      core: (n) => n.kind === 'img',
      applies: (n) => n.kind === 'img',
   },
   { name: 'style', kind: 'text', section: 'STYLE' },
   { name: 'color', kind: 'color', section: 'TEXT', core: isText },
   { name: 'size', kind: 'num', section: 'TEXT', core: isText },
   { name: 'weight', kind: 'num', section: 'TEXT', core: isText },
   { name: 'family', kind: 'text', section: 'TEXT' },
   { name: 'leading', kind: 'num', section: 'TEXT' },
   { name: 'tracking', kind: 'num', section: 'TEXT' },
   {
      name: 'align-text',
      kind: 'enum',
      section: 'TEXT',
      options: ['start', 'center', 'end'],
      applies: (n) => isText(n),
   },
   { name: 'act', kind: 'text', section: 'BINDINGS' },
   { name: 'field', kind: 'text', section: 'BINDINGS' },
   { name: 'submit', kind: 'text', section: 'BINDINGS' },
   { name: 'key', kind: 'text', section: 'BINDINGS' },
   { name: 'press', kind: 'text', section: 'BINDINGS' },
   { name: 'context', kind: 'text', section: 'BINDINGS' },
   { name: 'dblclick', kind: 'text', section: 'BINDINGS' },
   { name: 'drag', kind: 'text', section: 'BINDINGS' },
   { name: 'drop', kind: 'text', section: 'BINDINGS' },
   { name: 'resize', kind: 'text', section: 'BINDINGS' },
   { name: 'pointer-move', kind: 'text', section: 'BINDINGS' },
   { name: 'pointer-up', kind: 'text', section: 'BINDINGS' },
   { name: 'drag-update', kind: 'text', section: 'BINDINGS' },
   { name: 'drag-end', kind: 'text', section: 'BINDINGS' },
   { name: 'role', kind: 'text', section: 'BINDINGS', core: always },
   { name: 'label', kind: 'text', section: 'BINDINGS', core: always },
   { name: 'desc', kind: 'text', section: 'BINDINGS', core: always },
   {
      name: 'checked',
      kind: 'enum',
      section: 'BINDINGS',
      options: ['false', 'true', 'mixed'],
      core: always,
   },
   {
      name: 'expanded',
      kind: 'enum',
      section: 'BINDINGS',
      options: ['false', 'true'],
      core: always,
   },
   {
      name: 'selected',
      kind: 'enum',
      section: 'BINDINGS',
      options: ['false', 'true'],
      core: always,
   },
   { name: 'active-descendant', kind: 'text', section: 'BINDINGS', core: always },
   { name: 'controls', kind: 'text', section: 'BINDINGS', core: always },
   { name: 'value-now', kind: 'num', section: 'BINDINGS', core: always },
   { name: 'value-min', kind: 'num', section: 'BINDINGS', core: always },
   { name: 'value-max', kind: 'num', section: 'BINDINGS', core: always },
   { name: 'value-text', kind: 'text', section: 'BINDINGS', core: always },
   {
      name: 'modal',
      kind: 'enum',
      section: 'BINDINGS',
      options: ['false', 'true'],
      core: always,
   },
   {
      name: 'live',
      kind: 'enum',
      section: 'BINDINGS',
      options: ['off', 'polite', 'assertive'],
      core: always,
   },
   {
      name: 'live-atomic',
      kind: 'enum',
      section: 'BINDINGS',
      options: ['false', 'true'],
      core: always,
   },
   { name: 'level', kind: 'num', section: 'BINDINGS', core: always },
   { name: 'pos-in-set', kind: 'num', section: 'BINDINGS', core: always },
   { name: 'set-size', kind: 'num', section: 'BINDINGS', core: always },
];

const FLAG_NAMES = ['clip', 'nowrap', 'ellipsis', 'bleed', 'inert', 'focusable'];
const EASINGS = ['linear', 'ease', 'ease-in', 'ease-out', 'ease-in-out'];

const ANIM_MODES = ['loop', 'once', 'alternate'];

const SECTION_ORDER = ['LAYOUT', 'STYLE', 'TEXT', 'BINDINGS'];

/** #rgb/#rgba/#rrggbb/#rrggbbaa → css color for swatches (else null). */
function cssHex(v: string): string | null {
   return /^#[0-9a-fA-F]{3,8}$/.test(v) ? v : null;
}

export interface Inspector {
   /** Repaint for a (possibly null) selection. */
   render(sel: Selection | null): void;
   /** Re-render after external doc changes unless the user is typing here. */
   refresh(): void;
}
interface SemanticSceneFields {
   checked: boolean | 'mixed' | null;
   expanded: boolean | null;
   selected: boolean | null;
   active_descendant: string;
   controls: string;
   value_now: number | null;
   value_min: number | null;
   value_max: number | null;
   value_text: string;
   modal: boolean | null;
   live: 'off' | 'polite' | 'assertive' | null;
   live_atomic: boolean | null;
   level: number | null;
   pos_in_set: number | null;
   set_size: number | null;
   disabled: boolean;
   focused: boolean;
}

interface LiveListValue {
   signature: string;
   value: unknown[];
}

interface SourceAnchor {
   def: string | null;
   path: number[] | null;
   line: number;
   kind: string;
}

export function createInspector(host: InspectorHost): Inspector {
   const { body, meta, crumbs } = host;
   let curKey: string | null = null;
   let curAnchor: SourceAnchor | null = null;
   const listValues = new Map<string, LiveListValue>();

   function sourceAnchor(doc: SrcDoc, node: SrcNode, def: string | null): SourceAnchor {
      const path: number[] = [];
      let root = node;
      while (root.parent) {
         path.unshift(root.parent.children.indexOf(root));
         root = root.parent;
      }
      const roots = def ? doc.defs.get(def)?.body : doc.roots;
      const rootIndex = roots?.indexOf(root) ?? -1;
      if (rootIndex >= 0) path.unshift(rootIndex);
      return {
         def,
         path: rootIndex >= 0 ? path : null,
         line: node.line,
         kind: node.kind,
      };
   }

   /** Resolve key-addressed and materialized-list selections against the live parse. */
   function liveNode(): SrcNode | null {
      const doc = host.doc();
      if (curKey) {
         const keyed = resolveKey(doc, curKey)?.node;
         if (keyed) return keyed;
      }
      if (!curAnchor) return null;
      if (curAnchor.path) {
         let list = curAnchor.def ? doc.defs.get(curAnchor.def)?.body : doc.roots;
         let node: SrcNode | undefined;
         for (const index of curAnchor.path) {
            node = list?.[index];
            if (!node) break;
            list = node.children;
         }
         if (node?.kind === curAnchor.kind) return node;
      }
      const byLine = nodeAtLine(doc, curAnchor.line);
      return byLine?.def === curAnchor.def && byLine.node.kind === curAnchor.kind
         ? byLine.node
         : null;
   }

   function commitAttr(name: string, value: string): void {
      const node = liveNode();
      if (!node) return;
      const v = value.trim();
      host.apply(v === '' ? removeAttr(host.doc(), node, name) : setAttr(node, name, v), true);
      host.compileSoon();
   }

   // ── field widgets ─────────────────────────────────────────────────

   function rowEl(label: string): { row: HTMLDivElement; slot: HTMLDivElement } {
      const row = document.createElement('div');
      row.className = 'insp-row';
      const lab = document.createElement('div');
      lab.className = 'insp-label';
      lab.textContent = label.toUpperCase();
      const slot = document.createElement('div');
      slot.className = 'insp-field';
      row.append(lab, slot);
      return { row, slot };
   }

   function textInput(
      value: string,
      placeholder: string,
      commit: (v: string) => void,
   ): HTMLInputElement {
      const input = document.createElement('input');
      input.className = 'insp-input';
      input.value = value;
      input.placeholder = placeholder;
      input.spellcheck = false;
      input.addEventListener('keydown', (e) => {
         if (e.key === 'Enter') {
            commit(input.value);
            input.blur();
         }
         if (e.key === 'Escape') input.blur();
         e.stopPropagation();
      });
      input.addEventListener('change', () => commit(input.value));
      return input;
   }

   function listInput(
      value: unknown[],
      commit: (input: HTMLTextAreaElement) => void,
   ): HTMLTextAreaElement {
      const input = document.createElement('textarea');
      input.className = 'insp-input insp-list';
      input.value = JSON.stringify(value);
      input.placeholder = '[]';
      input.spellcheck = false;
      input.addEventListener('keydown', (event) => {
         if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
            commit(input);
            input.blur();
         }
         if (event.key === 'Escape') input.blur();
         event.stopPropagation();
      });
      input.addEventListener('change', () => commit(input));
      return input;
   }

   function commitScrollMode(next: ScrollMode): void {
      const node = liveNode();
      if (!node || next === nodeScrollMode(node)) return;
      const doc = host.doc();
      const changes: Change[] = [];
      if (next === 'off' || next === 'main') {
         changes.push(...removeAttr(doc, node, 'scroll'));
      } else {
         changes.push(...setAttr(node, 'scroll', next));
      }
      changes.push(...setFlag(doc, node, 'scroll', next === 'main'));
      host.apply(changes, true);
      host.compileSoon();
   }

   function toggleContextFlag(name: 'drag-ghost' | 'sticky' | 'virtual', on: boolean): void {
      const node = liveNode();
      if (!node) return;
      const doc = host.doc();
      const changes = setFlag(doc, node, name, on);
      if (name === 'virtual' && on && !node.attrs.some((attr) => attr.name === 'item-extent')) {
         const insertion = changes.find(
            (change) => change.from === node.headerSpan.to && change.to === node.headerSpan.to,
         );
         if (insertion) insertion.insert += ' item-extent=20';
         else changes.push(...setAttr(node, 'item-extent', '20'));
      }
      host.apply(changes, true);
      host.compileSoon();
   }

   /** Label scrubbing: horizontal pointer drag adjusts numeric fields. */
   function scrubbable(label: HTMLElement, input: HTMLInputElement, name: string): void {
      label.classList.add('scrub');
      label.addEventListener('pointerdown', (down) => {
         const node0 = liveNode();
         if (!node0) return;
         const start =
            Number.parseFloat(
               input.value || node0.attrs.find((a) => a.name === name)?.value || '0',
            ) || 0;
         const text0 = host.text();
         let moved = false;
         label.setPointerCapture(down.pointerId);
         const onMove = (e: PointerEvent) => {
            const step = e.shiftKey ? 10 : e.altKey ? 0.1 : 1;
            const raw = start + (e.clientX - down.clientX) * step;
            const val = step < 1 ? Math.round(raw * 10) / 10 : Math.round(raw);
            if (!moved && val === start) return;
            moved = true;
            const node = liveNode();
            if (!node) return;
            input.value = String(val);
            host.apply(setAttr(node, name, String(val)), false);
            host.compileSoon();
         };
         const onUp = () => {
            label.removeEventListener('pointermove', onMove);
            label.removeEventListener('pointerup', onUp);
            if (moved) host.commitDrag(text0);
         };
         label.addEventListener('pointermove', onMove);
         label.addEventListener('pointerup', onUp);
      });
   }

   function fieldWidget(f: Field, node: SrcNode): HTMLElement {
      const current = node.attrs.find((a) => a.name === f.name)?.value ?? '';
      if (f.kind === 'enum' && f.options && current !== '' && !f.options.includes(current)) {
         return textInput(current, 'literal or ref', (value) => commitAttr(f.name, value));
      }
      if (f.kind === 'enum' && f.options && f.options.length <= 4) {
         const seg = document.createElement('div');
         seg.className = 'seg insp-seg';
         for (const opt of f.options) {
            const b = document.createElement('button');
            b.type = 'button';
            b.textContent = opt.toUpperCase();
            b.classList.toggle('on', current === opt);
            b.addEventListener('click', () => {
               commitAttr(f.name, current === opt ? '' : opt);
            });
            seg.appendChild(b);
         }
         return seg;
      }
      if (f.kind === 'enum' && f.options) {
         const sel = document.createElement('select');
         sel.className = 'insp-input';
         const none = document.createElement('option');
         none.value = '';
         none.textContent = '—';
         sel.appendChild(none);
         for (const opt of f.options) {
            const o = document.createElement('option');
            o.value = opt;
            o.textContent = opt;
            sel.appendChild(o);
         }
         sel.value = f.options.includes(current) ? current : '';
         sel.addEventListener('change', () => commitAttr(f.name, sel.value));
         return sel;
      }
      if (f.kind === 'color') {
         const wrap = document.createElement('div');
         wrap.className = 'insp-color';
         const swatch = document.createElement('label');
         swatch.className = 'insp-swatch';
         const pick = document.createElement('input');
         pick.type = 'color';
         const hex = cssHex(current.length === 9 ? current.slice(0, 7) : current);
         if (hex) {
            swatch.style.background = current;
            if (hex.length === 7) pick.value = hex;
            else if (hex.length === 4) {
               pick.value = `#${hex[1]}${hex[1]}${hex[2]}${hex[2]}${hex[3]}${hex[3]}`;
            }
         } else if (current !== '') {
            swatch.classList.add('token');
         }
         const input = textInput(current, '#rrggbb · color.token', (v) => commitAttr(f.name, v));
         input.setAttribute('list', 'vv-token-colors');
         let pickText0: string | null = null;
         pick.addEventListener('input', () => {
            const node2 = liveNode();
            if (!node2) return;
            pickText0 ??= host.text();
            input.value = pick.value;
            swatch.style.background = pick.value;
            host.apply(setAttr(node2, f.name, pick.value), false);
            host.compileSoon();
         });
         pick.addEventListener('change', () => {
            if (pickText0 !== null) host.commitDrag(pickText0);
            pickText0 = null;
         });
         swatch.appendChild(pick);
         wrap.append(swatch, input);
         return wrap;
      }
      const placeholder = f.kind === 'size' ? 'hug · fill · 240 · 50%' : '';
      const input = textInput(current, placeholder, (v) => commitAttr(f.name, v));
      if (f.kind === 'size') input.setAttribute('list', 'vv-sizes');
      return input;
   }
   /** 3×3 placement matrix for row/col/wrap: visual x/y slots map onto
    * pack (main axis) and align (cross axis) by the container's axis.
    * BETWEEN and BASELINE ride as chips (off-matrix vocabulary). */
   function alignMatrix(node: SrcNode): HTMLElement {
      const sec = document.createElement('div');
      sec.className = 'insp-sec';
      const eyebrow = document.createElement('div');
      eyebrow.className = 'insp-eyebrow';
      eyebrow.textContent = `ALIGN · ${node.kind === 'col' ? 'MAIN ↓' : 'MAIN →'}`;
      const wrap = document.createElement('div');
      wrap.className = 'insp-align';
      const grid = document.createElement('div');
      grid.className = 'align-grid';
      const isRow = node.kind !== 'col';
      const packVal = node.attrs.find((a) => a.name === 'pack')?.value ?? 'start';
      const alignVal = node.attrs.find((a) => a.name === 'align')?.value ?? 'start';
      const setBoth = (pack: string, align: string): void => {
         const live = liveNode();
         if (!live) return;
         const doc = host.doc();
         const changes = [
            ...(pack === 'start' ? removeAttr(doc, live, 'pack') : setAttr(live, 'pack', pack)),
            ...(align === 'start' ? removeAttr(doc, live, 'align') : setAttr(live, 'align', align)),
         ];
         host.apply(changes, true);
         host.compileSoon();
      };
      const POS = ['start', 'center', 'end'];
      for (const vy of POS) {
         for (const vx of POS) {
            const main = isRow ? vx : vy;
            const cross = isRow ? vy : vx;
            const cell = document.createElement('button');
            cell.type = 'button';
            cell.className = 'align-cell';
            cell.title = `pack=${main} align=${cross}`;
            cell.classList.toggle('on', packVal === main && alignVal === cross);
            cell.addEventListener('click', () => setBoth(main, cross));
            grid.appendChild(cell);
         }
      }
      const chips = document.createElement('div');
      chips.className = 'insp-flags';
      const between = document.createElement('button');
      between.type = 'button';
      between.className = `insp-flag${packVal === 'between' ? ' on' : ''}`;
      between.textContent = 'BETWEEN';
      between.addEventListener('click', () => {
         setBoth(packVal === 'between' ? 'start' : 'between', alignVal);
      });
      const baseline = document.createElement('button');
      baseline.type = 'button';
      baseline.className = `insp-flag${alignVal === 'baseline' ? ' on' : ''}`;
      baseline.textContent = 'BASELINE';
      baseline.addEventListener('click', () => {
         setBoth(packVal, alignVal === 'baseline' ? 'start' : 'baseline');
      });
      chips.append(between, baseline);
      wrap.append(grid, chips);
      sec.append(eyebrow, wrap);
      return sec;
   }

   /** MOTION composer: `animate=name,dur[,mode][,easing]` + `transition=
    * dur[,easing]` from parts editors; the raw attr stays authoritative. */
   function motionSection(node: SrcNode): HTMLElement {
      const { sec, grid } = sectionEl('MOTION');
      const doc = host.doc();
      const animateRaw = node.attrs.find((a) => a.name === 'animate')?.value ?? '';
      const parts = animateRaw.split(',').map((s) => s.trim());
      const curName = parts[0] ?? '';
      const curDur = parts[1] ?? '';
      const curMode = parts.find((x) => ANIM_MODES.includes(x)) ?? 'loop';
      const curEase = parts.find((x) => EASINGS.includes(x)) ?? 'linear';
      const writeAnimate = (name: string, dur: string, mode: string, ease: string): void => {
         if (name === '') {
            commitAttr('animate', '');
            return;
         }
         commitAttr('animate', `${name},${dur || '600'},${mode},${ease}`);
      };
      {
         const { row, slot } = rowEl('animate');
         const sel = document.createElement('select');
         sel.className = 'insp-input';
         const none = document.createElement('option');
         none.value = '';
         none.textContent = '—';
         sel.appendChild(none);
         for (const an of doc.anims) {
            const o = document.createElement('option');
            o.value = an.name;
            o.textContent = an.name;
            sel.appendChild(o);
         }
         sel.value = doc.anims.some((a) => a.name === curName) ? curName : '';
         sel.addEventListener('change', () => writeAnimate(sel.value, curDur, curMode, curEase));
         const dur = textInput(curDur, 'ms', (v) => writeAnimate(curName, v, curMode, curEase));
         dur.classList.add('insp-narrow');
         slot.append(sel, dur);
         grid.appendChild(row);
      }
      if (curName !== '') {
         const { row, slot } = rowEl('mode');
         const seg = document.createElement('div');
         seg.className = 'seg insp-seg';
         for (const m of ANIM_MODES) {
            const b = document.createElement('button');
            b.type = 'button';
            b.textContent = m.toUpperCase();
            b.classList.toggle('on', curMode === m);
            b.addEventListener('click', () => writeAnimate(curName, curDur, m, curEase));
            seg.appendChild(b);
         }
         slot.appendChild(seg);
         grid.appendChild(row);
         const easeRow = rowEl('easing');
         const es = document.createElement('select');
         es.className = 'insp-input';
         for (const e of EASINGS) {
            const o = document.createElement('option');
            o.value = e;
            o.textContent = e;
            es.appendChild(o);
         }
         es.value = curEase;
         es.addEventListener('change', () => writeAnimate(curName, curDur, curMode, es.value));
         easeRow.slot.appendChild(es);
         grid.appendChild(easeRow.row);
      }
      {
         const transRaw = node.attrs.find((a) => a.name === 'transition')?.value ?? '';
         const tparts = transRaw.split(',').map((s) => s.trim());
         const tdur = tparts[0] ?? '';
         const tease = tparts.find((x) => EASINGS.includes(x)) ?? 'ease-out';
         const write = (d: string, e: string): void => {
            commitAttr('transition', d === '' ? '' : `${d},${e}`);
         };
         const { row, slot } = rowEl('transition');
         const dur = textInput(tdur, 'ms', (v) => write(v, tease));
         dur.classList.add('insp-narrow');
         const es = document.createElement('select');
         es.className = 'insp-input';
         for (const e of EASINGS) {
            const o = document.createElement('option');
            o.value = e;
            o.textContent = e;
            es.appendChild(o);
         }
         es.value = tease;
         es.addEventListener('change', () => write(tdur === '' ? '200' : tdur, es.value));
         slot.append(dur, es);
         grid.appendChild(row);
      }
      return sec;
   }

   /** Document-level animation designer: every `anim` block with editable
    * stops (pct + raw attrs), stop add/remove, and new-anim scaffolding. */
   function animsSection(): HTMLElement {
      const doc = host.doc();
      const sec = document.createElement('div');
      sec.className = 'insp-sec';
      const eyebrow = document.createElement('div');
      eyebrow.className = 'insp-eyebrow';
      eyebrow.textContent = 'ANIMATIONS';
      sec.appendChild(eyebrow);
      for (const an of doc.anims) {
         const head = document.createElement('div');
         head.className = 'anim-head';
         const name = document.createElement('button');
         name.type = 'button';
         name.className = 'anim-name';
         name.textContent = an.name.toUpperCase();
         name.title = 'go to source';
         name.addEventListener('click', () => host.revealPos(an.nameSpan.from));
         const add = document.createElement('button');
         add.type = 'button';
         add.className = 'insp-flag';
         add.textContent = '+ STOP';
         add.addEventListener('click', () => {
            const fresh = host.doc().anims.find((x) => x.name === an.name);
            if (!fresh) return;
            host.apply(
               [
                  {
                     from: fresh.closeAt,
                     to: fresh.closeAt,
                     insert: `${host.doc().indentUnit}50% { opacity=1 }\n`,
                  },
               ],
               true,
            );
            host.compileSoon();
            refreshSoon();
         });
         head.append(name, add);
         sec.appendChild(head);
         an.keyframes.forEach((kf, ki) => {
            const row = document.createElement('div');
            row.className = 'anim-kf';
            const pct = textInput(String(kf.pct), '%', (v) => {
               const fresh = host.doc().anims.find((x) => x.name === an.name)?.keyframes[ki];
               if (!fresh) return;
               const n = Number.parseFloat(v);
               if (Number.isNaN(n)) return;
               host.apply(
                  [{ from: fresh.pctSpan.from, to: fresh.pctSpan.to, insert: `${n}%` }],
                  true,
               );
               host.compileSoon();
            });
            pct.classList.add('insp-narrow');
            const bodyText = host
               .text()
               .slice(kf.bodySpan.from, kf.bodySpan.to)
               .replace(/\s*[\n;]\s*/g, '; ')
               .trim();
            const attrs = textInput(bodyText, 'attr=value; …', (v) => {
               const fresh = host.doc().anims.find((x) => x.name === an.name)?.keyframes[ki];
               if (!fresh) return;
               host.apply(
                  [{ from: fresh.bodySpan.from, to: fresh.bodySpan.to, insert: ` ${v.trim()} ` }],
                  true,
               );
               host.compileSoon();
            });
            const del = document.createElement('button');
            del.type = 'button';
            del.className = 'insp-flag';
            del.textContent = '×';
            del.addEventListener('click', () => {
               const fresh = host.doc().anims.find((x) => x.name === an.name)?.keyframes[ki];
               if (!fresh) return;
               const text = host.text();
               let from = fresh.span.from;
               while (from > 0 && (text[from - 1] === ' ' || text[from - 1] === '\t')) from--;
               let to = fresh.span.to;
               if (text[to] === '\n') to++;
               host.apply([{ from, to, insert: '' }], true);
               host.compileSoon();
               refreshSoon();
            });
            row.append(pct, attrs, del);
            sec.appendChild(row);
         });
      }
      const newBtn = document.createElement('button');
      newBtn.type = 'button';
      newBtn.className = 'insp-btn';
      newBtn.textContent = '+ NEW ANIMATION';
      newBtn.addEventListener('click', () => {
         const fresh = host.doc();
         let n = 1;
         let name = 'pulse';
         while (fresh.anims.some((a) => a.name === name)) name = `pulse${++n}`;
         const u = fresh.indentUnit;
         const tpl = `anim ${name} {\n${u}0% { opacity=1 }\n${u}100% { opacity=0.4 }\n}\n\n`;
         const first = fresh.roots[0];
         const at = first ? first.span.from - first.indent.length : fresh.text.length;
         host.apply([{ from: at, to: at, insert: tpl }], true);
         host.compileSoon();
         refreshSoon();
      });
      sec.appendChild(newBtn);
      return sec;
   }

   // ── sections ──────────────────────────────────────────────────────

   function renderCrumbs(sel: Selection): void {
      crumbs.replaceChildren();
      crumbs.hidden = false;
      const segs = sel.key.split('/');
      segs.forEach((seg, i) => {
         if (i > 0) {
            const s = document.createElement('span');
            s.className = 'crumb-sep';
            s.textContent = '/';
            crumbs.appendChild(s);
         }
         const b = document.createElement('button');
         b.type = 'button';
         b.className = `crumb${i === segs.length - 1 ? ' on' : ''}`;
         b.textContent = seg;
         const key = segs.slice(0, i + 1).join('/');
         b.addEventListener('click', () => host.vibe.select(key));
         crumbs.appendChild(b);
      });
   }

   function sectionEl(title: string): { sec: HTMLDivElement; grid: HTMLDivElement } {
      const sec = document.createElement('div');
      sec.className = 'insp-sec';
      const eyebrow = document.createElement('div');
      eyebrow.className = 'insp-eyebrow';
      eyebrow.textContent = title;
      const grid = document.createElement('div');
      grid.className = 'insp-grid';
      sec.append(eyebrow, grid);
      return { sec, grid };
   }

   function renderNode(sel: Selection, node: SrcNode, def: string | null): void {
      // head: kind + id
      const head = document.createElement('div');
      head.className = 'insp-head';
      const kind = document.createElement('span');
      kind.className = 'insp-kind';
      kind.textContent = node.kind.toUpperCase();
      head.appendChild(kind);
      const idInput = textInput(node.id ?? '', 'id', (v) => {
         const live = liveNode();
         if (!live) return;
         host.apply(setId(live, v.trim() === '' ? null : v.trim()), true);
         host.compileSoon();
      });
      idInput.classList.add('insp-id');
      const hashLabel = document.createElement('span');
      hashLabel.className = 'insp-hash';
      hashLabel.textContent = '#';
      head.append(hashLabel, idInput);
      body.appendChild(head);
      if (def) {
         const banner = document.createElement('div');
         banner.className = 'insp-banner';
         banner.textContent = `DEF ${def.toUpperCase()} — EDITS HIT EVERY INSTANCE`;
         body.appendChild(banner);
      }

      // Leaf positional values stay editable regardless of literal/ref form.
      // Component-call refs remain read-only because they bind def props.
      const editableArgs = node.args
         .map((arg, index) => ({ arg, index }))
         .filter(({ arg }) => !node.isCall || arg.kind === 'string');
      if (editableArgs.length > 0 || node.isCall) {
         const { sec, grid } = sectionEl(node.isCall ? 'PROPS' : 'CONTENT');
         for (const { arg, index } of editableArgs) {
            const firstLabel =
               node.kind === 'img'
                  ? 'src'
                  : node.kind === 'path'
                    ? 'd'
                    : node.kind === 'icon' || node.kind === 'hole'
                      ? 'name'
                      : 'text';
            const { row, slot } = rowEl(index === 0 ? firstLabel : `arg ${index}`);
            const value = arg.kind === 'string' ? arg.text.replace(/^"|"$/g, '') : arg.text;
            const input = textInput(value, 'literal or ref', (next) => {
               const live = liveNode();
               if (live) host.vibe.setNodeArg(live, index, next);
            });
            slot.appendChild(input);
            grid.appendChild(row);
         }
         if (node.isCall) {
            node.args.forEach((arg, index) => {
               if (arg.kind === 'string') return;
               const { row, slot } = rowEl(`ref ${index}`);
               const ro = document.createElement('div');
               ro.className = 'insp-ro';
               ro.textContent = arg.text;
               slot.appendChild(ro);
               grid.appendChild(row);
            });
         }
         if (grid.children.length > 0) body.appendChild(sec);
      }

      // 2-axis placement matrix for flow containers: x = horizontal slot,
      // y = vertical slot; writes pack= (main axis) + align= (cross axis)
      // per the container's own axis. `start` values erase the attr.
      if (node.kind === 'row' || node.kind === 'col' || node.kind === 'wrap') {
         body.appendChild(alignMatrix(node));
      }

      // attr sections from schema
      for (const section of SECTION_ORDER) {
         const fields = FIELDS.filter((f) => {
            if (f.section !== section) return false;
            const applies = f.applies ?? always;
            const has = node.attrs.some((a) => a.name === f.name);
            return has || (applies(node) && (f.core?.(node) ?? false));
         });
         if (fields.length === 0) continue;
         const { sec, grid } = sectionEl(section);
         for (const f of fields) {
            const { row, slot } = rowEl(f.name);
            const widget = fieldWidget(f, node);
            slot.appendChild(widget);
            if (f.kind === 'num' && widget instanceof HTMLInputElement) {
               scrubbable(row.querySelector('.insp-label') as HTMLElement, widget, f.name);
            }
            grid.appendChild(row);
         }
         body.appendChild(sec);
      }
      body.appendChild(motionSection(node));

      // Contextual flags: scroll is one four-state control; virtual/sticky
      // only appear where the compiler accepts them (or when present for repair).
      {
         const { sec, grid } = sectionEl('FLAGS');
         const hasScrollSyntax =
            node.flags.some((flag) => flag.name === 'scroll') ||
            node.attrs.some((attr) => attr.name === 'scroll');
         if (supportsScroll(node) || hasScrollSyntax) {
            const { row, slot } = rowEl('scroll');
            const seg = document.createElement('div');
            seg.className = 'seg insp-seg';
            for (const mode of ['off', 'main', 'cross', 'both'] as ScrollMode[]) {
               const button = document.createElement('button');
               button.type = 'button';
               button.textContent = mode.toUpperCase();
               button.classList.toggle('on', nodeScrollMode(node) === mode);
               button.addEventListener('click', () => commitScrollMode(mode));
               seg.appendChild(button);
            }
            slot.appendChild(seg);
            grid.appendChild(row);
         }

         const { row, slot } = rowEl('flags');
         slot.classList.add('insp-flags');
         for (const name of FLAG_NAMES) {
            const on = node.flags.some((flag) => flag.name === name);
            const chip = document.createElement('button');
            chip.type = 'button';
            chip.className = `insp-flag${on ? ' on' : ''}`;
            chip.textContent = name.toUpperCase();
            chip.addEventListener('click', () => {
               const live = liveNode();
               if (!live) return;
               host.apply(setFlag(host.doc(), live, name, !on), true);
               host.compileSoon();
            });
            slot.appendChild(chip);
         }
         for (const contextual of [
            { name: 'sticky' as const, applies: supportsSticky(node) },
            { name: 'virtual' as const, applies: supportsVirtual(node) },
            { name: 'drag-ghost' as const, applies: supportsDragGhost(node) },
         ]) {
            const on = node.flags.some((flag) => flag.name === contextual.name);
            if (!contextual.applies && !on) continue;
            const chip = document.createElement('button');
            chip.type = 'button';
            chip.className = `insp-flag${on ? ' on' : ''}`;
            chip.textContent = contextual.name.toUpperCase();
            if (contextual.name === 'virtual') {
               chip.title = 'Requires a uniform item-extent; defaults to 20';
            }
            chip.addEventListener('click', () => toggleContextFlag(contextual.name, !on));
            slot.appendChild(chip);
         }
         if (slot.children.length > 0) grid.appendChild(row);
         if (grid.children.length > 0) body.appendChild(sec);
      }

      // add-attr
      {
         const sec = document.createElement('div');
         sec.className = 'insp-sec';
         const eyebrow = document.createElement('div');
         eyebrow.className = 'insp-eyebrow';
         eyebrow.textContent = 'ADD ATTRIBUTE';
         const rowl = document.createElement('div');
         rowl.className = 'insp-add';
         const nameIn = document.createElement('input');
         nameIn.className = 'insp-input';
         nameIn.placeholder = 'attr';
         nameIn.setAttribute('list', 'vv-attr-names');
         nameIn.spellcheck = false;
         const valIn = textInput('', 'value', () => {});
         const go = document.createElement('button');
         go.type = 'button';
         go.className = 'insp-set';
         go.textContent = 'SET';
         const commit = () => {
            if (nameIn.value.trim() === '' || valIn.value.trim() === '') return;
            commitAttr(nameIn.value.trim(), valIn.value.trim());
            nameIn.value = '';
            valIn.value = '';
         };
         go.addEventListener('click', commit);
         valIn.addEventListener('keydown', (e) => {
            if (e.key === 'Enter') commit();
            e.stopPropagation();
         });
         rowl.append(nameIn, valIn, go);
         sec.append(eyebrow, rowl);
         body.appendChild(sec);
      }

      // actions
      {
         const actions = document.createElement('div');
         actions.className = 'insp-actions';
         const dup = document.createElement('button');
         dup.type = 'button';
         dup.className = 'insp-btn';
         dup.textContent = 'DUPLICATE';
         dup.addEventListener('click', () => {
            const doc = host.doc();
            const live = liveNode();
            if (!live) return;
            const edit = duplicateNode(doc, live);
            host.apply(edit.changes, true);
            const copy = nodeAtPos(host.doc(), edit.caret);
            if (copy) host.vibe.select(keyForNode(copy));
            host.compileSoon();
         });
         const del = document.createElement('button');
         del.type = 'button';
         del.className = 'insp-btn danger';
         del.textContent = 'DELETE';
         del.addEventListener('click', () => {
            const doc = host.doc();
            const live = liveNode();
            if (!live) return;
            const up = sel.key.includes('/') ? sel.key.slice(0, sel.key.lastIndexOf('/')) : null;
            host.apply(deleteNode(doc, live), true);
            host.vibe.select(up);
            host.compileSoon();
         });
         actions.append(dup, del);
         body.appendChild(actions);
      }

      // Runtime metadata is retained-scene data; materialized node ids are
      // not scene indices, so selection identity is always the authored key.
      const scene = host.view.sceneSnapshot().find((candidate) => candidate.key === sel.key);
      if (scene) {
         const runtimeProps: [string, string | number | boolean][] = [];
         if ((scene.flags & 4) !== 0) {
            runtimeProps.push(['scroll main', scene.scroll_off]);
            runtimeProps.push(['content main', scene.content_main]);
         }
         if ((scene.flags & 512) !== 0) {
            runtimeProps.push(['scroll cross', scene.scroll_cross]);
            runtimeProps.push(['content cross', scene.content_cross]);
         }
         if (scene.role !== '') runtimeProps.push(['role', scene.role]);
         if (scene.label !== '') runtimeProps.push(['label', scene.label]);
         if (scene.desc !== '') runtimeProps.push(['desc', scene.desc]);
         const semantic = scene as typeof scene & Partial<SemanticSceneFields>;
         const addSemantic = (
            name: string,
            value: string | number | boolean | null | undefined,
         ): void => {
            if (value !== null && value !== undefined && value !== '') {
               runtimeProps.push([name, value]);
            }
         };
         addSemantic('checked', semantic.checked);
         addSemantic('expanded', semantic.expanded);
         addSemantic('selected', semantic.selected);
         addSemantic('active descendant', semantic.active_descendant);
         addSemantic('controls', semantic.controls);
         addSemantic('value now', semantic.value_now);
         addSemantic('value min', semantic.value_min);
         addSemantic('value max', semantic.value_max);
         addSemantic('value text', semantic.value_text);
         addSemantic('modal', semantic.modal);
         addSemantic('live', semantic.live);
         addSemantic('live atomic', semantic.live_atomic);
         addSemantic('level', semantic.level);
         addSemantic('pos in set', semantic.pos_in_set);
         addSemantic('set size', semantic.set_size);
         addSemantic('disabled', semantic.disabled);
         addSemantic('focused', semantic.focused);
         if (runtimeProps.length > 0) {
            const { sec, grid } = sectionEl('RUNTIME');
            for (const [name, value] of runtimeProps) {
               const { row, slot } = rowEl(name);
               const ro = document.createElement('div');
               ro.className = 'insp-ro';
               ro.textContent = String(value);
               slot.appendChild(ro);
               grid.appendChild(row);
            }
            body.appendChild(sec);
         }
      }
   }

   function liveListValue(param: SrcParam): LiveListValue {
      const signature = `${param.type}\u0000${param.def}`;
      const cached = listValues.get(param.name);
      if (cached?.signature === signature) return cached;
      const value = parseListDefault(param.def) ?? [];
      const next = { signature, value };
      listValues.set(param.name, next);
      return next;
   }

   function updateNestedList(root: unknown[], path: string, value: unknown[]): boolean {
      const parts = path.split('.');
      let list = root;
      for (let part = 0; part < parts.length; part += 2) {
         const index = Number(parts[part]);
         const field = parts[part + 1];
         const item = list[index];
         if (
            !Number.isInteger(index) ||
            index < 0 ||
            !field ||
            item === null ||
            typeof item !== 'object' ||
            Array.isArray(item)
         ) {
            return false;
         }
         const record = item as Record<string, unknown>;
         if (part === parts.length - 2) {
            record[field] = value;
            return true;
         }
         const nested = record[field];
         if (!Array.isArray(nested)) return false;
         list = nested;
      }
      return false;
   }

   function appendListControl(
      grid: HTMLElement,
      doc: SrcDoc,
      param: SrcParam,
      state: LiveListValue,
      schema: SrcListRowSchema,
      path: string,
      value: unknown[],
      label: string,
   ): void {
      const { row, slot } = rowEl(label);
      row.classList.add('insp-list-row');
      const labelElement = row.querySelector('.insp-label');
      if (labelElement instanceof HTMLElement) {
         labelElement.title = path === '' ? param.name : `${param.name}:${path}`;
      }
      const input = listInput(value, (editor) => {
         let parsed: unknown;
         try {
            parsed = JSON.parse(editor.value) as unknown;
         } catch {
            parsed = null;
         }
         if (!Array.isArray(parsed)) {
            editor.setCustomValidity('Enter a JSON array');
            editor.classList.add('invalid');
            editor.reportValidity();
            return;
         }
         if (!host.view.setList(param.name, path, parsed)) {
            editor.setCustomValidity('List does not match the live Slab schema');
            editor.classList.add('invalid');
            editor.reportValidity();
            return;
         }
         editor.setCustomValidity('');
         editor.classList.remove('invalid');
         if (path === '') state.value = parsed;
         else updateNestedList(state.value, path, parsed);
         refreshSoon();
      });
      slot.appendChild(input);
      grid.appendChild(row);

      value.forEach((item, index) => {
         if (item === null || typeof item !== 'object' || Array.isArray(item)) return;
         const record = item as Record<string, unknown>;
         for (const field of schema.fields) {
            if (field.sub === 0) continue;
            const nestedSchema = doc.listSchemaRows[field.sub - 1];
            if (!nestedSchema) continue;
            const nested = Array.isArray(record[field.name]) ? record[field.name] : [];
            const nestedPath = path ? `${path}.${index}.${field.name}` : `${index}.${field.name}`;
            appendListControl(
               grid,
               doc,
               param,
               state,
               nestedSchema,
               nestedPath,
               nested,
               `${index}.${field.name}`,
            );
         }
      });
   }

   function renderEmpty(): void {
      crumbs.hidden = true;
      crumbs.replaceChildren();
      meta.textContent = 'DOCUMENT';
      const doc = host.doc();
      const hint = document.createElement('div');
      hint.className = 'insp-hint';
      hint.textContent = 'CLICK A NODE TO INSPECT · DRAG TO MOVE · DBLCLICK TO EDIT TEXT';
      body.appendChild(hint);
      if (doc.params.length > 0) {
         const { sec, grid } = sectionEl('PARAMS');
         for (const param of doc.params) {
            if (/^list\s*\(/.test(param.type)) {
               const state = liveListValue(param);
               const schema = doc.listSchemas[param.name];
               if (schema) {
                  appendListControl(grid, doc, param, state, schema, '', state.value, param.name);
               } else {
                  const { row, slot } = rowEl(param.name);
                  const input = listInput(state.value, () => {});
                  input.disabled = true;
                  input.title = 'List schema is unavailable until this document compiles';
                  slot.appendChild(input);
                  grid.appendChild(row);
               }
               continue;
            }
            const { row, slot } = rowEl(param.name);
            const current = host.view.getParam(param.name);
            if (param.type === 'bool') {
               const on = current === undefined ? param.def === 'true' : current === true;
               const chip = document.createElement('button');
               chip.type = 'button';
               chip.className = `insp-flag${on ? ' on' : ''}`;
               chip.textContent = on ? 'TRUE' : 'FALSE';
               chip.addEventListener('click', () => {
                  host.view.setParam(param.name, !on);
                  refreshSoon();
               });
               slot.appendChild(chip);
            } else {
               const input = textInput(
                  String(current ?? param.def.replace(/^"|"$/g, '')),
                  param.type,
                  (value) => host.view.setParam(param.name, value),
               );
               slot.appendChild(input);
            }
            grid.appendChild(row);
         }
         body.appendChild(sec);
      }
      body.appendChild(animsSection());
      const stats = document.createElement('div');
      stats.className = 'insp-stats';
      const count = host.view.sceneSnapshot().length;
      stats.textContent = `${count} SCENE NODES · ${doc.defs.size} DEFS · ${doc.params.length} PARAMS`;
      body.appendChild(stats);
   }

   let refreshQueued = false;
   function refreshSoon(): void {
      if (refreshQueued) return;
      refreshQueued = true;
      requestAnimationFrame(() => {
         refreshQueued = false;
         api.refresh();
      });
   }

   function renderDatalists(): void {
      const doc = host.doc();
      let colors = document.getElementById('vv-token-colors');
      if (!colors) {
         colors = document.createElement('datalist');
         colors.id = 'vv-token-colors';
         document.body.appendChild(colors);
         const sizes = document.createElement('datalist');
         sizes.id = 'vv-sizes';
         for (const v of ['hug', 'fill', 'fill:2', '100%', '50%']) {
            const o = document.createElement('option');
            o.value = v;
            sizes.appendChild(o);
         }
         document.body.appendChild(sizes);
         const attrs = document.createElement('datalist');
         attrs.id = 'vv-attr-names';
         for (const f of FIELDS) {
            const o = document.createElement('option');
            o.value = f.name;
            attrs.appendChild(o);
         }
         document.body.appendChild(attrs);
      }
      colors.replaceChildren();
      for (const p of doc.tokenPaths) {
         const o = document.createElement('option');
         o.value = p;
         colors.appendChild(o);
      }
   }

   const api: Inspector = {
      render(sel: Selection | null): void {
         curKey = sel?.key ?? null;
         curAnchor = sel?.target ? sourceAnchor(host.doc(), sel.target.node, sel.target.def) : null;
         body.replaceChildren();
         renderDatalists();
         if (!sel?.target) {
            if (sel) {
               // scene node without source anchor
               meta.textContent = sel.kind.toUpperCase();
               renderCrumbs(sel);
               const hint = document.createElement('div');
               hint.className = 'insp-hint';
               hint.textContent = 'NO SOURCE ANCHOR FOR THIS NODE';
               body.appendChild(hint);
               return;
            }
            renderEmpty();
            return;
         }
         meta.textContent = `${sel.kind.toUpperCase()}${sel.target.def ? ` · DEF ${sel.target.def.toUpperCase()}` : ''}`;
         renderCrumbs(sel);
         renderNode(sel, sel.target.node, sel.target.def);
      },
      refresh(): void {
         // repaint freely under button clicks; only typing keeps the DOM
         const a = document.activeElement;
         const typing =
            a instanceof HTMLElement &&
            body.contains(a) &&
            (a.tagName === 'INPUT' || a.tagName === 'TEXTAREA' || a.tagName === 'SELECT');
         if (typing) return;
         api.render(host.vibe.selection());
      },
   };
   return api;
}
