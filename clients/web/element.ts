// @stencil-hq/wslab SlabElement: the custom-element base every `slab gen wc`
// extends. The WASM kernel drives layout, hit testing, focus, editing, motion,
// and scrolling; this class translates platform events, paints decoded frames,
// surfaces effects, wires environment inputs, and mounts named holes.
//
// Frame loop discipline: request a frame only after an input or while the
// decoded kernel frame reports dirty state or active motion.

import { type FontMetrics, parseFontMetrics } from './fontmetrics.ts';
import { decodeFrame } from './frame-decode.ts';
import type {
   EachWindow,
   Effects,
   Frame,
   HoleRect,
   ImageInfo,
   LiftedAnimation,
   ParamValue,
   SceneNode,
   SigMeta,
   Statics,
} from './kernel.ts';
import { type FontCss, fontRulesCss, liftedAnimationCss, Painter } from './painter.ts';
import init, { KInst } from './wasm/slab_kernel.js';

/** Shared metadata carried by every named Slab signal. */
export type SignalMeta = SigMeta;

/** Detail carried by every named Slab signal CustomEvent. */
export interface SlabSignalDetail {
   /** Innermost list item key, or empty outside a list. */
   readonly item: string;
   /** Pointer/modifier, emitter-key, and drag-source metadata. */
   readonly meta: SignalMeta;
   /** Change/Submit text or a Divider's final resize extent. */
   readonly text?: string;
}

await init({ module_or_path: new URL('./wasm/slab_kernel_bg.wasm', import.meta.url) });

const BASE_CSS = `
:host { display: block; position: relative; contain: layout style; }
:host(:focus) { outline: none; }
.slab-ops, .slab-holes { position: absolute; left: 0; top: 0; width: 100%; height: 100%; pointer-events: none; }
.slab-ops * { position: absolute; box-sizing: border-box; margin: 0; padding: 0; }
.slab-ops div { left: 0; top: 0; width: 0; height: 0; }
.slab-ops svg { overflow: visible; }
.slab-ops span {
   font: 400 16px/1 sans-serif;
   font-kerning: none; font-variant-ligatures: none;
   letter-spacing: 0; white-space: pre;
}
.slab-hole { position: absolute; pointer-events: auto; }
.slab-hole { scrollbar-width: thin; scrollbar-color: color-mix(in srgb, currentColor 25%, transparent) transparent; }
.slab-a11y { position: absolute; left: 0; top: 0; width: 100%; height: 100%; pointer-events: none; overflow: visible; }
.slab-a11y-node { position: absolute; box-sizing: border-box; margin: 0; padding: 0; border: 0; outline: 0; pointer-events: none; color: transparent; background: transparent; }
.slab-caret {
   position: absolute; pointer-events: none;
   background: var(--slab-caret, #7dd3e8);
   animation: slab-caret-blink 1s steps(1) infinite;
}
@keyframes slab-caret-blink { 50% { opacity: 0; } }
.slab-ime {
   position: absolute; width: 1px; height: 16px; opacity: 0;
   border: 0; padding: 0; margin: 0; background: transparent;
   color: transparent; caret-color: transparent; outline: none;
   pointer-events: none; resize: none; overflow: hidden;
}
`;

let baseSheet: CSSStyleSheet | null = null;

function sheet(): CSSStyleSheet {
   if (!baseSheet) {
      baseSheet = new CSSStyleSheet();
      baseSheet.replaceSync(BASE_CSS);
   }
   return baseSheet;
}

function decodeBase64(s: string): Uint8Array {
   const bin = atob(s);
   const out = new Uint8Array(bin.length);
   for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
   return out;
}

const E_POINTER_MOVE = 0;
const E_POINTER_DOWN = 1;
const E_POINTER_UP = 2;
const E_WHEEL = 3;
const E_KEY_DOWN = 4;
const E_TEXT = 5;
const E_PASTE = 6;
const E_COPY = 7;
const E_CUT = 8;
const E_COMPOSITION_START = 9;
const E_COMPOSITION_UPDATE = 10;
const E_COMPOSITION_END = 11;
const E_BLUR = 12;
const E_CLOSE = 14;

const M_SHIFT = 1;
const M_ALT = 2;
const M_CTRL = 4;
const M_META = 8;

function modsOf(e: {
   shiftKey: boolean;
   altKey: boolean;
   ctrlKey: boolean;
   metaKey: boolean;
}): number {
   let modifiers = 0;
   if (e.shiftKey) modifiers |= M_SHIFT;
   if (e.altKey) modifiers |= M_ALT;
   if (e.ctrlKey) modifiers |= M_CTRL;
   if (e.metaKey) modifiers |= M_META;
   return modifiers;
}

/** '#rgb' | '#rgba' | '#rrggbb' | '#rrggbbaa' → SLIR rgba8 (r in the low byte). */
export function parseColor(s: string): number | null {
   const h = s.startsWith('#') ? s.slice(1) : s;
   if (!/^[0-9a-fA-F]+$/.test(h)) return null;
   let r: number;
   let g: number;
   let b: number;
   let a = 255;
   if (h.length === 3 || h.length === 4) {
      r = Number.parseInt(h[0] + h[0], 16);
      g = Number.parseInt(h[1] + h[1], 16);
      b = Number.parseInt(h[2] + h[2], 16);
      if (h.length === 4) a = Number.parseInt(h[3] + h[3], 16);
   } else if (h.length === 6 || h.length === 8) {
      r = Number.parseInt(h.slice(0, 2), 16);
      g = Number.parseInt(h.slice(2, 4), 16);
      b = Number.parseInt(h.slice(4, 6), 16);
      if (h.length === 8) a = Number.parseInt(h.slice(6, 8), 16);
   } else {
      return null;
   }
   return (r | (g << 8) | (b << 16) | (a << 24)) >>> 0;
}

/** Coerce a JS value to a kernel parameter payload for the declared type. */
export function coerceParam(kind: number, value: unknown): ParamValue | null {
   const param: ParamValue = { kind, num: 0, s: '', rgba: 0, sym: '' };
   switch (kind) {
      case 0:
         param.s = String(value ?? '');
         return param;
      case 1:
      case 2: {
         const raw = typeof value === 'string' && kind === 2 ? value.replace(/%$/, '') : value;
         const number = Number(raw);
         if (Number.isNaN(number)) return null;
         param.num = number;
         return param;
      }
      case 3: {
         if (typeof value === 'number') {
            param.rgba = value >>> 0;
            return param;
         }
         const rgba = parseColor(String(value ?? ''));
         if (rgba === null) return null;
         param.rgba = rgba;
         return param;
      }
      case 4:
         param.num =
            value === false || value === 0 || value === 'false' || value === '0' || value == null
               ? 0
               : 1;
         return param;
      case 5:
         param.sym = String(value ?? '');
         return param;
      default:
         return null;
   }
}

function isPlainRecord(v: unknown): v is Record<string, unknown> {
   if (v === null || typeof v !== 'object' || Array.isArray(v)) return false;
   const proto = Object.getPrototypeOf(v);
   return proto === Object.prototype || proto === null;
}

export interface ListFieldSchema {
   name: string;
   type: number;
   enum?: readonly string[];
   /** Zero for scalar fields, otherwise one plus the nested schema row. */
   sub?: number;
}

export interface ListRowSchema {
   fields: readonly ListFieldSchema[];
}

export interface ListSchema extends ListRowSchema {
   param: number;
   /** LIST-table row used to validate path-addressed nested writes. */
   row?: number;
}
interface NormalizedListWrite {
   row: number;
   schema: ListRowSchema;
   path: string;
   items: Record<string, unknown>[];
   keys: string[];
   values: (ParamValue | null)[][];
   children: NormalizedListWrite[];
}

