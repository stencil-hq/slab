// slab vibeviewer — CodeMirror editor + live in-page preview + design mode.
//
// The preview is ONE long-lived <slab-view> element (SlabElement subclass):
// every compile goes source → wasm `build` → SLIR bytes → `loadSlir` hot-swap
// on the same kernel instance the design overlay reads for hit-testing and
// geometry. No iframe, no gen_wc round-trip — the canvas, the inspector, and
// the text buffer stay one system with the source as the single truth.
//
// Workbench chrome follows the Cyanotype design system: pure black chassis,
// square geometry, cyan interaction prime, mono microlabels. Splitters are
// VSCode-style: pointer-driven, magnetic to center, double-click to reset.
//
// All URLs are relative — the site lives under
// https://stencil-hq.github.io/slab/.

import { defaultKeymap, history, historyKeymap, redo, undo } from '@codemirror/commands';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { type Diagnostic, linter, lintGutter } from '@codemirror/lint';
import { Annotation, EditorState, Transaction } from '@codemirror/state';
import {
   drawSelection,
   EditorView,
   highlightActiveLine,
   keymap,
   lineNumbers,
} from '@codemirror/view';
import { tags } from '@lezer/highlight';
import { SlabElement, type SlabSignalDetail } from '@stencil-hq/wslab';
import { createInspector } from './inspector.ts';
import { type Change, CONTAINERS, parseSlab, type SrcDoc, type SrcNode } from './slab-doc.ts';
import { slab } from './slab-lang.ts';
import { createTui, type Tui } from './tui.ts';
import { createVibe, type Vibe } from './vibe.ts';

type WasmModule = {
   default: () => Promise<void>;
   check: (src: string, file: string) => string;
   build: (src: string, assets: string) => Uint8Array;
};

let wasmReady: Promise<WasmModule> | null = null;

async function loadWasm() {
   if (!wasmReady) {
      const wasmUrl = new URL('./wasm/slab_wasm.js', import.meta.url).href;
      wasmReady = import(/* @vite-ignore */ wasmUrl).then(async (m) => {
         await m.default();
         return m as WasmModule;
      });
   }
   return wasmReady;
}

const diagsPane = document.getElementById('diags') as HTMLPreElement;
const select = document.getElementById('example-select') as HTMLSelectElement;
const statusState = document.getElementById('status-state') as HTMLSpanElement;
const statusSel = document.getElementById('status-sel') as HTMLSpanElement;
const statusDims = document.getElementById('status-dims') as HTMLSpanElement;
const panelMeta = document.getElementById('panel-meta') as HTMLSpanElement;

type DiagJson = {
   level: string;
   code: string;
   msg: string;
   line: number;
   remedy?: string;
   formatted: string;
};

// ── live preview element ─────────────────────────────────────────────

/** The single hot-swapped preview host (SLIR arrives via loadSlir). */
class SlabView extends SlabElement {}
customElements.define('slab-view', SlabView);

// Motion stays on the kernel clock: CSS-lifted animations would animate the
// DOM painter alone, leaving the design overlay and the terminal view — both
// of which read frames — frozen on a document that visibly moves.
SlabElement.lift = false;

const stage = document.getElementById('stage') as HTMLDivElement;
const view = new SlabView();
stage.appendChild(view);

// ── Cyanotype editor theme ───────────────────────────────────────────
// Code-block spec: keyword accent · string success · plain primary ·
// punctuation secondary · comment tertiary. Colors stay in the cyan
// instrument family — purple is reserved for live/presence, never syntax.

const cyanotypeHighlight = HighlightStyle.define([
   { tag: tags.keyword, color: '#44cfff' },
   { tag: tags.string, color: '#4ade80' },
   { tag: tags.number, color: '#f5f5f6' },
   { tag: tags.color, color: '#f5b04a' },
   { tag: tags.atom, color: '#75deff' },
   { tag: tags.propertyName, color: '#a3a3ac' },
   { tag: tags.variableName, color: '#f5f5f6' },
   { tag: tags.typeName, color: '#75deff' },
   { tag: tags.className, color: '#75deff' },
   { tag: tags.comment, color: '#63636d', fontStyle: 'italic' },
   { tag: tags.punctuation, color: '#a3a3ac' },
   { tag: tags.operator, color: '#a3a3ac' },
]);

