// @stencil-hq/wslab DOM painter: retained-mode translation of kernel FrameOps
// absolutely-positioned shadow-DOM elements. NO layout or text measurement
// happens here — every coordinate comes from the kernel Frame; the only
// arithmetic is wrapper-offset subtraction and the FRAME.md baseline formula
// (ascent from the SLIR FONT table).
//
// Keying: paint identity is `<opTag><node>#<occurrence>` (Text ops get one
// span per line); node-owned groups use the same scheme, while host envelopes
// and structural clips key on occurrence alone. Auxiliary siblings extend it:
// gradient stroke rings key `S<node>` and progressive-blur backdrop bands
// `Db<i>`. A retained key that switches element kind (rect div ⇄ squircle SVG)
// drops and recreates its element. Placement is delta-only: a per-layer cursor
// walks retained children. The DOM is touched only when an element is missing
// or out of order — a stable frame performs zero mutations, so running CSS
// animations (lifted keyframes and the caret blink) never restart.
// Genuine reorders use `moveBefore` where available, which moves a connected
// node without resetting its animation/iframe/focus state.
//
// Documented degradations (spec/support.toml `web` column):
// - stroke_align center/outside → CSS outline (side mask degrades to all
//   four sides); dash → border/outline-style dashed (on/off lengths lost).
// - dashed or per-side gradient box strokes → first stop; conic paints where
//   CSS conic-gradient can't reach (path fills/strokes, squircle svg strokes)
//   → first stop. Conic squircle FILLS paint via a clip-path'd sibling div.
// - smooth rects: inset shadows dropped, shadow spread lost, side masks
//   stroke the full perimeter (inline-svg drop-shadow branch).
// - backdrop-mask → six hard-banded backdrop divs (contract §6.6).

import type { Frame, ImageInfo, LiftedAnimation, LiftedStop, OpRect, Statics } from './kernel.ts';

/** CSS binding for one SLIR FONT table. */
export interface FontCss {
   /** Serialized `font-family` value, including a generic fallback when needed. */
   family: string;
   /** Font units per em used to place the CSS line box. */
   upem: number;
   /** Font ascender in font units. */
   ascent: number;
   /** Font descender in font units. */
   descent: number;
}

/** Converts the kernel's little-endian packed RGBA word into a CSS color. */
export function rgbaCss(v: number): string {
   const r = v & 0xff;
   const g = (v >>> 8) & 0xff;
   const b = (v >>> 16) & 0xff;
   const a = (v >>> 24) & 0xff;
   if (a === 255) return `rgb(${r},${g},${b})`;
   return `rgba(${r},${g},${b},${a / 255})`;
}

function gradientStops(doc: Statics, h: number): string {
   const off = doc.grad_stop_off[h];
   const len = doc.grad_stop_len[h];
   const parts: string[] = [];
   for (let i = 0; i < len; i++) {
      parts.push(`${rgbaCss(doc.grad_stop_rgba[off + i])} ${doc.grad_stop_pos[off + i] * 100}%`);
   }
   return parts.join(',');
}

/** CSS gradient image for GRAD entry `h` painted into a `w`×`hh` box. */
export function gradientCss(doc: Statics, h: number, w: number, hh: number): string {
   if (doc.grad_kind[h] === 0) {
      // SLIR linear angle: 0deg = up, direction (sin, -cos) — the CSS convention.
      return `linear-gradient(${doc.grad_angle[h]}deg,${gradientStops(doc, h)})`;
   }
   if (doc.grad_kind[h] === 2) {
      // Conic sweep about the box center, clockwise from `angle`, 0 = up
      // (contract §6.1) — exactly the CSS `conic-gradient(from …)` model.
      return `conic-gradient(from ${doc.grad_angle[h]}deg at 50% 50%,${gradientStops(doc, h)})`;
   }
   const r = Math.sqrt(w * w + hh * hh) / 2;
   return `radial-gradient(circle ${r}px at 50% 50%,${gradientStops(doc, h)})`;
}

/** Solid color for a paint; gradients yield their FIRST stop. Only used
 * where the chart pins first-stop degradation: per-side/dashed box strokes
 * and conic paints outside CSS background-image (SVG has no conic). */
function strokeCss(doc: Statics, kind: number, h: number): string | null {
   if (kind === 1) return rgbaCss(h);
   if (kind === 2 && doc.grad_stop_len[h] > 0)
      return rgbaCss(doc.grad_stop_rgba[doc.grad_stop_off[h]]);
   return null;
}

/** Rounds to ≤4 decimals for generated markup/css. One formatter keeps every
 * emitted coordinate in lockstep (~40 call sites), so retained-string
 * comparisons never churn on float noise. */
function fmt(n: number): string {
   return String(Math.round(n * 1e4) / 1e4);
}

/**
 * pcg2d hash (Jarzynski–Olano, contract §6.2), seedless, u32 wrapping via
 * `Math.imul`. Every client derives grain speckle from this exact sequence,
 * so a cell's speckle is static across frames and identical across painters.
 */
function pcg2d(i: number, j: number): number {
   let vx = (Math.imul(i, 1664525) + 1013904223) >>> 0;
   let vy = (Math.imul(j, 1664525) + 1013904223) >>> 0;
   vx = (vx + Math.imul(vy, 1664525)) >>> 0;
   vy = (vy + Math.imul(vx, 1664525)) >>> 0;
   vx ^= vx >>> 16;
   vy ^= vy >>> 16;
   vx = (vx + Math.imul(vy, 1664525)) >>> 0;
   // The contract's final round only mixes vy back into itself; h_u32 is vx
   // after one more xor-shift, so the symmetric vy ops stop here.
   return (vx ^ (vx >>> 16)) >>> 0;
}

/**
 * Squircle outline `d` for a `w`×`h` box with corner radius `r` and
 * Figma-style corner smoothing `smooth` (0..1). TS twin of
 * `slab_kernel::squircle::squircle_path` (contract §6.4): the construction
 * below mirrors that constructor step for step — smoothing rolloff past
 * radius = budget/2, flank coefficients a..d, one central-arc cubic with
 * handle 4/3·tan(Δ/4)·r, corners rotated 90° clockwise per quarter turn.
 * Painters MUST NOT fork these formulas. Origin at the box top-left,
 * clockwise from the top-right corner's start; degenerate zero-length cubics
 * are kept so the segment structure is invariant.
 */