export interface SlabDebugEntry {
   /** Scene geometry of the last painted frame (component-local coordinates). */
   geom(): { key: string; node: number; x: number; y: number; w: number; h: number }[];
   frame(): { width: number; height: number; ops: number };
}

const kBytes = Symbol('slab.bytes');
const kFonts = Symbol('slab.fonts');
const kImages = Symbol('slab.images');

type CachedSlabConstructor = typeof SlabElement & {
   [kBytes]?: Promise<Uint8Array>;
   [kFonts]?: (FontCss | null)[];
   [kImages]?: (string | null)[];
};

interface RegisteredFace {
   family: string;
   bytes: Uint8Array;
   metrics: FontMetrics;
   cssFamily: string;
}

interface RuntimeImageRegistration {
   image: number;
   width: number;
   height: number;
   format: number;
   bytes: Uint8Array;
}

// Page-level registrations are keyed by the authored family case-insensitively.
const registeredFaces = new Map<string, RegisteredFace>();
const mountedElements = new Set<SlabElement>();
let fontSeq = 0;
const CURSORS: readonly string[] = ['', 'pointer', 'text', 'col-resize', 'row-resize'];
const F_FOCUSABLE = 1 << 6;
const F_INERT = 1 << 5;
const sceneKeyEncoder = new TextEncoder();

/** Encodes every UTF-8 byte so stable scene identity, never synthetic node id, owns DOM identity. */
function semanticNodeId(key: string, occurrence = 0): string {
   let encoded = '';
   for (const byte of sceneKeyEncoder.encode(key)) encoded += byte.toString(16).padStart(2, '0');
   const duplicate = occurrence === 0 ? '' : `--duplicate-${occurrence}`;
   return `slab-a11y-${encoded || '00'}${duplicate}`;
}

/** Unambiguous reconciliation key for one occurrence of a potentially duplicated scene key. */
function semanticSceneIdentity(key: string, occurrence: number): string {
   return `${key.length}:${key}:${occurrence}`;
}

/** Applies this scene entry's local rotation; semantic DOM ancestors compose theirs naturally. */
function setSemanticTransform(element: HTMLElement, node: SceneNode): void {
   if (node.rotation === 0) {
      element.style.removeProperty('transform');
      element.style.removeProperty('transform-origin');
      return;
   }
   element.style.transformOrigin = `${node.cx - node.x}px ${node.cy - node.y}px`;
   element.style.transform = `rotate(${node.rotation}deg)`;
}

function setOptionalAttribute(
   element: HTMLElement,
   name: string,
   value: string | number | boolean | null,
): void {
   if (value === null || value === '') {
      element.removeAttribute(name);
      return;
   }
   element.setAttribute(name, String(value));
}

function fontKey(name: string): string {
   let key = '';
   for (let i = 0; i < name.length; i++) {
      const code = name.charCodeAt(i);
      key += code >= 65 && code <= 90 ? String.fromCharCode(code + 32) : name[i];
   }
   return key;
}

function isSlabConstructor(value: unknown): value is CachedSlabConstructor {
   if (typeof value !== 'function') return false;
   return value === SlabElement || value.prototype instanceof SlabElement;
}

function slabConstructor(element: SlabElement): CachedSlabConstructor {
   return isSlabConstructor(element.constructor) ? element.constructor : SlabElement;
}

interface KernelEvent {
   eventType: number;
   x: number;
   y: number;
   dx: number;
   dy: number;
   button: number;
   clicks: number;
   key: string;
   text: string;
   modifiers: number;
}

function kernelEvent(eventType: number, init: Partial<KernelEvent> = {}): KernelEvent {
   return {
      eventType,
      x: init.x ?? 0,
      y: init.y ?? 0,
      dx: init.dx ?? 0,
      dy: init.dy ?? 0,
      button: init.button ?? 0,
      clicks: init.clicks ?? 0,
      key: init.key ?? '',
      text: init.text ?? '',
      modifiers: init.modifiers ?? 0,
   };
}

function emptyEffects(): Effects {
   return {
      repaint: false,
      sig_name: [],
      sig_text: [],
      sig_item: [],
      sig_meta: [],
      scrolls: [],
      has_caret: false,
      caret_x: 0,
      caret_y: 0,
      caret_w: 0,
      caret_h: 0,
      has_ime: false,
      ime_x: 0,
      ime_y: 0,
      ime_w: 0,
      ime_h: 0,
      cursor: 0,
      focus: 0xffffffff,
   };
}

export class SlabElement extends HTMLElement {
   /** Set (or define `globalThis.__SLAB_DEBUG__`) before elements connect to
    * expose per-element scene geometry on `globalThis.__slabDebug`. */
   static debug = false;
   /** Clear before elements connect to keep every animation on the kernel
    * clock instead of replaying eligible ones as CSS. Frames then cost a
    * paint each, and every host reading the kernel sees the motion. */
   static lift = true;
   /** Generated subclasses embed SLIR as base64 here (or a URL with slirIsUrl). */
   static slir: string | Uint8Array = '';
   static slirIsUrl = false;
   /** Generated subclasses describe list element fields here. */
   static listSchemas: Readonly<Record<string, ListSchema>> = {};
   /** Generated schema rows referenced by nested list fields. */
   static listSchemaRows: readonly ListRowSchema[] = [];

   #inst: KInst | null = null;
   #statics: Statics | null = null;
   #painter: Painter | null = null;
   #lastFrame: Frame | null = null;
   #scene: readonly SceneNode[] | null = null;

   #ops: HTMLDivElement;
   #holesLayer: HTMLDivElement;
   #a11yLayer: HTMLDivElement;
   #a11yNodes = new Map<string, HTMLDivElement>();
   #holeEls: HTMLDivElement[] = [];
   #caret: HTMLDivElement;
   #ime: HTMLTextAreaElement;
   #editFocus = 0xffffffff;

   #raf = 0;
   #inited = false;
   #envReady = false;
   #selfSized = false;
   #vw = 0;
   #vh = 0;
   #suppressInput = false;
   #ownImageUrls: string[] = [];
   #appliedFonts = new Set<string>();
   #ro: ResizeObserver | null = null;
   #holeRo: ResizeObserver | null = null;
   #mqDark = matchMedia('(prefers-color-scheme: dark)');
   #mqCoarse = matchMedia('(pointer: coarse)');

   #pendingAttrs = new Map<string, string | null>();
   #props = new Map<string, unknown>();
   #lists = new Map<string, Map<string, readonly Record<string, unknown>[]>>();
   #scrolls = new Map<string, Map<number, number>>();
   #runtimeImages = new Map<string, RuntimeImageRegistration>();
   #dividers = new Map<string, number>();
   #pendingFocus: [string, boolean] | null = null;
   #theme = '';
   // Per-element sheet holding lifted @keyframes (names are binding-scoped).
   #animSheet = new CSSStyleSheet();
   // Per-element sheet mapping SLIR FONT indices to `.f<i>` family classes.
   #fontSheet = new CSSStyleSheet();