const cyanotypeTheme = EditorView.theme(
   {
      '&': {
         backgroundColor: '#000000',
         color: '#f5f5f6',
         fontSize: '12px',
         height: '100%',
      },
      '.cm-content': {
         fontFamily: 'var(--mono)',
         caretColor: '#44cfff',
         padding: '8px 0',
      },
      '.cm-cursor': { borderLeftColor: '#44cfff' },
      '&.cm-focused .cm-selectionBackground, .cm-selectionBackground': {
         background: 'rgba(68, 207, 255, 0.16)',
      },
      '.cm-activeLine': { backgroundColor: 'rgba(255, 255, 255, 0.03)' },
      '.cm-gutters': {
         backgroundColor: '#000000',
         color: '#3d3d45',
         border: 'none',
         borderRight: '1px solid #15151a',
         fontFamily: 'var(--mono)',
         fontSize: '10px',
      },
      '.cm-activeLineGutter': { backgroundColor: 'transparent', color: '#a3a3ac' },
      '.cm-lineNumbers .cm-gutterElement': { padding: '0 8px 0 12px' },
      '.cm-lint-marker': { width: '8px', height: '8px' },
      '.cm-diagnostic': {
         fontFamily: 'var(--mono)',
         fontSize: '11px',
         border: '1px solid #2a2a35',
         background: '#0a0a0c',
      },
      '.cm-diagnostic-error': { borderLeft: '2px solid #f4644a' },
      '.cm-diagnostic-warning': { borderLeft: '2px solid #f5b04a' },
      '.cm-tooltip': { background: '#0a0a0c', border: '1px solid #2a2a35' },
   },
   { dark: true },
);

// ── diagnostics ──────────────────────────────────────────────────────

function renderDiags(diags: DiagJson[]) {
   diagsPane.innerHTML = diags
      .map(
         (d) =>
            `<span class="${d.level === 'error' ? 'err' : d.level === 'warning' ? 'warn' : ''}">${d.formatted}</span>`,
      )
      .join('\n');
   const errs = diags.filter((d) => d.level === 'error').length;
   const warns = diags.filter((d) => d.level === 'warning').length;
   panelMeta.textContent = errs + warns === 0 ? 'NO PROBLEMS' : `${errs} ERR · ${warns} WARN`;
   if (errs > 0) {
      statusState.innerHTML = `<span class="status-pixel err"></span>${errs} ERROR${errs === 1 ? '' : 'S'}`;
   } else {
      statusState.innerHTML = `<span class="status-pixel ok"></span>OK`;
   }
}

// Lint: call wasm check, convert diagnostics to CodeMirror Diagnostic[].
const slabLinter = linter(
   async (v): Promise<Diagnostic[]> => {
      const W = await loadWasm();
      const src = v.state.doc.toString();
      const json = W.check(src, 'playground.slab');
      let diags: DiagJson[];
      try {
         diags = JSON.parse(json) as DiagJson[];
      } catch {
         return [];
      }
      renderDiags(diags);
      return diags.map((d) => {
         // Diag.line is 1-based; doc.line() takes a 1-based line number.
         const lineNo = Math.min(Math.max(1, d.line), v.state.doc.lines);
         const l = v.state.doc.line(lineNo);
         return {
            from: l.from,
            to: l.to,
            severity: d.level === 'error' ? 'error' : d.level === 'warning' ? 'warning' : 'info',
            message: `${d.code}: ${d.msg}${d.remedy ? `\n${d.remedy}` : ''}`,
         } as Diagnostic;
      });
   },
   { delay: 300 },
);

// ── editor ───────────────────────────────────────────────────────────

/** Transactions authored by the design layer (skip canvas-sync echo). */
const fromVibe = Annotation.define<boolean>();

let editor: EditorView;
let srcDoc: SrcDoc | null = null;

function getDoc(): SrcDoc {
   if (!srcDoc) srcDoc = parseSlab(editor.state.doc.toString());
   return srcDoc;
}

function createEditor(initialDoc: string) {
   const parent = document.getElementById('editor') as HTMLDivElement;
   parent.innerHTML = '';
   editor = new EditorView({
      state: EditorState.create({
         doc: initialDoc,
         extensions: [
            lineNumbers(),
            history(),
            drawSelection(),
            highlightActiveLine(),
            keymap.of([...defaultKeymap, ...historyKeymap]),
            lintGutter(),
            slabLinter,
            slab(),
            syntaxHighlighting(cyanotypeHighlight),
            cyanotypeTheme,
            EditorView.lineWrapping,
            EditorView.updateListener.of((update) => {
               if (update.docChanged) {
                  srcDoc = null;
                  scheduleCompile();
                  inspector.refresh();
               } else if (
                  update.selectionSet &&
                  vibe.mode() === 'design' &&
                  !update.transactions.some((tr) => tr.annotation(fromVibe))
               ) {
                  vibe.selectAtPos(update.state.selection.main.head);
               }
            }),
         ],
      }),
      parent,
   });
}