export function squirclePathD(w: number, h: number, r: number, smooth: number): string {
   const maxRadius = Math.max(Math.min(w, h) / 2, 0);
   const radius = Math.min(Math.max(r, 0), maxRadius);
   // Near the geometric limit the smoothing collapses so flanks never
   // overlap the opposite corner (Figma's rolloff).
   let s = Math.min(Math.max(smooth, 0), 1);
   if (radius > maxRadius / 2) s *= 1 - (radius - maxRadius / 2) / (maxRadius / 2);
   const p = Math.min((1 + s) * radius, maxRadius);
   const arcMeasure = 90 * (1 - s);
   const arcLen = Math.sin(((arcMeasure / 2) * Math.PI) / 180) * radius * Math.SQRT2;
   const angleAlpha = (90 - arcMeasure) / 2;
   const p3ToP4 = radius * Math.tan(((angleAlpha / 2) * Math.PI) / 180);
   const angleBeta = 45 * s;
   const c = p3ToP4 * Math.cos((angleBeta * Math.PI) / 180);
   const d = c * Math.tan((angleBeta * Math.PI) / 180);
   const b = (p - arcLen - c - d) / 3;
   const a = 2 * b;
   const handle = (4 / 3) * Math.tan(((arcMeasure / 4) * Math.PI) / 180) * radius;
   // Segment displacements for the top-right corner (traveling +x along the
   // top edge); later corners rotate them 90° clockwise per quarter turn.
   const flankIn: readonly (readonly [number, number])[] = [
      [a, 0],
      [a + b, 0],
      [a + b + c, d],
   ];
   const flankOut: readonly (readonly [number, number])[] = [
      [d, c],
      [d, b + c],
      [d, a + b + c],
   ];
   // Unit tangents at the arc join points; the handle length already carries
   // the radius factor. smooth == 0 → flanks vanish, arc spans the quarter.
   const tLen = Math.hypot(c, d);
   const tin: readonly [number, number] = tLen > 0 ? [c / tLen, d / tLen] : [1, 0];
   const tout: readonly [number, number] = tLen > 0 ? [d / tLen, c / tLen] : [0, 1];
   const rot = (dx: number, dy: number, turn: number): readonly [number, number] => {
      switch (turn % 4) {
         case 0:
            return [dx, dy];
         case 1:
            return [-dy, dx];
         case 2:
            return [-dx, -dy];
         default:
            return [dy, -dx];
      }
   };
   const starts: readonly (readonly [number, number])[] = [
      [w - p, 0],
      [w, h - p],
      [p, h],
      [0, p],
   ];
   let out = `M${fmt(w - p)} 0`;
   for (let turn = 0; turn < 4; turn++) {
      const [sx, sy] = starts[turn];
      if (turn > 0) out += `L${fmt(sx)} ${fmt(sy)}`;
      let cx = sx;
      let cy = sy;
      const flank = (pts: readonly (readonly [number, number])[]): void => {
         const abs: string[] = [];
         for (const [dx, dy] of pts) {
            const [rx, ry] = rot(dx, dy, turn);
            abs.push(`${fmt(cx + rx)} ${fmt(cy + ry)}`);
         }
         out += `C${abs.join(' ')}`;
         const [ex, ey] = rot(pts[2][0], pts[2][1], turn);
         cx += ex;
         cy += ey;
      };
      flank(flankIn);
      // Central arc as a single cubic: controls follow the join tangents.
      const [adx, ady] = rot(arcLen, arcLen, turn);
      const [t0x, t0y] = rot(tin[0], tin[1], turn);
      const [t1x, t1y] = rot(tout[0], tout[1], turn);
      const ex = cx + adx;
      const ey = cy + ady;
      out += `C${fmt(cx + handle * t0x)} ${fmt(cy + handle * t0y)} ${fmt(ex - handle * t1x)} ${fmt(ey - handle * t1y)} ${fmt(ex)} ${fmt(ey)}`;
      cx = ex;
      cy = ey;
      flank(flankOut);
   }
   return `${out}Z`;
}

/** `<stop>` rows for GRAD entry `h` (offset %, hex color, opacity) — the
 * same emission the SVG client uses, so inline defs match svg.rs output. */
function svgStops(doc: Statics, h: number): string {
   const off = doc.grad_stop_off[h];
   const len = doc.grad_stop_len[h];
   let out = '';
   for (let i = 0; i < len; i++) {
      const v = doc.grad_stop_rgba[off + i];
      const rgb = ((v & 0xff) << 16) | (v & 0xff00) | ((v >>> 16) & 0xff);
      const alpha = v >>> 24;
      out += `<stop offset="${fmt(doc.grad_stop_pos[off + i] * 100)}%" stop-color="#${rgb.toString(16).padStart(6, '0')}"${alpha < 255 ? ` stop-opacity="${fmt(alpha / 255)}"` : ''}/>`;
   }
   return out;
}

/**
 * userSpaceOnUse gradient def mapped over `box`, mirroring the SVG client's
 * geometry (rect-box math; for paths the box is the path's coord bbox).
 * Returns '' for conic entries — SVG has no conic primitive; callers fall
 * back to the first stop (chart-noted).
 */
function svgGradientDef(doc: Statics, h: number, id: string, box: PathBox): string {
   const [x, y, w, hh] = box;
   if (doc.grad_kind[h] === 2) return '';
   const cx = x + w / 2;
   const cy = y + hh / 2;
   if (doc.grad_kind[h] === 0) {
      const th = (doc.grad_angle[h] * Math.PI) / 180;
      const dx = Math.sin(th);
      const dy = -Math.cos(th);
      const ln = Math.abs(w * dx) + Math.abs(hh * dy);
      return `<linearGradient id="${id}" gradientUnits="userSpaceOnUse" x1="${fmt(cx - (dx * ln) / 2)}" y1="${fmt(cy - (dy * ln) / 2)}" x2="${fmt(cx + (dx * ln) / 2)}" y2="${fmt(cy + (dy * ln) / 2)}">${svgStops(doc, h)}</linearGradient>`;
   }
   const r = Math.sqrt(w * w + hh * hh) / 2;
   return `<radialGradient id="${id}" gradientUnits="userSpaceOnUse" cx="${fmt(cx)}" cy="${fmt(cy)}" r="${fmt(r)}">${svgStops(doc, h)}</radialGradient>`;
}

/** Mask-paint alpha at ramp position `t`: stops lerp linearly, ends clamp. */
function gradAlphaAt(doc: Statics, h: number, t: number): number {
   const off = doc.grad_stop_off[h];
   const len = doc.grad_stop_len[h];
   if (len === 0) return 0;
   const alpha = (i: number): number => (doc.grad_stop_rgba[off + i] >>> 24) / 255;
   if (t <= doc.grad_stop_pos[off]) return alpha(0);
   for (let i = 1; i < len; i++) {
      const hi = doc.grad_stop_pos[off + i];
      if (t <= hi) {
         const lo = doc.grad_stop_pos[off + i - 1];
         const f = hi > lo ? (t - lo) / (hi - lo) : 1;
         return alpha(i - 1) * (1 - f) + alpha(i) * f;
      }
   }
   return alpha(len - 1);
}