   constructor() {
      super();
      const root = this.attachShadow({ mode: 'open', delegatesFocus: true });
      root.adoptedStyleSheets = [sheet(), this.#fontSheet, this.#animSheet];
      this.#ime = document.createElement('textarea');
      this.#ime.className = 'slab-ime';
      this.#ime.setAttribute('aria-hidden', 'true');
      this.#ime.tabIndex = -1;
      this.#ops = document.createElement('div');
      this.#ops.className = 'slab-ops';
      this.#ops.setAttribute('aria-hidden', 'true');
      this.#a11yLayer = document.createElement('div');
      this.#a11yLayer.className = 'slab-a11y';
      this.#a11yLayer.addEventListener('focusin', (event) => {
         const target = event.target;
         if (target instanceof HTMLElement && target.dataset.slabKey) {
            this.setFocus(target.dataset.slabKey, true);
         }
      });
      this.#a11yLayer.addEventListener('beforeinput', (event) => {
         const target = event.target;
         if (
            !(event instanceof InputEvent) ||
            !(target instanceof HTMLElement) ||
            target.dataset.slabEditor !== 'true'
         ) {
            return;
         }
         // The semantic control remains the AT focus target and acts as the
         // browser's editable proxy; kernel edit state remains authoritative.
         event.preventDefault();
         if (!event.isComposing && event.inputType.startsWith('insert') && event.data) {
            this.#dispatch(kernelEvent(E_TEXT, { text: event.data }));
         }
      });
      this.#a11yLayer.addEventListener('click', (event) => {
         const target = event.target;
         if (!(target instanceof HTMLElement) || !target.dataset.slabKey) return;
         event.preventDefault();
         event.stopPropagation();
         if (!this.setFocus(target.dataset.slabKey, true)) return;
         // Reuse the kernel's ordinary pointer activation path. In particular,
         // never turn a semantic click on an editable field into Enter/submit.
         const rect = target.getBoundingClientRect();
         const hostRect = this.getBoundingClientRect();
         const x = rect.left + rect.width / 2 - hostRect.left;
         const y = rect.top + rect.height / 2 - hostRect.top;
         this.#dispatch(kernelEvent(E_POINTER_DOWN, { x, y, button: 0, clicks: 1 }));
         this.#dispatch(kernelEvent(E_POINTER_UP, { x, y, button: 0, clicks: 1 }));
      });
      this.#holesLayer = document.createElement('div');
      this.#holesLayer.className = 'slab-holes';
      this.#caret = document.createElement('div');
      this.#caret.className = 'slab-caret';
      this.#caret.hidden = true;
      root.append(this.#ime, this.#a11yLayer, this.#ops, this.#holesLayer, this.#caret);
      this.#listen();
   }

   // ------------------------------------------------------------- lifecycle

   connectedCallback(): void {
      mountedElements.add(this);
      if (this.#inited) {
         this.#ro?.observe(this);
         this.#measure();
         this.#schedule();
         return;
      }
      void this.#init();
   }

   disconnectedCallback(): void {
      this.#endInstance(E_BLUR);
      mountedElements.delete(this);
      this.#ro?.disconnect();
      if (this.#raf !== 0) {
         cancelAnimationFrame(this.#raf);
         this.#raf = 0;
      }
   }
   // ------------------------------------------------------------ live editing

   /** Live kernel instance — the geometry/hit-test surface editor tooling
    * (e.g. the playground's design mode) reads. Null until SLIR is mounted. */
   get instance(): KInst | null {
      return this.#inst;
   }

   /** The most recently painted frame (null before the first paint). */
   get lastFrame(): Frame | null {
      return this.#lastFrame;
   }

   /** Mount or hot-swap SLIR bytes on a live element (live-preview hosts
    * compile-on-edit and swap in place). Params, lists, scroll/divider offsets,
    * runtime images, and env survive; focus and edit state reset.
    * Returns false when the bytes fail to decode (previous content is
    * already torn down). Fires `slab-frame` after every subsequent paint. */
   loadSlir(bytes: Uint8Array): boolean {
      this.#inited = true; // supersedes any pending static-slir boot
      this.#teardown();
      if (!this.#mount(bytes, false)) return false;
      this.#wire();
      this.#envReady = false; // force env onto the fresh instance
      this.#measure();
      this.#registerDebug();
      return true;
   }

   async #init(): Promise<void> {
      if (this.#inited) return;
      this.#inited = true;
      const cls = slabConstructor(this);
      const bytes = await SlabElement.#loadSlir(cls);
      // Subclasses without embedded SLIR (live-preview hosts) mount later
      // through loadSlir().
      if (bytes.length === 0) return;
      if (!this.#mount(bytes, true)) return;
      this.#wire();
      this.#measure();
      this.#registerDebug();
   }

   /** Decode SLIR and build painter + holes; re-applies buffered attrs, props,
    * lists, scroll/divider offsets, and runtime images. `cached` uses the
    * per-class font/image caches; live swaps derive per-mount and revoke
    * their object URLs on the next swap. */
   #mount(bytes: Uint8Array, cached: boolean): boolean {
      let inst: KInst | null = null;
      let statics: Statics | null = null;
      try {
         inst = new KInst(bytes);
         statics = JSON.parse(inst.statics_json());
      } catch (error) {
         inst?.free();
         console.error(
            'slab: SLIR decode failed:',
            error instanceof Error ? error.message : String(error),
         );
         return false;
      }
      if (!inst || !statics) return false;
      this.#inst = inst;
      this.#statics = statics;
      const painter = new Painter(statics, this.#ops);
      painter.imageInfo = (image) => this.imgInfo(image);
      painter.imageBytes = (image) => this.imgBytes(image);
      painter.invalidate = () => this.#schedule();
      if (cached) {
         const cls = slabConstructor(this);
         painter.fonts = SlabElement.#fonts(cls, statics);
         painter.imageUrls = SlabElement.#images(cls, statics, inst);
      } else {
         painter.fonts = SlabElement.#fontsFor(statics);
         const urls = SlabElement.#imagesFor(statics, inst);
         this.#ownImageUrls = urls.filter((url): url is string => url !== null);
         painter.imageUrls = urls;
      }
      this.#fontSheet.replaceSync(fontRulesCss(painter.fonts));
      this.#painter = painter;
      this.#liftAnimations(inst, painter);
      this.#appliedFonts.clear();
      for (const [key, face] of registeredFaces) this.#applyFont(key, face);
      this.#makeHoles();
      this.setTheme(this.#theme);
      const listReplay = [...this.#lists].flatMap(([name, paths]) =>
         [...paths].map(([path, items]) => ({ name, path, items })),
      );
      listReplay.sort(
         (left, right) =>
            (left.path === '' ? 0 : left.path.split('.').length) -
            (right.path === '' ? 0 : right.path.split('.').length),
      );
      for (const [attr, value] of this.#pendingAttrs) this.#applyAttr(attr, value);
      this.#pendingAttrs.clear();
      for (const [name, value] of this.#props) this.setParam(name, value);
      for (const { name, path, items } of listReplay) this.setList(name, path, items);
      for (const [key, axes] of this.#scrolls) {
         for (const [axis, offset] of axes) this.setScroll(key, axis, offset);
      }
      for (const [name, image] of this.#runtimeImages) {
         image.image = inst.img_register(
            name,
            image.width,
            image.height,
            image.format,
            image.bytes,
         );
      }
      for (const [key, extent] of this.#dividers) this.setDivider(key, extent);
      // Focus needs a solved scene; #tick replays any pending request.
      return true;
   }

   /** Lift CSS-replayable animation bindings out of the kernel loop: the
    * kernel stops re-solving for them and their nodes replay identical
    * keyframes as CSS animations (a fully lifted document paints once and
    * goes idle instead of mutating the DOM every frame).
    *
    * Hosts that read motion off the kernel every frame (the playground's
    * design overlay and terminal view) clear [`SlabElement.lift`] instead. */
   #liftAnimations(inst: KInst, painter: Painter): void {
      this.#animSheet.replaceSync('');
      if (!SlabElement.lift) return;
      const lifted: LiftedAnimation[] = JSON.parse(inst.lift_animations_json());
      if (lifted.length === 0) return;
      const { rules, byNode, byGroup } = liftedAnimationCss(lifted);
      this.#animSheet.replaceSync(rules);
      painter.animations = byNode;
      painter.groupAnimations = byGroup;
   }

   /** Resize observer + env media listeners; idempotent across swaps. */
   #wire(): void {
      if (this.#ro) return;
      this.#ro = new ResizeObserver(() => this.#measure());
      this.#ro.observe(this);
      const envChange = () => {
         if (this.isConnected && this.#envReady) this.#setEnv(this.#vw, this.#vh);
      };
      this.#mqDark.addEventListener('change', envChange);
      this.#mqCoarse.addEventListener('change', envChange);
   }

   /** Cancel live gesture state and surface every lifecycle signal. */
   #endInstance(eventType: typeof E_BLUR | typeof E_CLOSE): void {
      const inst = this.#inst;
      const statics = this.#statics;
      if (!inst || !statics) return;
      const pending = JSON.parse(inst.take_signals_json()) as Effects;
      const terminal = this.#dispatchKernel(inst, kernelEvent(eventType));
      this.#emitSignals(pending, statics);
      this.#applyEffects(terminal, statics);
   }

   /** Drop the current instance and every DOM artifact it painted. */
   #teardown(): void {
      this.#endInstance(E_CLOSE);
      this.#painter?.dispose();
      this.#holeRo?.disconnect();
      this.#holeRo = null;
      this.#holeEls = [];
      this.#holesLayer.replaceChildren();
      this.#ops.replaceChildren();
      this.#a11yNodes.clear();
      this.#a11yLayer.replaceChildren();
      this.#caret.hidden = true;
      this.#editFocus = 0xffffffff;
      for (const url of this.#ownImageUrls) URL.revokeObjectURL(url);
      this.#ownImageUrls = [];
      this.#inst?.free();
      this.#inst = null;
      this.#statics = null;
      this.#painter = null;
      this.#lastFrame = null;
      this.#scene = null;
   }

   static #loadSlir(cls: CachedSlabConstructor): Promise<Uint8Array> {
      if (!Object.hasOwn(cls, kBytes)) {
         if (cls.slirIsUrl) {
            cls[kBytes] = fetch(String(cls.slir)).then(async (response) => {
               if (!response.ok) throw new Error(`slab: fetching SLIR failed (${response.status})`);
               return new Uint8Array(await response.arrayBuffer());
            });
         } else if (typeof cls.slir === 'string') {
            cls[kBytes] = Promise.resolve(decodeBase64(cls.slir));
         } else {
            cls[kBytes] = Promise.resolve(cls.slir);
         }
      }
      return cls[kBytes] ?? Promise.resolve(new Uint8Array(0));
   }

   static #fonts(cls: CachedSlabConstructor, statics: Statics): (FontCss | null)[] {
      const cached = Object.hasOwn(cls, kFonts) ? cls[kFonts] : undefined;
      if (cached) return cached;
      const fonts = SlabElement.#fontsFor(statics);
      cls[kFonts] = fonts;
      return fonts;
   }

   /** Resolve authored names through registered faces, otherwise use CSS family
    * lookup with the compiled class as the generic fallback. */
   static #fontsFor(statics: Statics): (FontCss | null)[] {
      const fonts: (FontCss | null)[] = [];
      for (let index = 0; index < statics.font_upem.length; index++) {
         const named = statics.strs[statics.font_family[index]] ?? '';
         const fallback = statics.font_class[index] === 1 ? 'monospace' : 'sans-serif';
         const face = registeredFaces.get(fontKey(named));
         fonts.push({
            family:
               face?.cssFamily ??
               (named !== '' ? `${JSON.stringify(named)}, ${fallback}` : fallback),
            upem: statics.font_upem[index],
            ascent: statics.font_ascent[index],
            descent: statics.font_descent[index],
         });
      }
      return fonts;
   }

   static #images(cls: CachedSlabConstructor, statics: Statics, inst: KInst): (string | null)[] {
      const cached = Object.hasOwn(cls, kImages) ? cls[kImages] : undefined;
      if (cached) return cached;
      const urls = SlabElement.#imagesFor(statics, inst);
      cls[kImages] = urls;
      return urls;
   }

   /** Object URLs for embedded image data. Callers own uncached URLs. */
   static #imagesFor(statics: Statics, inst: KInst): (string | null)[] {
      const urls: (string | null)[] = [];
      for (let index = 0; index < statics.img_src.length; index++) {
         const data = inst.image_data(index);
         const source = Uint8Array.from(data).buffer;
         urls.push(
            source.byteLength === 0
               ? null
               : URL.createObjectURL(new Blob([source], { type: 'image/png' })),
         );
      }
      return urls;
   }

   #applyFont(key: string, face: RegisteredFace): void {
      const inst = this.#inst;
      const painter = this.#painter;
      if (!inst || !painter || this.#appliedFonts.has(key)) return;
      const metrics = face.metrics;
      const table = inst.font_register(
         face.family,
         metrics.weight,
         metrics.upem,
         metrics.ascent,
         metrics.descent,
         metrics.lineGap,
         metrics.defaultAdvance,
         metrics.cps,
         metrics.gids,
         metrics.advs,
      );
      painter.fonts[table] = {
         family: face.cssFamily,
         upem: metrics.upem,
         ascent: metrics.ascent,
         descent: metrics.descent,
      };
      this.#fontSheet.replaceSync(fontRulesCss(painter.fonts));
      this.#appliedFonts.add(key);
      this.#schedule();
   }

   /** Register a runtime font for all current and future SLIR instances. */
   static registerFont(name: string, bytes: Uint8Array): boolean {
      const metrics = parseFontMetrics(bytes);
      if (!metrics) return false;
      const key = fontKey(name);
      const cssFamily = `slab-f${fontSeq++}`;
      const face: RegisteredFace = { family: name, bytes, metrics, cssFamily };
      registeredFaces.set(key, face);
      if (typeof FontFace === 'function') {
         const source = Uint8Array.from(bytes).buffer;
         const loaded = new FontFace(cssFamily, source, {
            weight: String(metrics.weight),
            ascentOverride: `${(metrics.ascent / metrics.upem) * 100}%`,
            descentOverride: `${(-metrics.descent / metrics.upem) * 100}%`,
            lineGapOverride: '0%',
         });
         document.fonts.add(loaded);
         void loaded.load().catch(() => undefined);
      }
      for (const element of mountedElements) element.#applyFont(key, face);
      return true;
   }

   // -------------------------------------------------------------------- env

   #measure(): void {
      if (!this.#inst) return;
      const w = this.clientWidth;
      let h = this.clientHeight;
      if (w === 0 && h === 0 && !this.#selfSized) return;
      if (h === 0) this.#selfSized = true;
      // Self-sized hosts keep vh unbounded (0) and take height from the frame.
      if (this.#selfSized) h = 0;
      if (this.#envReady && w === this.#vw && h === this.#vh) return;
      this.#setEnv(w, h);
   }

   #setEnv(vw: number, vh: number): void {
      const inst = this.#inst;
      if (!inst) return;
      this.#vw = vw;
      this.#vh = vh;
      this.#envReady = true;
      inst.set_env(vw, vh, 0, this.#mqDark.matches, this.#mqCoarse.matches);
      this.#schedule();
   }

   // ------------------------------------------------------------- frame loop

   #schedule(): void {
      if (this.#raf === 0 && this.isConnected) {
         this.#raf = requestAnimationFrame(this.#tick);
      }
   }

   #tick = (t: number): void => {
      this.#raf = 0;
      const inst = this.#inst;
      const statics = this.#statics;
      const painter = this.#painter;
      if (!inst || !statics || !painter || !this.#envReady) return;
      const decoded = decodeFrame(inst.frame(t));
      this.#lastFrame = decoded;
      this.#scene = null;
      painter.paint(decoded, this.sceneSnapshot());
      this.#syncHoles();
      this.#refreshCaret();
      this.#syncSemantics();
      if (this.#selfSized) {
         const height = `${decoded.height}px`;
         if (this.style.height !== height) this.style.height = height;
      }
      const pending: Effects = JSON.parse(inst.take_signals_json());
      this.#emitSignals(pending, statics);
      // Geometry consumers (design overlays) resync after every paint.
      this.dispatchEvent(new CustomEvent('slab-frame'));
      if (decoded.dirty || decoded.motionActive) this.#schedule();
      if (this.#pendingFocus !== null) {
         const [key, visible] = this.#pendingFocus;
         this.#pendingFocus = null;
         if (inst.set_focus(key, visible)) this.#schedule();
      }
   };

   // ------------------------------------------------------------------ holes

   #makeHoles(): void {
      const statics = this.#statics;
      if (!statics || statics.holes.length === 0) return;
      this.#holeRo = new ResizeObserver(() => this.#measureHoleContent());
      for (const hole of statics.holes) {
         const element = document.createElement('div');
         element.className = 'slab-hole';
         element.style.overflow = hole.scroll ? 'auto' : 'hidden';
         const slot = document.createElement('slot');
         slot.name = hole.name;
         slot.addEventListener('slotchange', () => {
            this.#holeRo?.disconnect();
            for (const holeElement of this.#holeEls) {
               const assignedSlot = holeElement.querySelector('slot');
               if (!assignedSlot) continue;
               for (const assigned of assignedSlot.assignedElements())
                  this.#holeRo?.observe(assigned);
            }
            this.#measureHoleContent();
         });
         element.appendChild(slot);
         this.#holesLayer.appendChild(element);
         this.#holeEls.push(element);
      }
   }

   #measureHoleContent(): void {
      const inst = this.#inst;
      if (!inst) return;
      for (let index = 0; index < this.#holeEls.length; index++) {
         const element = this.#holeEls[index];
         inst.set_hole_size(index, element.scrollWidth, element.scrollHeight);
      }
      this.#schedule();
   }

   #syncHoles(): void {
      const inst = this.#inst;
      if (!inst || this.#holeEls.length === 0) return;
      const holes: HoleRect[] = JSON.parse(inst.holes_json());
      for (const hole of holes) {
         const element = this.#holeEls[hole.hole];
         if (!element) continue;
         element.style.left = `${hole.x}px`;
         element.style.top = `${hole.y}px`;
         element.style.width = `${hole.w}px`;
         element.style.height = `${hole.h}px`;
      }
   }

   // ------------------------------------------------------------------ input

   #listen(): void {
      this.addEventListener('pointerdown', (event) => {
         if (this.#overHole(event)) return;
         event.preventDefault();
         if (event.button === 0) {
            this.setPointerCapture(event.pointerId);
            this.#ime.focus({ preventScroll: true });
         }
         const { x, y } = this.#xy(event);
         this.#dispatch(
            kernelEvent(E_POINTER_DOWN, {
               x,
               y,
               button: event.button,
               clicks: event.detail,
               modifiers: modsOf(event),
            }),
         );
      });
      this.addEventListener('pointermove', (event) => {
         const { x, y } = this.#xy(event);
         this.#dispatch(
            kernelEvent(E_POINTER_MOVE, {
               x,
               y,
               dx: event.movementX,
               dy: event.movementY,
               modifiers: modsOf(event),
            }),
         );
      });
      this.addEventListener('pointerleave', (event) => {
         if (this.hasPointerCapture(event.pointerId)) return;
         this.#dispatch(
            kernelEvent(E_POINTER_MOVE, {
               x: -1,
               y: -1,
               dx: event.movementX,
               dy: event.movementY,
               modifiers: modsOf(event),
            }),
         );
      });
      const pointerUp = (event: PointerEvent) => {
         const { x, y } = this.#xy(event);
         this.#dispatch(
            kernelEvent(E_POINTER_UP, {
               x,
               y,
               dx: event.movementX,
               dy: event.movementY,
               button: event.button,
               modifiers: modsOf(event),
            }),
         );
      };
      this.addEventListener('pointerup', pointerUp);
      this.addEventListener('pointercancel', () => {
         this.#dispatch(kernelEvent(E_BLUR));
      });
      this.addEventListener('contextmenu', (event) => {
         if (!this.#overHole(event)) event.preventDefault();
      });
      this.addEventListener(
         'wheel',
         (event) => {
            if (this.#overHole(event)) return;
            const { x, y } = this.#xy(event);
            const effects = this.#dispatch(
               kernelEvent(E_WHEEL, {
                  x,
                  y,
                  dx: event.deltaX,
                  dy: event.deltaY,
                  modifiers: modsOf(event),
               }),
            );
            if (effects.repaint) event.preventDefault();
         },
         { passive: false },
      );
      this.addEventListener('keydown', (event) => {
         if (event.isComposing || event.keyCode === 229) return;
         const effects = this.#dispatch(
            kernelEvent(E_KEY_DOWN, { key: event.key, modifiers: modsOf(event) }),
         );
         // Printable keys reach the textarea and return as E_TEXT. Enter is
         // always kernel-owned to avoid a duplicate textarea insertion.
         if (
            event.key === 'Enter' ||
            (effects.repaint && (event.key.length > 1 || event.key === ' '))
         ) {
            event.preventDefault();
         }
      });
      this.#ime.addEventListener('input', (event) => {
         if (!(event instanceof InputEvent) || event.isComposing) return;
         if (this.#suppressInput) {
            this.#suppressInput = false;
            this.#ime.value = '';
            return;
         }
         const text = event.data ?? this.#ime.value;
         this.#ime.value = '';
         if (text !== '') this.#dispatch(kernelEvent(E_TEXT, { text }));
      });
      this.addEventListener('compositionstart', () => {
         this.#dispatch(kernelEvent(E_COMPOSITION_START));
      });
      this.addEventListener('compositionupdate', (event) => {
         this.#dispatch(kernelEvent(E_COMPOSITION_UPDATE, { text: event.data ?? '' }));
      });
      this.addEventListener('compositionend', (event) => {
         this.#suppressInput = true;
         setTimeout(() => {
            this.#suppressInput = false;
         }, 0);
         this.#ime.value = '';
         this.#dispatch(kernelEvent(E_COMPOSITION_END, { text: event.data ?? '' }));
      });
      this.addEventListener('paste', (event) => {
         const text = event.clipboardData?.getData('text/plain') ?? '';
         event.preventDefault();
         if (text !== '') this.#dispatch(kernelEvent(E_PASTE, { text }));
      });
      this.addEventListener('cut', (event) => {
         event.preventDefault();
         this.#dispatch(kernelEvent(E_CUT));
      });
      this.addEventListener('copy', () => {
         // Kernel copy is a no-op (§15.6): clipboard is embedding territory.
         this.#dispatch(kernelEvent(E_COPY));
      });
      this.addEventListener('focusout', (event) => {
         const related = event.relatedTarget;
         if (
            related instanceof Node &&
            (this === related || this.contains(related) || this.shadowRoot?.contains(related))
         ) {
            return;
         }
         this.#dispatch(kernelEvent(E_BLUR));
      });
   }

   #xy(event: { clientX: number; clientY: number }): { x: number; y: number } {
      const rect = this.getBoundingClientRect();
      return { x: event.clientX - rect.left, y: event.clientY - rect.top };
   }

   #overHole(event: Event): boolean {
      for (const node of event.composedPath()) {
         if (node === this) break;
         if (node instanceof HTMLElement && node.classList.contains('slab-hole')) return true;
      }
      return false;
   }

   #dispatch(event: KernelEvent): Effects {
      const inst = this.#inst;
      const statics = this.#statics;
      if (!inst || !statics) return emptyEffects();
      const effects = this.#dispatchKernel(inst, event);
      this.#applyEffects(effects, statics);
      return effects;
   }

   #dispatchKernel(inst: KInst, event: KernelEvent): Effects {
      return JSON.parse(
         inst.dispatch_json(
            event.eventType,
            event.x,
            event.y,
            event.dx,
            event.dy,
            event.button,
            event.key,
            event.text,
            event.modifiers,
            event.clicks,
         ),
      ) as Effects;
   }

   #applyEffects(effects: Effects, statics: Statics): void {
      for (const scroll of effects.scrolls) {
         let axes = this.#scrolls.get(scroll.key);
         if (!axes) {
            axes = new Map();
            this.#scrolls.set(scroll.key, axes);
         }
         axes.set(scroll.axis, scroll.off);
      }
      this.#emitSignals(effects, statics);
      const cursor = CURSORS[effects.cursor] ?? '';
      if (this.style.cursor !== cursor) this.style.cursor = cursor;
      this.#showCaret(effects);
      if (effects.repaint) this.#schedule();
   }

   #emitSignals(effects: Effects, statics: Statics): void {
      for (let index = 0; index < effects.sig_name.length; index++) {
         const name = statics.strs[effects.sig_name[index]] ?? '';
         const item = effects.sig_item[index];
         const meta = effects.sig_meta[index];
         const textBearing = statics.signals.some(
            (signal) =>
               signal.name === name &&
               (signal.trigger === 1 || signal.trigger === 2 || signal.trigger === 8),
         );
         const detail: SlabSignalDetail = textBearing
            ? { text: effects.sig_text[index], item, meta }
            : { item, meta };
         this.dispatchEvent(
            new CustomEvent<SlabSignalDetail>(name, { detail, bubbles: true, composed: true }),
         );
      }
   }

   /** Re-derive caret/IME rects from the fresh solve (FRAME.md: dispatch-time
    * geometry is of the LAST solve; hosts refresh after the next frame). */
   #refreshCaret(): void {
      const inst = this.#inst;
      if (!inst) return;
      const effects: Effects = JSON.parse(inst.caret_effects_json());
      this.#showCaret(effects);
   }

   #showCaret(effects: Effects): void {
      if (effects.has_caret) {
         this.#caret.style.left = `${effects.caret_x}px`;
         this.#caret.style.top = `${effects.caret_y}px`;
         this.#caret.style.width = `${effects.caret_w}px`;
         this.#caret.style.height = `${effects.caret_h}px`;
         this.#caret.hidden = false;
      } else {
         this.#caret.hidden = true;
      }
      this.#editFocus = effects.has_ime ? effects.focus : 0xffffffff;
      if (effects.has_ime) {
         this.#ime.style.left = `${effects.ime_x}px`;
         this.#ime.style.top = `${effects.ime_y}px`;
         this.#ime.style.height = `${effects.ime_h}px`;
      }
   }

   // ----------------------------------------------------------------- params

   attributeChangedCallback(name: string, _old: string | null, value: string | null): void {
      if (!this.#inst) {
         this.#pendingAttrs.set(name, value);
         return;
      }
      this.#applyAttr(name, value);
   }

   #applyAttr(attr: string, value: string | null): void {
      const statics = this.#statics;
      if (!statics) return;
      if (attr === 'theme') {
         this.setTheme(value ?? '');
         return;
      }
      const schemas = slabConstructor(this).listSchemas;
      let listName: string | undefined;
      for (const name in schemas) {
         if (name.replace(/_/g, '-') === attr) {
            listName = name;
            break;
         }
      }
      if (listName !== undefined) {
         if (value === null) return;
         try {
            this.setList(listName, '', JSON.parse(value));
         } catch {
            console.warn(`slab: list attribute ${attr} must be a JSON array`);
         }
         return;
      }
      for (const param of statics.params) {
         if (param.name.replace(/_/g, '-') !== attr) continue;
         if (param.ty === 4) {
            this.setParam(param.name, value !== null && value !== 'false');
         } else if (value !== null) {
            this.setParam(param.name, value);
         }
         return;
      }
   }

   /** Set a declared param; returns false on unknown name / bad coercion /
    * kernel-side type or enum-member rejection. */
   setParam(name: string, value: unknown): boolean {
      const inst = this.#inst;
      const statics = this.#statics;
      if (!inst || !statics) {
         this.#props.set(name, value);
         return true;
      }
      for (let index = 0; index < statics.params.length; index++) {
         const definition = statics.params[index];
         if (definition.name !== name) continue;
         const param = coerceParam(definition.ty, value);
         if (!param) return false;
         if (!inst.set_param(index, param.kind, param.num, param.s, param.rgba, param.sym)) {
            return false;
         }
         this.#props.set(name, value);
         this.#schedule();
         return true;
      }
      return false;
   }

   /** Last value set through the property/attribute surface (not kernel state). */
   getParam(name: string): unknown {
      return this.#props.get(name);
   }

   /** Validate, then replace a root or nested declared list and its descendants. */
   setList(name: string, path: string, value: unknown): boolean {
      const ctor = slabConstructor(this);
      const root = ctor.listSchemas[name];
      if (!root || !Array.isArray(value)) {
         console.warn(`slab: ${name} must be an array`);
         return false;
      }

      let row = root.row ?? -1;
      let schema: ListRowSchema = root;
      if (path) {
         const parts = path.split('.');
         if (parts.length % 2 !== 0) {
            console.warn(`slab: invalid nested list path ${JSON.stringify(path)}`);
            return false;
         }
         for (let part = 0; part < parts.length; part += 2) {
            const itemIndex = Number(parts[part]);
            if (!Number.isInteger(itemIndex) || itemIndex < 0) {
               console.warn(`slab: invalid nested list path ${JSON.stringify(path)}`);
               return false;
            }
            const field = schema.fields.find((candidate) => candidate.name === parts[part + 1]);
            if (!field?.sub) {
               console.warn(`slab: unknown nested list path ${JSON.stringify(path)}`);
               return false;
            }
            row = field.sub - 1;
            const nested = ctor.listSchemaRows[row];
            if (!nested) {
               console.warn(`slab: missing nested schema row ${row}`);
               return false;
            }
            schema = nested;
         }
      }

      const normalize = (
         activeSchema: ListRowSchema,
         activeRow: number,
         activePath: string,
         raw: unknown,
      ): NormalizedListWrite | null => {
         if (!Array.isArray(raw)) return null;
         const validNames = new Set(activeSchema.fields.map((field) => field.name));
         const items: Record<string, unknown>[] = [];
         const values: (ParamValue | null)[][] = [];
         const keys: string[] = [];
         const children: NormalizedListWrite[] = [];
         const seenKeys = new Set<string>();
         for (let index = 0; index < raw.length; index++) {
            const item = raw[index];
            if (!isPlainRecord(item)) {
               console.warn(`slab: ${name}[${index}] must be a plain object`);
               return null;
            }
            if (
               Object.hasOwn(item, 'key') &&
               typeof item.key !== 'string' &&
               (typeof item.key !== 'number' || !Number.isFinite(item.key))
            ) {
               console.warn(`slab: ${name}[${index}].key must be a string or finite number`);
               return null;
            }
            const key = String(item.key ?? index);
            if (key.length === 0) {
               console.warn(`slab: ${name}[${index}].key must not be empty`);
               return null;
            }
            if (seenKeys.has(key)) {
               console.warn(`slab: duplicate key ${JSON.stringify(key)} in ${name}`);
               return null;
            }
            seenKeys.add(key);
            keys.push(key);
            for (const fieldName in item) {
               if (fieldName !== 'key' && !validNames.has(fieldName)) {
                  console.warn(`slab: unknown field ${name}[${index}].${fieldName}`);
                  return null;
               }
            }
            const itemValues: (ParamValue | null)[] = [];
            for (const field of activeSchema.fields) {
               if (field.sub) {
                  itemValues.push(null);
                  const nestedSchema = ctor.listSchemaRows[field.sub - 1];
                  if (!nestedSchema) return null;
                  const childPath = activePath
                     ? `${activePath}.${index}.${field.name}`
                     : `${index}.${field.name}`;
                  const child = normalize(
                     nestedSchema,
                     field.sub - 1,
                     childPath,
                     Object.hasOwn(item, field.name) ? item[field.name] : [],
                  );
                  if (!child) {
                     console.warn(`slab: ${name}[${index}].${field.name} must be an array`);
                     return null;
                  }
                  children.push(child);
                  continue;
               }
               if (!Object.hasOwn(item, field.name)) {
                  itemValues.push(null);
                  continue;
               }
               const param = coerceParam(field.type, item[field.name]);
               if (!param || (field.enum && !field.enum.includes(param.sym))) {
                  console.warn(`slab: invalid value for ${name}[${index}].${field.name}`);
                  return null;
               }
               itemValues.push(param);
            }
            values.push(itemValues);
            items.push({ ...item });
         }
         return {
            row: activeRow,
            schema: activeSchema,
            path: activePath,
            items,
            keys,
            values,
            children,
         };
      };

      const write = normalize(schema, row, path, value);
      if (!write) return false;
      let paths = this.#lists.get(name);
      if (!paths) {
         paths = new Map();
         this.#lists.set(name, paths);
      }
      const cache = (entry: NormalizedListWrite): void => {
         paths?.set(entry.path, entry.items);
         for (const child of entry.children) cache(child);
      };

      const inst = this.#inst;
      const statics = this.#statics;
      if (!inst || !statics) {
         cache(write);
         return true;
      }
      const validate = (entry: NormalizedListWrite): boolean => {
         const definition =
            entry.row >= 0
               ? statics.lists[entry.row]
               : statics.lists.find((candidate) => candidate.param === root.param);
         if (!definition || definition.fields.length !== entry.schema.fields.length) {
            return false;
         }
         for (let fieldIndex = 0; fieldIndex < entry.schema.fields.length; fieldIndex++) {
            const field = entry.schema.fields[fieldIndex];
            const decoded = definition.fields[fieldIndex];
            if (decoded.name !== field.name || decoded.ty !== field.type) return false;
            if (!field.sub) {
               for (const itemValues of entry.values) {
                  if (!itemValues[fieldIndex]) itemValues[fieldIndex] = decoded.default;
               }
            }
         }
         return entry.children.every(validate);
      };
      if (!validate(write)) {
         console.warn(`slab: list schema mismatch for ${name}`);
         return false;
      }

      const apply = (entry: NormalizedListWrite): boolean => {
         if (!inst.set_list_len(root.param, entry.path, entry.items.length)) return false;
         for (let index = 0; index < entry.items.length; index++) {
            if (!inst.set_list_key(root.param, entry.path, index, entry.keys[index])) return false;
            for (let fieldIndex = 0; fieldIndex < entry.schema.fields.length; fieldIndex++) {
               const field = entry.schema.fields[fieldIndex];
               if (field.sub) continue;
               const param = entry.values[index][fieldIndex];
               if (
                  !param ||
                  !inst.set_list_field(
                     root.param,
                     entry.path,
                     index,
                     field.name,
                     param.kind,
                     param.num,
                     param.s,
                     param.rgba,
                     param.sym,
                  )
               ) {
                  return false;
               }
            }
         }
         return entry.children.every(apply);
      };
      if (!apply(write)) return false;
      cache(write);
      this.#schedule();
      return true;
   }

   /** Last list value accepted for one declared list and runtime path. */
   getList(name: string, path: string): unknown {
      return this.#lists.get(name)?.get(path);
   }

   /** Select a compiler-declared theme; the empty name restores authored values. */
   setTheme(name: string): boolean {
      const inst = this.#inst;
      if (!inst) {
         this.#theme = name;
         return true;
      }
      if (!inst.set_theme(name)) return false;
      this.#theme = name;
      this.#schedule();
      return true;
   }

   /** Current theme name; empty means the authored base. */
   getTheme(): string {
      return this.#inst ? this.#inst.theme() : this.#theme;
   }

   /** Current theme name; assigning selects a compiler-declared theme. */
   get theme(): string {
      return this.getTheme();
   }

   set theme(name: string) {
      this.setTheme(name);
   }

   /** Set a keyed scroll node's offset on axis `0` (main) or `1` (cross). */
   setScroll(key: string, axis: number, off: number): boolean {
      if (axis !== 0 && axis !== 1) return false;
      const inst = this.#inst;
      if (!inst) {
         let axes = this.#scrolls.get(key);
         if (!axes) {
            axes = new Map();
            this.#scrolls.set(key, axes);
         }
         axes.set(axis, off);
         return true;
      }
      if (!inst.set_scroll(key, axis, off)) return false;
      let axes = this.#scrolls.get(key);
      if (!axes) {
         axes = new Map();
         this.#scrolls.set(key, axes);
      }
      axes.set(axis, off);
      this.#schedule();
      return true;
   }

   /** Move focus to a keyed focusable node; the empty key clears focus.
    * `visible` shows the keyboard-grade focus ring. Before the first frame
    * the request is buffered and replayed once the scene has solved. */
   setFocus(key: string, visible = true): boolean {
      const inst = this.#inst;
      if (!inst || !this.#lastFrame) {
         this.#pendingFocus = [key, visible];
         return true;
      }
      if (!inst.set_focus(key, visible)) return false;
      this.#schedule();
      return true;
   }

   /** Read a keyed scroll offset on axis `0` (main) or `1` (cross). */
   getScroll(key: string, axis: number): number {
      if (axis !== 0 && axis !== 1) return 0;
      const inst = this.#inst;
      return inst ? inst.get_scroll(key, axis) : (this.#scrolls.get(key)?.get(axis) ?? 0);
   }

   /** Register or replace one named runtime image and return its unified index. */
   imgRegister(
      name: string,
      width: number,
      height: number,
      format: number,
      bytes: Uint8Array,
   ): number {
      const inst = this.#inst;
      if (!inst) return -1;
      const image = inst.img_register(name, width, height, format, bytes);
      if (image < 0) return image;
      this.#runtimeImages.set(name, {
         image,
         width,
         height,
         format,
         bytes: bytes.slice(),
      });
      this.#schedule();
      return image;
   }

   /** Unregister one named runtime image. */
   imgUnregister(name: string): boolean {
      const inst = this.#inst;
      if (!inst?.img_unregister(name)) return false;
      const registration = this.#runtimeImages.get(name);
      if (registration) this.#painter?.releaseImage(registration.image);
      this.#runtimeImages.delete(name);
      this.#schedule();
      return true;
   }

   /** Read immutable metadata for one embedded or runtime image index. */
   imgInfo(image: number): ImageInfo | null {
      const inst = this.#inst;
      return inst ? (JSON.parse(inst.image_info_json(image)) as ImageInfo | null) : null;
   }

   /** Copy the encoded or raw bytes for one embedded or runtime image index. */
   imgBytes(image: number): Uint8Array {
      return this.#inst?.image_data(image) ?? new Uint8Array();
   }

   /** Scroll every retained ancestor minimally to reveal a keyed node. */
   reveal(key: string, margin: number): boolean {
      const inst = this.#inst;
      if (!inst?.reveal(key, margin)) return false;
      this.#schedule();
      return true;
   }

   /** Reveal one virtual-list item using start, center, end, or nearest alignment. */
   revealItem(each: string, index: number, align: number): boolean {
      const inst = this.#inst;
      if (!inst?.reveal_item(each, index, align)) return false;
      this.#schedule();
      return true;
   }

   /** Return a virtual list's materialized range `[start, end)`. */
   eachWindow(each: string): EachWindow {
      const inst = this.#inst;
      return inst ? (JSON.parse(inst.each_window_json(each)) as EachWindow) : ([-1, -1] as const);
   }

   /** Set one keyed divider's retained extent overlay. */
   setDivider(key: string, extent: number): boolean {
      const inst = this.#inst;
      if (!inst) {
         this.#dividers.set(key, extent);
         return true;
      }
      if (!inst.set_divider(key, extent)) return false;
      this.#dividers.set(key, extent);
      this.#schedule();
      return true;
   }

   /** Read one keyed divider extent, or `-1` when it is unknown. */
   getDivider(key: string): number {
      const inst = this.#inst;
      return inst ? inst.get_divider(key) : (this.#dividers.get(key) ?? -1);
   }

   #syncSemantics(): void {
      const scene = this.sceneSnapshot();
      const retained = new Set<string>();
      const occurrences = new Map<string, number>();
      const elements: HTMLDivElement[] = [];
      const canonical = new Map<string, HTMLDivElement>();

      for (const node of scene) {
         const occurrence = occurrences.get(node.key) ?? 0;
         occurrences.set(node.key, occurrence + 1);
         const identity = semanticSceneIdentity(node.key, occurrence);
         retained.add(identity);
         let element = this.#a11yNodes.get(identity);
         if (!element) {
            element = document.createElement('div');
            element.className = 'slab-a11y-node';
            element.dataset.slabKey = node.key;
            element.id = semanticNodeId(node.key, occurrence);
            this.#a11yNodes.set(identity, element);
         }
         elements.push(element);
         if (occurrence === 0) canonical.set(node.key, element);
      }
      for (const [identity, element] of this.#a11yNodes) {
         if (retained.has(identity)) continue;
         element.remove();
         this.#a11yNodes.delete(identity);
      }

      let focused: HTMLDivElement | null = null;
      const focusable: HTMLDivElement[] = [];
      for (let index = 0; index < scene.length; index++) {
         const node = scene[index];
         const element = elements[index];
         if (!element) continue;
         const parentNode = node.parent >= 0 ? scene[node.parent] : undefined;
         const parent = node.parent >= 0 ? elements[node.parent] : this.#a11yLayer;
         if (parent && element.parentElement !== parent) parent.append(element);

         const originX = parentNode?.x ?? 0;
         const originY = parentNode?.y ?? 0;
         element.style.left = `${node.x - originX}px`;
         element.style.top = `${node.y - originY}px`;
         element.style.width = `${Math.max(0, node.w)}px`;
         element.style.height = `${Math.max(0, node.h)}px`;
         element.dataset.rotation = String(node.rotation);
         setSemanticTransform(element, node);

         setOptionalAttribute(element, 'role', node.role || null);
         setOptionalAttribute(element, 'aria-label', node.label || null);
         setOptionalAttribute(element, 'aria-description', node.desc || null);
         setOptionalAttribute(element, 'aria-checked', node.checked);
         setOptionalAttribute(element, 'aria-expanded', node.expanded);
         setOptionalAttribute(element, 'aria-selected', node.selected);
         const activeDescendant = canonical.get(node.active_descendant);
         const controls = canonical.get(node.controls);
         setOptionalAttribute(element, 'aria-activedescendant', activeDescendant?.id ?? null);
         setOptionalAttribute(element, 'aria-controls', controls?.id ?? null);
         setOptionalAttribute(element, 'aria-valuenow', node.value_now);
         setOptionalAttribute(element, 'aria-valuemin', node.value_min);
         setOptionalAttribute(element, 'aria-valuemax', node.value_max);
         setOptionalAttribute(element, 'aria-valuetext', node.value_text || null);
         setOptionalAttribute(element, 'aria-modal', node.modal);
         setOptionalAttribute(element, 'aria-live', node.live);
         setOptionalAttribute(element, 'aria-atomic', node.live_atomic);
         setOptionalAttribute(element, 'aria-level', node.level);
         setOptionalAttribute(element, 'aria-posinset', node.pos_in_set);
         setOptionalAttribute(element, 'aria-setsize', node.set_size);
         setOptionalAttribute(element, 'aria-disabled', node.disabled ? true : null);
         element.toggleAttribute('data-slab-disabled', node.disabled);
         element.toggleAttribute('data-slab-focused', node.focused);
         const editor = node.focused && node.node === this.#editFocus;
         element.dataset.slabEditor = editor ? 'true' : 'false';
         if (editor) element.setAttribute('contenteditable', 'plaintext-only');
         else element.removeAttribute('contenteditable');
         const acceptsFocus =
            (node.flags & F_FOCUSABLE) !== 0 && (node.flags & F_INERT) === 0 && !node.disabled;
         if (acceptsFocus) {
            focusable.push(element);
            if (node.focused) focused = element;
         } else {
            element.removeAttribute('tabindex');
         }
      }
      const entry = focused ?? focusable[0] ?? null;
      for (const element of focusable) element.tabIndex = element === entry ? 0 : -1;

      const activeElement = this.shadowRoot?.activeElement;
      if (focused && activeElement !== this.#ime && activeElement !== focused) {
         focused.focus({ preventScroll: true });
      } else if (
         !focused &&
         activeElement instanceof HTMLElement &&
         activeElement.classList.contains('slab-a11y-node')
      ) {
         activeElement.blur();
      }
   }

   /** Return the current retained scene, stable until the next solve. */
   sceneSnapshot(): readonly SceneNode[] {
      const inst = this.#inst;
      if (!inst) return [];
      const scene: readonly SceneNode[] = this.#scene ?? JSON.parse(inst.scene_json());
      this.#scene = scene;
      return scene;
   }

   /** Test a point against a retained scene node, including clips and rotation. */
   sceneHitContains(index: number, x: number, y: number): boolean {
      return this.#inst?.hit_contains(index, x, y) ?? false;
   }

   /** Return retained scene indices from root through the requested node. */
   sceneChain(index: number): number[] {
      const inst = this.#inst;
      if (!inst) return [];
      const chain: number[] = JSON.parse(inst.chain_json(index));
      return chain;
   }

   // ------------------------------------------------------------------ debug

   #registerDebug(): void {
      const cls = slabConstructor(this);
      if (!cls.debug && !SlabElement.debug && Reflect.get(globalThis, '__SLAB_DEBUG__') !== true) {
         return;
      }
      const current = Reflect.get(globalThis, '__slabDebug');
      let debugMap: Map<Element, SlabDebugEntry>;
      if (current instanceof Map) {
         debugMap = current;
      } else {
         debugMap = new Map<Element, SlabDebugEntry>();
         Reflect.set(globalThis, '__slabDebug', debugMap);
      }
      debugMap.set(this, {
         geom: () =>
            this.sceneSnapshot().map((node) => ({
               key: node.key,
               node: node.node,
               x: node.x,
               y: node.y,
               w: node.w,
               h: node.h,
            })),
         frame: () => {
            const frame = this.#lastFrame;
            return {
               width: frame?.width ?? 0,
               height: frame?.height ?? 0,
               ops: frame?.ops.length ?? 0,
            };
         },
      });
   }
}