function apply(changes: Change[], hist: boolean): void {
   if (changes.length === 0) return;
   const annotations = [fromVibe.of(true)];
   if (!hist) annotations.push(Transaction.addToHistory.of(false));
   editor.dispatch({ changes, annotations });
}

/** Fold every live (history-suppressed) edit since `text0` into ONE undo
 * entry: revert to text0 outside history, re-apply the net diff inside. */
function commitDrag(text0: string): void {
   const now = editor.state.doc.toString();
   if (now === text0) return;
   // minimal diff: trim common prefix/suffix
   let p = 0;
   const minLen = Math.min(text0.length, now.length);
   while (p < minLen && text0[p] === now[p]) p++;
   let s = 0;
   while (s < minLen - p && text0[text0.length - 1 - s] === now[now.length - 1 - s]) s++;
   const from = p;
   editor.dispatch({
      changes: { from, to: now.length - s, insert: text0.slice(p, text0.length - s) },
      annotations: [fromVibe.of(true), Transaction.addToHistory.of(false)],
   });
   editor.dispatch({
      changes: { from, to: text0.length - s, insert: now.slice(p, now.length - s) },
      annotations: [fromVibe.of(true)],
   });
}

function reveal(node: SrcNode): void {
   const pos = Math.min(node.nameSpan.from, editor.state.doc.length);
   editor.dispatch({
      selection: { anchor: pos },
      scrollIntoView: true,
      annotations: [fromVibe.of(true)],
   });
}
/** Scroll the editor to an absolute text offset (inspector GOTO). */
function revealPos(pos: number): void {
   const anchor = Math.min(pos, editor.state.doc.length);
   editor.dispatch({
      selection: { anchor },
      scrollIntoView: true,
      annotations: [fromVibe.of(true)],
   });
}

// Undo/redo reach the document history from ANYWHERE outside a text field —
// canvas gestures and inspector edits are CodeMirror transactions, so ⌘Z on
// the canvas must unwind them too.
document.addEventListener('keydown', (e) => {
   if (!(e.metaKey || e.ctrlKey)) return;
   const k = e.key.toLowerCase();
   if (k !== 'z' && k !== 'y') return;
   const t = e.target as HTMLElement | null;
   if (t?.closest?.('.cm-editor')) return; // the editor's own keymap owns it
   const typing =
      t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable === true);
   if (typing) return; // native input undo
   e.preventDefault();
   if (k === 'y' || e.shiftKey) redo(editor);
   else undo(editor);
   compileSoon();
});

// ── compile → preview ────────────────────────────────────────────────

let clean = false;
let lastCompiled = '';
const signalsWired = new Set<string>();

const SIGNAL_ATTRS: Record<string, true> = {
   act: true,
   field: true,
   submit: true,
   press: true,
   context: true,
   dblclick: true,
   drag: true,
   drop: true,
   resize: true,
   'pointer-move': true,
   'pointer-up': true,
   'drag-update': true,
   'drag-end': true,
};