/**
 * Progressive-blur band mask (contract §6.6): an on/off hard-stop CSS
 * gradient covering the ramp regions whose mask alpha falls in band `band`
 * of `bands` (alpha ∈ [band/bands, (band+1)/bands)). Gradient ramps are
 * sampled at 64 points along their axis; solid masks are all-or-nothing.
 */
function bandMaskCss(
   doc: Statics,
   kind: number,
   h: number,
   band: number,
   bands: number,
   w: number,
   hh: number,
): string {
   const bandOf = (alpha: number): number =>
      Math.min(bands - 1, Math.max(0, Math.floor(alpha * bands)));
   if (kind === 1) {
      const on = bandOf((h >>> 24) / 255) === band;
      return `linear-gradient(${on ? '#fff' : 'transparent'} 0 0)`;
   }
   const samples = 64;
   let stops = '';
   let covered = 0; // percent emitted so far
   let run = -1; // first sample of the open on-run
   const flush = (end: number): void => {
      if (run < 0) return;
      const from = (run / samples) * 100;
      const to = (end / samples) * 100;
      if (from > covered) stops += `,transparent ${fmt(covered)}% ${fmt(from)}%`;
      stops += `,#fff ${fmt(from)}% ${fmt(to)}%`;
      covered = to;
      run = -1;
   };
   for (let i = 0; i < samples; i++) {
      if (bandOf(gradAlphaAt(doc, h, (i + 0.5) / samples)) === band) {
         if (run < 0) run = i;
      } else flush(i);
   }
   flush(samples);
   if (covered < 100) stops += `,transparent ${fmt(covered)}% 100%`;
   const ramp = stops.slice(1);
   if (doc.grad_kind[h] === 0) return `linear-gradient(${doc.grad_angle[h]}deg,${ramp})`;
   if (doc.grad_kind[h] === 2)
      return `conic-gradient(from ${doc.grad_angle[h]}deg at 50% 50%,${ramp})`;
   return `radial-gradient(circle ${fmt(Math.sqrt(w * w + hh * hh) / 2)}px at 50% 50%,${ramp})`;
}

// Segment curves matching the kernel's normalized lifts: every keyframe
// carries the exact `cubic-bezier(1/3, y1, 2/3, y2)` restriction of the
// whole-cycle Slab easing, so (1/3, 2/3) means the segment is linear.
const LINEAR_Y1 = 1 / 3;
const LINEAR_Y2 = 2 / 3;

// SLIR node kinds whose lifted color keyframes target distinct CSS channels.
const KIND_PATH = 12;
const NO_NODE = 0xffffffff;

type AnimationTarget = 'group' | 'paint';

/**
 * Declarations one lifted keyframe contributes. Offset and opacity belong to
 * the node-sized compositing wrapper so they move/fade every paint operation;
 * leaf-local rotate/scale and paint colors stay on their native element.
 */
function stopCss(
   anim: LiftedAnimation,
   stop: LiftedStop,
   last: boolean,
   target: AnimationTarget,
): string {
   let css = '';
   if (target === 'group') {
      if (stop.offset) {
         const [bx, by] = anim.base_offset;
         css += `translate:${fmt(stop.offset[0] - bx)}px ${fmt(stop.offset[1] - by)}px;`;
      }
      if (stop.opacity !== null) css += `opacity:${fmt(stop.opacity)};`;
   } else {
      if (stop.rotate !== null) css += `rotate:${fmt(stop.rotate - anim.base_rotate)}deg;`;
      if (stop.scale !== null) {
         const sx = stop.scale[0] / anim.base_scale[0];
         const sy = stop.scale[1] / anim.base_scale[1];
         css += sx === sy ? `scale:${fmt(sx)};` : `scale:${fmt(sx)} ${fmt(sy)};`;
      }
      if (stop.bg !== null) {
         css += `${anim.kind === KIND_PATH ? 'fill' : 'background-color'}:${rgbaCss(stop.bg)};`;
      }
      if (stop.color !== null) css += `color:${rgbaCss(stop.color)};`;
   }
   const [y1, y2] = stop.ctrl;
   if (!last && !(y1 === LINEAR_Y1 && y2 === LINEAR_Y2)) {
      css += `animation-timing-function:cubic-bezier(${fmt(LINEAR_Y1)},${fmt(y1)},${fmt(LINEAR_Y2)},${fmt(y2)});`;
   }
   return css;
}

function animationDeclaration(items: Map<number, string[]>): Map<number, string> {
   const result = new Map<number, string>();
   for (const [node, animations] of items) {
      result.set(node, `animation:${animations.join(',')};`);
   }
   return result;
}

/**
 * Translates normalized lifts into CSS. Group tracks own offset/opacity;
 * paint tracks own rotate/scale/colors. Splitting a binding preserves the same
 * timing and segment curves on both targets while avoiding per-frame DOM
 * writes and per-paint opacity multiplication.
 */
export function liftedAnimationCss(lifted: LiftedAnimation[]): {
   rules: string;
   byNode: Map<number, string>;
   byGroup: Map<number, string>;
} {
   let rules = '';
   const perNode = new Map<number, string[]>();
   const perGroup = new Map<number, string[]>();
   for (const anim of lifted) {
      const first = anim.stops[0];
      if (!first) continue;
      const count = anim.mode === 1 ? '1' : 'infinite';
      const direction = anim.mode === 2 ? 'alternate' : 'normal';
      const timing = `${anim.dur}ms linear ${anim.delay}ms ${count} ${direction} both`;
      const targets: [AnimationTarget, boolean, Map<number, string[]>][] = [
         ['group', first.offset !== null || first.opacity !== null, perGroup],
         [
            'paint',
            first.rotate !== null ||
               first.scale !== null ||
               first.bg !== null ||
               first.color !== null,
            perNode,
         ],
      ];
      for (const [target, present, declarations] of targets) {
         if (!present) continue;
         const name = `slab-b${anim.binding}-${target === 'group' ? 'g' : 'p'}`;
         let frames = '';
         for (let i = 0; i < anim.stops.length; i++) {
            const last = i === anim.stops.length - 1;
            frames += `${fmt(anim.stops[i].pos * 100)}%{${stopCss(anim, anim.stops[i], last, target)}}`;
         }
         rules += `@keyframes ${name}{${frames}}`;
         const items = declarations.get(anim.node) ?? [];
         items.push(`${timing} ${name}`);
         declarations.set(anim.node, items);
      }
   }
   return {
      rules,
      byNode: animationDeclaration(perNode),
      byGroup: animationDeclaration(perGroup),
   };
}

const OBJECT_FIT = ['cover', 'contain', 'fill'];
const SVG_NS = 'http://www.w3.org/2000/svg';
/** Last-applied cssText, tagged on the element (skip redundant style writes). */
const kCss = Symbol('slab.css');
/** Last innerHTML written, tagged on the element (skip redundant DOM writes). */
const kMarkup = Symbol('slab.markup');