/** Discover compiled CustomEvent names from source, including def arguments. */
function sourceSignalNames(doc: SrcDoc): string[] {
   const names = new Set<string>();
   const resolveBinding = (raw: string, bindings: ReadonlyMap<string, string>): string => {
      let value = raw.trim();
      const seen = new Set<string>();
      while (bindings.has(value) && !seen.has(value)) {
         seen.add(value);
         value = bindings.get(value) ?? value;
      }
      return value;
   };
   const add = (
      raw: string,
      bindings: ReadonlyMap<string, string>,
      unresolved: ReadonlySet<string>,
   ): void => {
      const value = resolveBinding(raw, bindings);
      if (unresolved.has(value) || value.startsWith('param.') || value.startsWith('prop.')) {
         return;
      }
      if (value.startsWith('"')) {
         try {
            const decoded = JSON.parse(value) as unknown;
            if (typeof decoded === 'string' && decoded !== '') names.add(decoded);
         } catch {
            // Invalid source never reaches this clean-compile path.
         }
      } else if (/^[A-Za-z_][A-Za-z0-9_-]*$/.test(value)) {
         names.add(value);
      }
   };
   const visit = (
      nodes: readonly SrcNode[],
      bindings: ReadonlyMap<string, string>,
      unresolved: ReadonlySet<string>,
      stack: ReadonlySet<string>,
   ): void => {
      for (const node of nodes) {
         for (const attr of node.attrs) {
            if (SIGNAL_ATTRS[attr.name]) add(attr.value, bindings, unresolved);
         }
         if (node.isCall) {
            const def = doc.defs.get(node.kind);
            if (def && !stack.has(def.name)) {
               const nextBindings = new Map(bindings);
               const nextUnresolved = new Set(unresolved);
               def.fields.forEach((field, index) => {
                  const raw = node.args[index]?.text ?? field.def;
                  if (raw !== '') {
                     nextBindings.set(field.name, resolveBinding(raw, bindings));
                     nextUnresolved.delete(field.name);
                  } else {
                     nextUnresolved.add(field.name);
                  }
               });
               const nextStack = new Set(stack);
               nextStack.add(def.name);
               visit(def.body, nextBindings, nextUnresolved, nextStack);
            }
         }
         visit(node.children, bindings, unresolved, stack);
      }
   };

   visit(doc.roots, new Map(), new Set(), new Set());
   for (const def of doc.defs.values()) {
      visit(def.body, new Map(), new Set(def.params), new Set([def.name]));
   }
   return [...names];
}

async function compile(force = false) {
   const W = await loadWasm();
   const src = editor.state.doc.toString();
   if (!force && src === lastCompiled) return;
   lastCompiled = src;
   const ds = JSON.parse(W.check(src, 'playground.slab')) as DiagJson[];
   renderDiags(ds);
   clean = !ds.some((d) => d.level === 'error');
   if (!clean) {
      vibe.refresh(); // stale overlay state
      return;
   }
   let bytes: Uint8Array;
   try {
      bytes = W.build(src, '{}');
   } catch (e) {
      console.error('slab build failed:', e);
      clean = false;
      vibe.refresh();
      return;
   }
   const sourceDoc = getDoc();
   SlabView.listSchemas = sourceDoc.listSchemas;
   SlabView.listSchemaRows = sourceDoc.listSchemaRows;
   syncStageViewport(sourceDoc);
   if (!view.loadSlir(bytes)) {
      clean = false;
      vibe.refresh();
      return;
   }
   wireSignals(sourceDoc);
   vibe.refresh();
}

/** Signals surface as CustomEvents on the element — log their generic detail. */
function wireSignals(doc: SrcDoc): void {
   for (const name of sourceSignalNames(doc)) {
      if (signalsWired.has(name)) continue;
      signalsWired.add(name);
      view.addEventListener(name, (event) => {
         const span = document.createElement('span');
         span.className = 'sig';
         const detail = (event as CustomEvent<SlabSignalDetail>).detail;
         span.textContent = `\nsignal: ${name} ${JSON.stringify(detail ?? null)}`;
         diagsPane.appendChild(span);
         diagsPane.scrollTop = diagsPane.scrollHeight;
      });
   }
}

// Debounced recompile on editor change; throttled fast path for gestures.
let debounce: ReturnType<typeof setTimeout> | undefined;
function scheduleCompile() {
   clearTimeout(debounce);
   debounce = setTimeout(compile, 300);
}

let liveLast = 0;
let liveTimer: ReturnType<typeof setTimeout> | undefined;
function compileSoon(): void {
   const now = performance.now();
   const wait = Math.max(0, 40 - (now - liveLast));
   clearTimeout(liveTimer);
   liveTimer = setTimeout(() => {
      liveLast = performance.now();
      void compile();
   }, wait);
}

// status line under the canvas: frame dims + scene count. The terminal view
// repaints from the same frames, so motion and interaction land in both.
view.addEventListener('slab-frame', () => {
   const fr = view.lastFrame;
   if (fr) {
      const nodes = view.sceneSnapshot().length;
      statusDims.textContent = `${Math.round(fr.width)}×${Math.round(fr.height)} · ${nodes} NODES`;
   }
   vibe.refresh();
   tui.paint();
});

// ── design mode + inspector ──────────────────────────────────────────

const overlay = document.getElementById('overlay') as HTMLDivElement;
const stageWrap = document.getElementById('stage-wrap') as HTMLDivElement;

interface FixedViewport {
   width: number;
   height: number;
   scale: number;
}

let fixedViewport: FixedViewport | null = null;

function authoredFixedViewport(doc: SrcDoc): FixedViewport | null {
   if (doc.roots.length !== 1) return null;
   const root = doc.roots[0];
   const widthText = root.attrs.find((attr) => attr.name === 'w')?.value.trim() ?? '';
   const heightText = root.attrs.find((attr) => attr.name === 'h')?.value.trim() ?? '';
   const fixedNumber = /^\+?(?:\d+(?:\.\d+)?|\.\d+)$/;
   if (!fixedNumber.test(widthText) || !fixedNumber.test(heightText)) return null;
   const width = Number(widthText);
   const height = Number(heightText);
   return width > 0 && height > 0 ? { width, height, scale: 1 } : null;
}

function updateStageFit(): void {
   const viewport = fixedViewport;
   if (!viewport || stageWrap.clientWidth <= 0 || stageWrap.clientHeight <= 0) return;
   viewport.scale = Math.min(
      1,
      stageWrap.clientWidth / viewport.width,
      stageWrap.clientHeight / viewport.height,
   );
   const left = (stageWrap.clientWidth - viewport.width * viewport.scale) / 2;
   const top = (stageWrap.clientHeight - viewport.height * viewport.scale) / 2;
   stageWrap.style.setProperty('--stage-width', `${viewport.width}px`);
   stageWrap.style.setProperty('--stage-height', `${viewport.height}px`);
   stageWrap.style.setProperty('--stage-scale', String(viewport.scale));
   stageWrap.style.setProperty('--stage-left', `${left}px`);
   stageWrap.style.setProperty('--stage-top', `${top}px`);
}

function syncStageViewport(doc: SrcDoc): void {
   fixedViewport = authoredFixedViewport(doc);
   stageWrap.classList.toggle('fixed-frame', fixedViewport !== null);
   if (fixedViewport) {
      updateStageFit();
      return;
   }
   for (const property of [
      '--stage-width',
      '--stage-height',
      '--stage-scale',
      '--stage-left',
      '--stage-top',
   ]) {
      stageWrap.style.removeProperty(property);
   }
}

new ResizeObserver(updateStageFit).observe(stageWrap);

/** SlabElement receives real captured pointer events in interact mode. Its
 * public driver currently reads CSS pixels, so shadow event coordinates and
 * wheel deltas into the fixed document's logical coordinate space first. */
function remapFixedViewportEvent(event: Event): void {
   const viewport = fixedViewport;
   if (
      !viewport ||
      viewport.scale === 1 ||
      !(event instanceof MouseEvent) ||
      !event.composedPath().includes(view)
   ) {
      return;
   }
   const rect = view.getBoundingClientRect();
   Object.defineProperties(event, {
      clientX: {
         configurable: true,
         value: rect.left + (event.clientX - rect.left) / viewport.scale,
      },
      clientY: {
         configurable: true,
         value: rect.top + (event.clientY - rect.top) / viewport.scale,
      },
   });
   if (event instanceof WheelEvent) {
      Object.defineProperties(event, {
         deltaX: { configurable: true, value: event.deltaX / viewport.scale },
         deltaY: { configurable: true, value: event.deltaY / viewport.scale },
      });
   }
}

for (const eventName of ['pointerdown', 'pointermove', 'pointerup', 'wheel']) {
   stageWrap.addEventListener(eventName, remapFixedViewportEvent, { capture: true });
}

const vibe: Vibe = createVibe({
   view,
   overlay,
   doc: getDoc,
   clean: () => clean,
   text: () => editor.state.doc.toString(),
   apply,
   commitDrag,
   reveal,
   compileSoon,
   onSelect(sel) {
      inspector.render(sel);
      statusSel.textContent = sel ? sel.key : '';
      if (sel && document.body.classList.contains('inspect-collapsed')) setInspector(true);
   },
});

const inspector = createInspector({
   body: document.getElementById('inspect-body') as HTMLDivElement,
   meta: document.getElementById('inspect-meta') as HTMLSpanElement,
   crumbs: document.getElementById('crumbs') as HTMLDivElement,
   view,
   vibe,
   doc: getDoc,
   text: () => editor.state.doc.toString(),
   apply,
   commitDrag,
   compileSoon,
   revealPos,
});

const tui: Tui = createTui({
   surface: document.getElementById('tui') as HTMLDivElement,
   meta: document.getElementById('preview-meta') as HTMLSpanElement,
   view,
});