interface StyledElement extends Element {
   [kCss]?: string;
   [kMarkup]?: string;
   style: CSSStyleDeclaration;
}

function isStyledElement(el: Element): el is StyledElement {
   return el instanceof HTMLElement || el instanceof SVGElement;
}

function setCss(el: Element, css: string): void {
   if (!isStyledElement(el))
      throw new Error('painter created an element without a CSS style declaration');
   if (el[kCss] !== css) {
      el.style.cssText = css;
      el[kCss] = css;
   }
}

function setMarkup(el: Element, markup: string): void {
   if (!isStyledElement(el))
      throw new Error('painter created an element without a CSS style declaration');
   if (el[kMarkup] !== markup) {
      el.innerHTML = markup;
      el[kMarkup] = markup;
   }
}

/** Narrows to the state-preserving atomic-move API (Chrome 133+). */
function hasMoveBefore(
   parent: Element,
): parent is Element & { moveBefore(node: Node, child: Node | null): void } {
   return 'moveBefore' in parent;
}

interface Layer {
   el: Element;
   ox: number;
   oy: number;
   /** Last element placed in this layer; the next one belongs after it. */
   prev: Element | null;
}

type PathBox = readonly [x: number, y: number, width: number, height: number];

interface PaintedPath {
   d: string;
   box: PathBox;
}

function buildPath(
   verbs: readonly number[],
   coords: readonly number[],
   verbOffset = 0,
   verbLength = verbs.length,
   coordOffset = 0,
   coordLength = coords.length,
): PaintedPath {
   let minx = Infinity;
   let miny = Infinity;
   let maxx = -Infinity;
   let maxy = -Infinity;
   const coordEnd = coordOffset + coordLength;
   for (let index = coordOffset; index < coordEnd; index += 2) {
      const x = coords[index];
      const y = coords[index + 1];
      if (x < minx) minx = x;
      if (y < miny) miny = y;
      if (x > maxx) maxx = x;
      if (y > maxy) maxy = y;
   }
   const box: PathBox =
      minx === Infinity
         ? [0, 0, 1, 1]
         : [minx, miny, Math.max(1, maxx - minx), Math.max(1, maxy - miny)];

   let d = '';
   let coordIndex = coordOffset;
   const verbEnd = verbOffset + verbLength;
   for (let index = verbOffset; index < verbEnd; index++) {
      switch (verbs[index]) {
         case 0:
            d += `M${coords[coordIndex]} ${coords[coordIndex + 1]}`;
            coordIndex += 2;
            break;
         case 1:
            d += `L${coords[coordIndex]} ${coords[coordIndex + 1]}`;
            coordIndex += 2;
            break;
         case 2:
            d += `C${coords[coordIndex]} ${coords[coordIndex + 1]} ${coords[coordIndex + 2]} ${coords[coordIndex + 3]} ${coords[coordIndex + 4]} ${coords[coordIndex + 5]}`;
            coordIndex += 6;
            break;
         case 3:
            d += `Q${coords[coordIndex]} ${coords[coordIndex + 1]} ${coords[coordIndex + 2]} ${coords[coordIndex + 3]}`;
            coordIndex += 4;
            break;
         default:
            d += 'Z';
      }
   }
   return { d, box };
}

interface RuntimeBitmap {
   generation: number;
   bitmap: ImageBitmap | null;
}

/** Retained-mode DOM painter for decoded kernel frames. */
export class Painter {
   readonly doc: Statics;
   /** Indexed by SLIR FONT table; filled by the element after FontFace load. */
   fonts: (FontCss | null)[] = [];
   /** Object URLs per IMGS entry (null = not embedded; falls back to src). */
   imageUrls: (string | null)[] = [];
   /** Paint-element `animation:` declaration per lifted node. */
   animations = new Map<number, string>();
   /** Compositing-group `animation:` declaration per lifted node. */
   groupAnimations = new Map<number, string>();
   /** Reads unified runtime image metadata from the owning element. */
   imageInfo: ((image: number) => ImageInfo | null) | null = null;
   /** Copies encoded or raw runtime image bytes from the owning element. */
   imageBytes: ((image: number) => Uint8Array) | null = null;
   /** Requests a repaint after asynchronous runtime image decoding. */
   invalidate: (() => void) | null = null;

   #root: HTMLElement;
   #keyed = new Map<string, Element>();
   #paths: (PaintedPath | undefined)[] = [];
   #runtimeImages = new Map<number, RuntimeBitmap>();
   /** Data-URI grain tiles keyed by quantized `amount:size` (see #grainTile). */
   #grainTiles = new Map<string, string>();

   constructor(doc: Statics, root: HTMLElement) {
      this.doc = doc;
      this.#root = root;
   }