/** Light up the clicked button of a segmented control. */
function segSelect(seg: HTMLDivElement, on: HTMLButtonElement): void {
   for (const b of seg.querySelectorAll('button')) b.classList.toggle('on', b === on);
}

// Surface and interaction mode are independent: the terminal paints over the
// stage but passes input through, so TUI + INTERACT drives the live document
// through cells, and TUI + DESIGN edits source with the overlay on top.
const viewSeg = document.getElementById('view-seg') as HTMLDivElement;
viewSeg.addEventListener('click', (e) => {
   const btn = (e.target as HTMLElement).closest('button');
   if (!btn) return;
   segSelect(viewSeg, btn);
   tui.setActive(btn.dataset.view === 'tui');
});

const modeSeg = document.getElementById('mode-seg') as HTMLDivElement;
modeSeg.addEventListener('click', (e) => {
   const btn = (e.target as HTMLElement).closest('button');
   if (!btn) return;
   segSelect(modeSeg, btn);
   const m = btn.dataset.mode === 'interact' ? 'interact' : 'design';
   vibe.setMode(m);
   stageWrap.classList.toggle('interact', m === 'interact');
});

// ── ADD palette ──────────────────────────────────────────────────────

const addBtn = document.getElementById('add-btn') as HTMLButtonElement;
const addMenu = document.getElementById('add-menu') as HTMLDivElement;

const BASIC_SNIPPETS: [string, string][] = [
   ['rect', 'rect w=120 h=64 bg=#1A1A20 stroke=#2A2A35'],
   ['text', 'text "Text" size=13'],
   ['spacer', 'spacer'],
   ['path', 'path "M0 0 L48 24 L0 48 Z" bg=#44CFFF'],
];

const CONTAINER_SNIPPETS: [string, string][] = [
   ['row', 'row gap=8'],
   ['col', 'col gap=8'],
   ['stack', 'stack'],
   ['canvas', 'canvas w=240 h=160'],
   ['grid', 'grid cols=fill,fill gap=8'],
   ['wrap', 'wrap gap=8'],
   ['para', 'para size=13 { "Paragraph text" }'],
   ['group', 'group'],
];

function menuSection(title: string): HTMLDivElement {
   const s = document.createElement('div');
   s.className = 'menu-sec';
   s.textContent = title;
   addMenu.appendChild(s);
   return s;
}

function menuItem(label: string, hint: string, snippet: string): void {
   const b = document.createElement('button');
   b.type = 'button';
   b.className = 'menu-item';
   b.setAttribute('role', 'menuitem');
   const l = document.createElement('span');
   l.textContent = label.toUpperCase();
   const h = document.createElement('span');
   h.className = 'menu-hint';
   h.textContent = hint;
   b.append(l, h);
   b.addEventListener('click', () => {
      addMenu.hidden = true;
      vibe.insertSnippet(snippet);
   });
   addMenu.appendChild(b);
}

function paletteInsertionParent(doc: SrcDoc): SrcNode | null {
   const selected = vibe.selection()?.target?.node ?? null;
   if (selected && CONTAINERS[selected.kind] && !selected.isCall) return selected;
   if (selected?.parent) return selected.parent;
   return doc.roots.length === 1 && CONTAINERS[doc.roots[0].kind] ? doc.roots[0] : null;
}

function dividerParentForPalette(doc: SrcDoc): SrcNode | null {
   const selected = vibe.selection()?.target?.node ?? null;
   if (
      selected &&
      (selected.kind === 'row' || selected.kind === 'col') &&
      selected.children.length >= 2
   ) {
      return selected;
   }
   if (
      selected?.parent &&
      (selected.parent.kind === 'row' || selected.parent.kind === 'col') &&
      selected.parent.children.length >= 2
   ) {
      return selected.parent;
   }
   const root = doc.roots.length === 1 ? doc.roots[0] : null;
   return root && (root.kind === 'row' || root.kind === 'col') && root.children.length >= 2
      ? root
      : null;
}