   #compiledPath(path: number): PaintedPath {
      const cached = this.#paths[path];
      if (cached !== undefined) return cached;
      const doc = this.doc;
      const built = buildPath(
         doc.path_verbs,
         doc.path_coords,
         doc.path_verb_off[path],
         doc.path_verb_len[path],
         doc.path_coord_off[path],
         doc.path_coord_len[path],
      );
      this.#paths[path] = built;
      return built;
   }

   #runtimeBitmap(image: number, info: ImageInfo): ImageBitmap | null {
      const generation = info[3];
      const cached = this.#runtimeImages.get(image);
      if (cached?.generation === generation) return cached.bitmap;
      cached?.bitmap?.close();
      const entry: RuntimeBitmap = {
         generation,
         bitmap: null,
      };
      this.#runtimeImages.set(image, entry);
      const bytes = this.imageBytes?.(image);
      if (
         !bytes ||
         bytes.byteLength === 0 ||
         info[0] <= 0 ||
         info[1] <= 0 ||
         typeof createImageBitmap !== 'function'
      ) {
         return null;
      }
      let source: ImageBitmapSource;
      if (info[2] === 0) {
         source = new Blob([bytes.slice().buffer as ArrayBuffer], { type: 'image/png' });
      } else if (info[2] === 1 && bytes.byteLength === info[0] * info[1] * 4) {
         const rgba = new Uint8ClampedArray(bytes.byteLength);
         rgba.set(bytes);
         source = new ImageData(rgba, info[0], info[1]);
      } else {
         return null;
      }
      void createImageBitmap(source)
         .then((bitmap) => {
            if (this.#runtimeImages.get(image) !== entry) {
               bitmap.close();
               return;
            }
            entry.bitmap = bitmap;
            this.invalidate?.();
         })
         .catch(() => undefined);
      return null;
   }

   #drawRuntimeImage(
      canvas: HTMLCanvasElement,
      image: number,
      info: ImageInfo,
      fit: number,
      width: number,
      height: number,
   ): void {
      const ratio = globalThis.devicePixelRatio || 1;
      const pixelWidth = Math.max(1, Math.round(width * ratio));
      const pixelHeight = Math.max(1, Math.round(height * ratio));
      if (canvas.width !== pixelWidth) canvas.width = pixelWidth;
      if (canvas.height !== pixelHeight) canvas.height = pixelHeight;
      const context = canvas.getContext('2d');
      if (!context) return;
      context.setTransform(ratio, 0, 0, ratio, 0, 0);
      context.clearRect(0, 0, width, height);
      const bitmap = this.#runtimeBitmap(image, info);
      if (!bitmap || info[0] <= 0 || info[1] <= 0 || width <= 0 || height <= 0) return;
      if (fit === 2) {
         context.drawImage(bitmap, 0, 0, width, height);
         return;
      }
      const scale =
         fit === 1
            ? Math.min(width / info[0], height / info[1])
            : Math.max(width / info[0], height / info[1]);
      const drawnWidth = info[0] * scale;
      const drawnHeight = info[1] * scale;
      context.drawImage(
         bitmap,
         (width - drawnWidth) / 2,
         (height - drawnHeight) / 2,
         drawnWidth,
         drawnHeight,
      );
   }

   /** Drops one decoded runtime image immediately after host unregistration. */
   releaseImage(image: number): void {
      this.#runtimeImages.get(image)?.bitmap?.close();
      this.#runtimeImages.delete(image);
   }

   /** Releases browser image resources owned by this painter. */
   dispose(): void {
      for (const entry of this.#runtimeImages.values()) entry.bitmap?.close();
      this.#runtimeImages.clear();
      this.invalidate = null;
   }

   /**
    * Data-URI PNG grain tile (contract §6.2): 128×128 cells, one pixel per
    * cell — white when the hash is positive, black otherwise, alpha
    * `amount·|h|`. Callers quantize `amount` to 1/255 and `size` to 0.25
    * steps so near-identical frames share tiles; `size` never changes the
    * pixels (it only scales the tile via background-size / pattern units)
    * but keys the cache alongside amount.
    */
   #grainTile(amount: number, size: number): string {
      const key = `${amount}:${size}`;
      const cached = this.#grainTiles.get(key);
      if (cached !== undefined) return cached;
      const cells = 128;
      const canvas = document.createElement('canvas');
      canvas.width = cells;
      canvas.height = cells;
      const context = canvas.getContext('2d');
      let url = '';
      if (context) {
         const image = context.createImageData(cells, cells);
         const data = image.data;
         for (let j = 0; j < cells; j++) {
            for (let i = 0; i < cells; i++) {
               const signed = (2 * pcg2d(i, j)) / 4294967296 - 1;
               const at = (j * cells + i) * 4;
               const lum = signed > 0 ? 255 : 0;
               data[at] = lum;
               data[at + 1] = lum;
               data[at + 2] = lum;
               data[at + 3] = Math.round(amount * Math.abs(signed) * 255);
            }
         }
         context.putImageData(image, 0, 0);
         url = canvas.toDataURL('image/png');
      }
      this.#grainTiles.set(key, url);
      return url;
   }

   /**
    * Rect with corner smoothing: the retained element switches to inline SVG
    * (CSS border-radius cannot trace a squircle). Fill, grain, and stroke are
    * child paths; outer shadows ride a CSS drop-shadow chain. A conic fill —
    * which SVG cannot express — paints on `conicBgEl`, a sibling div carrying
    * `conic-gradient` clipped by the squircle path (shadows follow it there,
    * since `filter` applies after `clip-path`). Chart-noted degradations:
    * inset shadows are dropped, shadow spread is lost, and side masks stroke
    * the full perimeter.
    */
   #squircleRect(el: Element, o: OpRect, ox: number, oy: number, conicBgEl: Element | null): void {
      const doc = this.doc;
      const d = squirclePathD(o.w, o.h, o.radius, o.smooth);
      let defs = '';
      let body = '';
      let shadowHost: 'svg' | 'bg' = 'svg';
      if (conicBgEl !== null) {
         let bgCss = `position:absolute;left:${o.x - ox}px;top:${o.y - oy}px;width:${o.w}px;height:${o.h}px;clip-path:path('${d}');background-image:${gradientCss(doc, o.bg, o.w, o.h)};`;
         if (o.opacity !== 1) bgCss += `opacity:${o.opacity};`;
         bgCss += this.#dropShadowCss(o);
         setCss(conicBgEl, bgCss);
         shadowHost = 'bg';
      } else if (o.bg_kind !== 0) {
         let fill: string | null;
         if (o.bg_kind === 2 && doc.grad_kind[o.bg] !== 2) {
            defs += svgGradientDef(doc, o.bg, `sq${o.node}f`, [0, 0, o.w, o.h]);
            fill = `url(#sq${o.node}f)`;
         } else {
            fill = strokeCss(doc, o.bg_kind, o.bg);
         }
         if (fill !== null) body += `<path d="${d}" fill="${fill}"/>`;
      }
      if (o.grain_amount > 0) {
         const amount = Math.round(Math.min(Math.max(o.grain_amount, 0), 1) * 255) / 255;
         const size = Math.max(0.25, Math.round(o.grain_size * 4) / 4);
         const tile = 128 * size;
         defs += `<pattern id="sq${o.node}g" patternUnits="userSpaceOnUse" width="${tile}" height="${tile}"><image href="${this.#grainTile(amount, size)}" width="${tile}" height="${tile}" style="image-rendering:pixelated"/></pattern>`;
         body += `<path d="${d}" fill="url(#sq${o.node}g)"/>`;
      }
      if (o.stroke_kind !== 0 && o.stroke_w > 0) {
         const inset =
            o.stroke_align === 1 ? o.stroke_w / 2 : o.stroke_align === 2 ? -o.stroke_w / 2 : 0;
         const sw = o.w - 2 * inset;
         const sh = o.h - 2 * inset;
         const sd = squirclePathD(sw, sh, Math.max(0, o.radius - inset), o.smooth);
         let paint: string | null;
         if (o.stroke_kind === 2 && doc.grad_kind[o.stroke] !== 2) {
            defs += svgGradientDef(doc, o.stroke, `sq${o.node}s`, [0, 0, sw, sh]);
            paint = `url(#sq${o.node}s)`;
         } else {
            paint = strokeCss(doc, o.stroke_kind, o.stroke);
         }
         if (paint !== null) {
            const dash = o.has_dash
               ? ` stroke-dasharray="${fmt(o.dash_on)} ${fmt(o.dash_off)}"`
               : '';
            body += `<path d="${sd}" transform="translate(${fmt(inset)} ${fmt(inset)})" fill="none" stroke="${paint}" stroke-width="${fmt(o.stroke_w)}"${dash}/>`;
         }
      }
      setMarkup(el, (defs !== '' ? `<defs>${defs}</defs>` : '') + body);
      let css = `position:absolute;left:${o.x - ox}px;top:${o.y - oy}px;width:${o.w}px;height:${o.h}px;overflow:visible;`;
      if (shadowHost === 'svg') css += this.#dropShadowCss(o);
      if (o.opacity !== 1) css += `opacity:${o.opacity};`;
      css += this.animations.get(o.node) ?? '';
      setCss(el, css);
   }

   /** Outset shadows as a CSS `filter: drop-shadow(…)` chain (inset skipped). */
   #dropShadowCss(o: OpRect): string {
      if (o.shadow_len <= 0) return '';
      const doc = this.doc;
      const parts: string[] = [];
      for (let i = 0; i < o.shadow_len; i++) {
         const k = o.shadow_off + i;
         if (doc.shdw_inset[k] !== 0) continue;
         parts.push(
            `drop-shadow(${doc.shdw_x[k]}px ${doc.shdw_y[k]}px ${doc.shdw_blur[k]}px ${rgbaCss(doc.shdw_rgba[k])})`,
         );
      }
      return parts.length > 0 ? `filter:${parts.join(' ')};` : '';
   }

   paint(fr: Frame): void {
      const doc = this.doc;
      const used = new Set<string>();
      const counts = new Map<string, number>();
      const stack: Layer[] = [{ el: this.#root, ox: 0, oy: 0, prev: null }];
      const runtimePaths: (PaintedPath | undefined)[] = [];

      const take = (base: string, tag: 'canvas' | 'div' | 'span' | 'img' | 'svg'): Element => {
         const n = counts.get(base) ?? 0;
         counts.set(base, n + 1);
         const key = `${base}#${n}`;
         let el = this.#keyed.get(key);
         if (el && el.localName !== tag) {
            // The retained key switched element kind (rect div ⇄ squircle
            // svg): recreate — keyed reuse assumes stable tags.
            el.remove();
            el = undefined;
         }
         if (!el) {
            if (tag === 'svg') {
               el = document.createElementNS(SVG_NS, 'svg');
               el.appendChild(document.createElementNS(SVG_NS, 'path'));
            } else {
               el = document.createElement(tag);
               if (el instanceof HTMLImageElement) el.draggable = false;
            }
            this.#keyed.set(key, el);
         }
         used.add(key);
         const layer = stack[stack.length - 1];
         const expected = layer.prev ? layer.prev.nextSibling : layer.el.firstChild;
         if (el !== expected) {
            if (el.isConnected && hasMoveBefore(layer.el)) layer.el.moveBefore(el, expected);
            else layer.el.insertBefore(el, expected);
         }
         layer.prev = el;
         return el;
      };

      for (const op of fr.ops) {
         const { ox, oy } = stack[stack.length - 1];
         switch (op.tag) {
            case 'Rect': {
               const o = op.v;
               if (o.smooth > 0 && o.radius > 0) {
                  // A conic fill needs a CSS-painted sibling (SVG has no
                  // conic primitive); it precedes the svg so grain and
                  // strokes stay on top.
                  const conicBg =
                     o.bg_kind === 2 && doc.grad_kind[o.bg] === 2
                        ? take(`Rc${o.node}`, 'div')
                        : null;
                  this.#squircleRect(take(`R${o.node}`, 'svg'), o, ox, oy, conicBg);
                  break;
               }
               const el = take(`R${o.node}`, 'div');
               let css = `position:absolute;box-sizing:border-box;left:${o.x - ox}px;top:${o.y - oy}px;width:${o.w}px;height:${o.h}px;`;
               if (o.radius > 0) css += `border-radius:${o.radius}px;`;
               // Fill and grain share one background stack; grain is the TOP
               // layer (contract §6.2 — CSS inset shadows still paint above).
               const layers: string[] = [];
               let grainSize = '';
               if (o.grain_amount > 0) {
                  const amount = Math.round(Math.min(Math.max(o.grain_amount, 0), 1) * 255) / 255;
                  const size = Math.max(0.25, Math.round(o.grain_size * 4) / 4);
                  layers.push(`url("${this.#grainTile(amount, size)}")`);
                  grainSize = `${128 * size}px ${128 * size}px`;
               }
               if (o.bg_kind === 1) css += `background-color:${rgbaCss(o.bg)};`;
               else if (o.bg_kind === 2) layers.push(gradientCss(doc, o.bg, o.w, o.h));
               if (layers.length > 0) {
                  css += `background-image:${layers.join(',')};`;
                  if (grainSize !== '') {
                     css += `background-size:${grainSize}${layers.length > 1 ? ',auto' : ''};background-origin:border-box;image-rendering:pixelated;`;
                  }
               }
               const ring =
                  o.stroke_kind === 2 && o.stroke_w > 0 && o.stroke_sides === 15 && !o.has_dash;
               const stroke = ring ? null : strokeCss(doc, o.stroke_kind, o.stroke);
               if (stroke !== null && o.stroke_w > 0) {
                  const style = o.has_dash ? 'dashed' : 'solid';
                  if (o.stroke_sides !== 15) {
                     // a side mask always renders as CSS border sides — the
                     // sub-pixel align nuance is invisible, losing sides isn't
                     const edge = `${o.stroke_w}px ${style} ${stroke}`;
                     if (o.stroke_sides & 1) css += `border-top:${edge};`;
                     if (o.stroke_sides & 2) css += `border-right:${edge};`;
                     if (o.stroke_sides & 4) css += `border-bottom:${edge};`;
                     if (o.stroke_sides & 8) css += `border-left:${edge};`;
                  } else if (o.stroke_align === 1) {
                     css += `border:${o.stroke_w}px ${style} ${stroke};`;
                  } else {
                     // center (0) / outside (2) → outline
                     css += `outline:${o.stroke_w}px ${style} ${stroke};`;
                     if (o.stroke_align === 0) css += `outline-offset:${-o.stroke_w / 2}px;`;
                  }
               }
               if (o.shadow_len > 0) {
                  const parts: string[] = [];
                  for (let i = 0; i < o.shadow_len; i++) {
                     const k = o.shadow_off + i;
                     const inset = doc.shdw_inset[k] !== 0 ? ' inset' : '';
                     parts.push(
                        `${doc.shdw_x[k]}px ${doc.shdw_y[k]}px ${doc.shdw_blur[k]}px ${doc.shdw_spread[k]}px ${rgbaCss(doc.shdw_rgba[k])}${inset}`,
                     );
                  }
                  css += `box-shadow:${parts.join(',')};`;
               }
               if (o.opacity !== 1) css += `opacity:${o.opacity};`;
               css += this.animations.get(o.node) ?? '';
               setCss(el, css);
               if (ring) {
                  // W3: full-side non-dashed gradient strokes render as a real
                  // gradient ring — a masked sibling div keyed `S<node>`,
                  // dropped by the retained sweep when the stroke goes away.
                  const grow =
                     o.stroke_align === 1 ? 0 : o.stroke_align === 0 ? o.stroke_w / 2 : o.stroke_w;
                  const rw = o.w + 2 * grow;
                  const rh = o.h + 2 * grow;
                  const mask = 'linear-gradient(#fff 0 0) content-box,linear-gradient(#fff 0 0)';
                  let rcss = `position:absolute;box-sizing:border-box;left:${o.x - ox - grow}px;top:${o.y - oy - grow}px;width:${rw}px;height:${rh}px;padding:${o.stroke_w}px;background-image:${gradientCss(doc, o.stroke, rw, rh)};`;
                  if (o.radius > 0) rcss += `border-radius:${o.radius + grow}px;`;
                  rcss += `mask:${mask};-webkit-mask:${mask};mask-composite:exclude;-webkit-mask-composite:xor;`;
                  if (o.opacity !== 1) rcss += `opacity:${o.opacity};`;
                  rcss += this.animations.get(o.node) ?? '';
                  setCss(take(`S${o.node}`, 'div'), rcss);
               }
               break;
            }
            case 'Text': {
               const o = op.v;
               const el = take(`T${o.node}`, 'span');
               const f = o.font >= 0 ? this.fonts[o.font] : null;
               // FRAME.md half-leading model with a driver-chosen line box of
               // exactly (asc-desc)·size/upem: half-leading is zero, so the
               // baseline sits at asc·size/upem below the span top. The
               // FontFace ascent/descent overrides pin the browser to the
               // same hhea metrics the kernel measured with.
               const asc = f ? (f.ascent * o.size) / f.upem : o.size * 0.8;
               const lineH = f ? ((f.ascent - f.descent) * o.size) / f.upem : o.size;
               let css = `position:absolute;white-space:pre;left:${o.x - ox}px;top:${o.y_baseline - oy - asc}px;line-height:${lineH}px;font-family:${f ? f.family : 'sans-serif'};font-size:${o.size}px;font-weight:${o.weight};`;
               if (o.color_kind === 2) {
                  // W5 gradient text: the paint spans the node's content box
                  // (contract §6.7), offset per line from the span's own
                  // origin so multi-line headlines stay continuous.
                  const image = gradientCss(doc, o.color, o.gw, o.gh);
                  css += `background-image:${image};background-size:${o.gw}px ${o.gh}px;background-repeat:no-repeat;background-position:${o.gx - o.x}px ${o.gy - (o.y_baseline - asc)}px;-webkit-background-clip:text;background-clip:text;color:transparent;`;
               } else {
                  css += `color:${rgbaCss(o.color)};`;
               }
               if (o.tracking !== 0) css += `letter-spacing:${o.tracking}px;`;
               if (o.opacity !== 1) css += `opacity:${o.opacity};`;
               css += this.animations.get(o.node) ?? '';
               setCss(el, css);
               const text = fr.strings[o.str_ref];
               if (el.textContent !== text) el.textContent = text;
               break;
            }
            case 'Image': {
               const o = op.v;
               const runtime = o.img >= doc.img_src.length;
               const el = take(`${runtime ? 'J' : 'I'}${o.node}`, runtime ? 'canvas' : 'img');
               let css = `position:absolute;left:${o.x - ox}px;top:${o.y - oy}px;width:${o.w}px;height:${o.h}px;`;
               if (o.smooth > 0 && o.radius > 0)
                  css += `clip-path:path('${squirclePathD(o.w, o.h, o.radius, o.smooth)}');`;
               else if (o.radius > 0) css += `border-radius:${o.radius}px;`;
               if (o.opacity !== 1) css += `opacity:${o.opacity};`;
               css += this.animations.get(o.node) ?? '';
               if (runtime) {
                  if (!(el instanceof HTMLCanvasElement))
                     throw new Error('retained runtime image key does not refer to a canvas');
                  const info = this.imageInfo?.(o.img);
                  if (info) this.#drawRuntimeImage(el, o.img, info, o.fit, o.w, o.h);
               } else {
                  if (!(el instanceof HTMLImageElement))
                     throw new Error('retained image key does not refer to an image element');
                  const src =
                     o.img >= 0 ? (this.imageUrls[o.img] ?? doc.strs[doc.img_src[o.img]]) : '';
                  if (el.getAttribute('src') !== src) el.setAttribute('src', src);
                  css += `object-fit:${OBJECT_FIT[o.fit] ?? 'cover'};`;
               }
               setCss(el, css);
               break;
            }
            case 'PathDraw': {
               const o = op.v;
               const el = take(`P${o.node}`, 'svg');
               const pathEl = el.firstElementChild;
               if (!(pathEl instanceof SVGPathElement))
                  throw new Error('retained path key does not contain an SVG path element');
               let path: PaintedPath;
               if (o.path >= 0) {
                  path = this.#compiledPath(o.path);
               } else {
                  const index = ~o.path;
                  const source = fr.pathsRt[index];
                  if (!source) {
                     throw new Error(`runtime path reference ${o.path} is absent from this frame`);
                  }
                  path = runtimePaths[index] ?? buildPath(source.verbs, source.coords);
                  runtimePaths[index] = path;
               }
               if (pathEl.getAttribute('d') !== path.d) pathEl.setAttribute('d', path.d);
               const [bx, by, bw, bh] = path.box;
               if (el.getAttribute('viewBox') !== `${bx} ${by} ${bw} ${bh}`) {
                  el.setAttribute('viewBox', `${bx} ${by} ${bw} ${bh}`);
               }
               let css = `position:absolute;left:${o.dx + bx - ox}px;top:${o.dy + by - oy}px;width:${bw}px;height:${bh}px;overflow:visible;`;
               // W3: gradient path paints are real per-path defs mapped over
               // the path's coord bbox; conic paints keep their first stop
               // (SVG has no conic primitive).
               let defsMarkup = '';
               let fill: string | null = null;
               if (o.bg_kind === 2 && doc.grad_kind[o.bg] !== 2) {
                  defsMarkup += svgGradientDef(doc, o.bg, `pg${o.node}f`, path.box);
                  fill = `url(#pg${o.node}f)`;
               } else if (o.bg_kind !== 0) {
                  fill = strokeCss(doc, o.bg_kind, o.bg);
               }
               css += `fill:${fill ?? 'none'};`;
               let stroke: string | null = null;
               if (o.stroke_w > 0) {
                  if (o.stroke_kind === 2 && doc.grad_kind[o.stroke] !== 2) {
                     defsMarkup += svgGradientDef(doc, o.stroke, `pg${o.node}s`, path.box);
                     stroke = `url(#pg${o.node}s)`;
                  } else {
                     stroke = strokeCss(doc, o.stroke_kind, o.stroke);
                  }
               }
               if (stroke !== null) {
                  css += `stroke:${stroke};stroke-width:${o.stroke_w}px;`;
                  if (o.has_dash) css += `stroke-dasharray:${o.dash_on} ${o.dash_off};`;
               }
               if (o.opacity !== 1) css += `opacity:${o.opacity};`;
               css += this.animations.get(o.node) ?? '';
               setCss(el, css);
               const defsEl = pathEl.nextElementSibling;
               if (defsMarkup !== '') {
                  let target = defsEl;
                  if (!target) {
                     target = document.createElementNS(SVG_NS, 'defs');
                     el.appendChild(target);
                  }
                  setMarkup(target, defsMarkup);
               } else if (defsEl) {
                  defsEl.remove();
               }
               break;
            }
            case 'ClipPush': {
               const o = op.v;
               const el = take('C', 'div');
               let css = `position:absolute;left:${o.x - ox}px;top:${o.y - oy}px;width:${o.w}px;height:${o.h}px;overflow:hidden;`;
               if (o.smooth > 0 && o.radius > 0)
                  css += `clip-path:path('${squirclePathD(o.w, o.h, o.radius, o.smooth)}');`;
               else if (o.radius > 0) css += `border-radius:${o.radius}px;`;
               setCss(el, css);
               stack.push({ el, ox: o.x, oy: o.y, prev: null });
               break;
            }
            case 'GroupPush': {
               const o = op.v;
               const el = take(o.node === NO_NODE ? 'G' : `G${o.node}`, 'div');
               const animation = this.groupAnimations.get(o.node);
               const sized = o.mask_kind !== 0 || animation !== undefined;
               let originX = ox;
               let originY = oy;
               let css = 'position:absolute;left:0;top:0;width:0;height:0;overflow:visible;';
               if (sized) {
                  css = `position:absolute;left:${o.mx - ox}px;top:${o.my - oy}px;width:${o.mw}px;height:${o.mh}px;overflow:visible;`;
                  originX = o.mx;
                  originY = o.my;
               }
               if (o.mask_kind !== 0) {
                  // W7: the subtree's alpha is multiplied by the paint over
                  // the owning node's border box (contract §6.3); ink outside
                  // the box is masked to zero.
                  const paint =
                     o.mask_kind === 1
                        ? `linear-gradient(${rgbaCss(o.mask)} 0 0)`
                        : gradientCss(doc, o.mask, o.mw, o.mh);
                  css += `mask-image:${paint};-webkit-mask-image:${paint};mask-repeat:no-repeat;-webkit-mask-repeat:no-repeat;mask-size:100% 100%;-webkit-mask-size:100% 100%;`;
               }
               if (o.opacity !== 1) css += `opacity:${o.opacity};`;
               if (o.blur > 0) css += `filter:blur(${o.blur / 2}px);`;
               css += animation ?? '';
               setCss(el, css);
               stack.push({ el, ox: originX, oy: originY, prev: null });
               break;
            }
            case 'RotatePush': {
               const o = op.v;
               const el = take('O', 'div');
               const css = `position:absolute;left:0;top:0;width:0;height:0;overflow:visible;transform-origin:${o.cx - ox}px ${o.cy - oy}px;transform:rotate(${o.deg}deg);`;
               setCss(el, css);
               stack.push({ el, ox, oy, prev: null });
               break;
            }
            case 'ScalePush': {
               const o = op.v;
               const el = take('S', 'div');
               const css = `position:absolute;left:0;top:0;width:0;height:0;overflow:visible;transform-origin:${o.cx - ox}px ${o.cy - oy}px;transform:scale(${o.sx},${o.sy});`;
               setCss(el, css);
               stack.push({ el, ox, oy, prev: null });
               break;
            }
            case 'TiltPush': {
               // W10: ink-only perspective tilt (contract §6.5); the default
               // transform-style keeps the subtree flattened into one plane.
               const o = op.v;
               const el = take('V', 'div');
               const css = `position:absolute;left:0;top:0;width:0;height:0;overflow:visible;transform-origin:${o.cx - ox}px ${o.cy - oy}px;transform:perspective(${o.depth}px) rotateX(${o.rx}deg) rotateY(${o.ry}deg);`;
               setCss(el, css);
               stack.push({ el, ox, oy, prev: null });
               break;
            }
            case 'Backdrop': {
               const o = op.v;
               let shape = '';
               if (o.smooth > 0 && o.radius > 0)
                  shape = `clip-path:path('${squirclePathD(o.w, o.h, o.radius, o.smooth)}');`;
               else if (o.radius > 0) shape = `border-radius:${o.radius}px;`;
               const base = `position:absolute;left:${o.x - ox}px;top:${o.y - oy}px;width:${o.w}px;height:${o.h}px;${shape}`;
               if (o.mask_kind !== 0) {
                  // W9: six banded sibling divs approximate progressive blur
                  // (contract §6.6); the plain backdrop div is suppressed.
                  for (let band = 0; band < 6; band++) {
                     const el = take(`Db${band}`, 'div');
                     const alpha = (band + 0.5) / 6;
                     const parts: string[] = [];
                     if (o.blur > 0) parts.push(`blur(${(o.blur * alpha) / 2}px)`);
                     if (o.saturate !== 1) parts.push(`saturate(${1 + (o.saturate - 1) * alpha})`);
                     if (o.brightness !== 1)
                        parts.push(`brightness(${1 + (o.brightness - 1) * alpha})`);
                     let css = base;
                     if (parts.length > 0) {
                        const bf = parts.join(' ');
                        css += `backdrop-filter:${bf};-webkit-backdrop-filter:${bf};`;
                     }
                     const mask = bandMaskCss(doc, o.mask_kind, o.mask, band, 6, o.w, o.h);
                     css += `mask-image:${mask};-webkit-mask-image:${mask};mask-repeat:no-repeat;-webkit-mask-repeat:no-repeat;mask-size:100% 100%;-webkit-mask-size:100% 100%;`;
                     setCss(el, css);
                  }
                  break;
               }
               const el = take('D', 'div');
               let css = base;
               const parts: string[] = [];
               if (o.blur > 0) parts.push(`blur(${o.blur / 2}px)`);
               if (o.saturate !== 1) parts.push(`saturate(${o.saturate})`);
               if (o.brightness !== 1) parts.push(`brightness(${o.brightness})`);
               if (parts.length > 0) {
                  const bf = parts.join(' ');
                  css += `backdrop-filter:${bf};-webkit-backdrop-filter:${bf};`;
               }
               setCss(el, css);
               break;
            }
            case 'ClipPop':
            case 'GroupPop':
            case 'RotatePop':
            case 'ScalePop':
            case 'TiltPop':
               if (stack.length > 1) stack.pop();
               break;
            default: {
               const exhaustive: never = op;
               throw new Error(`unsupported frame operation ${String(exhaustive)}`);
            }
         }
      }

      for (const [key, el] of this.#keyed) {
         if (!used.has(key)) {
            el.remove();
            this.#keyed.delete(key);
         }
      }
   }
}