function openAddMenu(): void {
   addMenu.replaceChildren();
   const doc = getDoc();
   const insertionParent = paletteInsertionParent(doc);
   menuSection('BASICS');
   for (const [kind, snippet] of BASIC_SNIPPETS) {
      if (kind === 'path' && insertionParent?.kind !== 'canvas') continue;
      menuItem(kind, '', snippet);
   }
   menuSection('CONTAINERS');
   for (const [kind, snippet] of CONTAINER_SNIPPETS) menuItem(kind, '', snippet);
   const schemaFitsParent = (schemaName: string): boolean => {
      if (insertionParent?.kind !== 'para') return true;
      const body = doc.defs.get(schemaName)?.body;
      return body !== undefined && body.length === 1 && body[0].kind === 'span';
   };

   const rootLists = doc.params.filter((param) => {
      if (!/^list\s*\(/.test(param.type)) return false;
      const schema = doc.listSchemas[param.name];
      return schema !== undefined && schemaFitsParent(schema.name);
   });
   const selectedDef = vibe.selection()?.target?.def;
   const selectedDefIsList =
      selectedDef !== null &&
      selectedDef !== undefined &&
      doc.listSchemaRows.some((schema) => schema.name === selectedDef);
   const nestedLists = selectedDefIsList
      ? (doc.defs.get(selectedDef)?.fields.filter((field) => {
           const schemaName = /^list\s*\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*\)$/.exec(field.def)?.[1];
           return schemaName !== undefined && schemaFitsParent(schemaName);
        }) ?? [])
      : [];
   if (rootLists.length > 0 || nestedLists.length > 0) {
      menuSection('DATA');
      for (const param of rootLists) {
         menuItem(`each ${param.name}`, param.type, `each param.${param.name}`);
      }
      for (const field of nestedLists) {
         menuItem(`each ${field.name}`, field.def, `each prop.${field.name}`);
      }
   }

   if (doc.icons.length > 0) {
      menuSection('ICONS');
      for (const icon of doc.icons) {
         menuItem(icon.name, 'declared icon', `icon ${icon.name} size=24`);
      }
   }

   const dividerParent = dividerParentForPalette(doc);
   if (dividerParent) {
      menuSection('STRUCTURE');
      const snippet = dividerParent.kind === 'row' ? 'divider w=4 h=fill' : 'divider h=4 w=fill';
      menuItem('divider', 'between children', snippet);
   }

   const defs = [...doc.defs.values()];
   if (defs.length > 0) {
      menuSection('COMPONENTS');
      for (const def of defs) menuItem(def.name, def.params.join(', '), def.name);
   }
   menuSection('RUNTIME');
   const uploadBtn = document.createElement('label');
   uploadBtn.className = 'menu-item';
   uploadBtn.setAttribute('role', 'menuitem');
   uploadBtn.innerHTML = `<span>IMAGE (UPLOAD)</span><span class="menu-hint">runtime</span>`;
   const fileIn = document.createElement('input');
   fileIn.type = 'file';
   fileIn.accept = 'image/png, image/jpeg, image/webp';
   fileIn.style.display = 'none';
   fileIn.addEventListener('change', async () => {
      const file = fileIn.files?.[0];
      if (!file) return;
      addMenu.hidden = true;
      const bitmap = await createImageBitmap(file);
      try {
         const canvas = document.createElement('canvas');
         canvas.width = bitmap.width;
         canvas.height = bitmap.height;
         const ctx = canvas.getContext('2d');
         if (!ctx) return;
         ctx.drawImage(bitmap, 0, 0);
         const data = ctx.getImageData(0, 0, bitmap.width, bitmap.height);
         const bytes = new Uint8Array(data.data.buffer, data.data.byteOffset, data.data.byteLength);
         const name = file.name.replace(/[^a-zA-Z0-9_-]/g, '_');
         const image = view.imgRegister(name, bitmap.width, bitmap.height, 1, bytes);
         if (image < 0) {
            console.error(`slab: runtime image registration failed for ${JSON.stringify(name)}`);
            return;
         }
         const width = Math.min(bitmap.width, 320);
         const height = Math.min(bitmap.height, 320);
         vibe.insertSnippet(`img src="${name}" w=${width} h=${height} fit=contain`);
      } finally {
         bitmap.close();
      }
   });
   uploadBtn.appendChild(fileIn);
   addMenu.appendChild(uploadBtn);
   addMenu.hidden = false;
}

addBtn.addEventListener('click', (e) => {
   e.stopPropagation();
   if (addMenu.hidden) openAddMenu();
   else addMenu.hidden = true;
});
document.addEventListener('click', (e) => {
   if (!addMenu.hidden && !addMenu.contains(e.target as Node)) addMenu.hidden = true;
});
document.addEventListener('keydown', (e) => {
   if (e.key === 'Escape' && !addMenu.hidden) addMenu.hidden = true;
});

// ── splitters (VSCode-style: magnetic, double-click reset) ───────────

const splitEl = document.getElementById('split') as HTMLDivElement;
const gutterV = document.getElementById('gutter-v') as HTMLDivElement;
const gutterH = document.getElementById('gutter-h') as HTMLDivElement;
const panel = document.getElementById('panel') as HTMLDivElement;
const panelToggle = document.getElementById('panel-toggle') as HTMLButtonElement;
const inspectToggle = document.getElementById('inspect-toggle') as HTMLButtonElement;

/** Vertical splitter: editor/preview ratio with a magnetic center snap. */
gutterV.addEventListener('pointerdown', (down) => {
   down.preventDefault();
   gutterV.setPointerCapture(down.pointerId);
   gutterV.classList.add('active');
   document.body.classList.add('dragging', 'dragging-col');
   const move = (e: PointerEvent) => {
      const rect = splitEl.getBoundingClientRect();
      const inspectW = document.body.classList.contains('inspect-collapsed')
         ? 0
         : (document.getElementById('inspect-pane')?.getBoundingClientRect().width ?? 0);
      const usable = rect.width - inspectW - gutterV.offsetWidth;
      let frac = (e.clientX - rect.left) / usable;
      const mid = 0.5;
      if (Math.abs(frac - mid) < 0.04) {
         frac = mid;
         gutterV.classList.add('snapped');
      } else {
         gutterV.classList.remove('snapped');
      }
      frac = Math.min(0.85, Math.max(0.15, frac));
      splitEl.style.setProperty('--split', `${frac * usable}px`);
   };
   const up = () => {
      gutterV.classList.remove('active', 'snapped');
      document.body.classList.remove('dragging', 'dragging-col');
      gutterV.removeEventListener('pointermove', move);
      gutterV.removeEventListener('pointerup', up);
   };
   gutterV.addEventListener('pointermove', move);
   gutterV.addEventListener('pointerup', up);
});

gutterV.addEventListener('dblclick', () => {
   splitEl.style.removeProperty('--split');
});

/** Horizontal splitter: problems panel height; collapses under 48px. */
gutterH.addEventListener('pointerdown', (down) => {
   down.preventDefault();
   gutterH.setPointerCapture(down.pointerId);
   gutterH.classList.add('active');
   document.body.classList.add('dragging', 'dragging-row');
   const move = (e: PointerEvent) => {
      const rect = panel.parentElement?.getBoundingClientRect();
      if (!rect) return;
      const h = rect.bottom - e.clientY - 24; // status bar
      if (h < 48) {
         setPanel(false);
      } else {
         setPanel(true);
         panel.style.height = `${Math.min(h, rect.height - 120)}px`;
      }
   };
   const up = () => {
      gutterH.classList.remove('active');
      document.body.classList.remove('dragging', 'dragging-row');
      gutterH.removeEventListener('pointermove', move);
      gutterH.removeEventListener('pointerup', up);
   };
   gutterH.addEventListener('pointermove', move);
   gutterH.addEventListener('pointerup', up);
});

gutterH.addEventListener('dblclick', () => {
   panel.style.height = '140px';
});

function setPanel(open: boolean) {
   document.body.classList.toggle('panel-collapsed', !open);
   panelToggle.classList.toggle('on', open);
   if (open && panel.getBoundingClientRect().height < 48) {
      panel.style.height = '140px';
   }
}

panelToggle.addEventListener('click', () => {
   setPanel(document.body.classList.contains('panel-collapsed'));
});
setPanel(true);

function setInspector(open: boolean) {
   document.body.classList.toggle('inspect-collapsed', !open);
   inspectToggle.classList.toggle('on', open);
}

inspectToggle.addEventListener('click', () => {
   setInspector(document.body.classList.contains('inspect-collapsed'));
});
setInspector(true);

// ── examples ─────────────────────────────────────────────────────────

async function loadExamples() {
   const res = await fetch('./examples/manifest.json');
   const names = (await res.json()) as string[];
   for (const n of names) {
      const opt = document.createElement('option');
      opt.value = n;
      opt.textContent = n;
      select.appendChild(opt);
   }
   select.addEventListener('change', async () => {
      const r = await fetch(`./examples/${select.value}`);
      const text = await r.text();
      vibe.select(null);
      editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: text } });
      scheduleCompile();
   });
   const def = names.find((n) => n === '09-widget.slab') ?? names[0];
   if (def) {
      select.value = def;
      const r = await fetch(`./examples/${def}`);
      createEditor(await r.text());
      inspector.render(null);
      setTimeout(compile, 100);
   }
}

loadExamples();
