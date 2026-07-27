// slab tui view — a live terminal client for the preview pane.
//
// The kernel rasterizes its solved frame into an 8×16 cell grid and serializes
// it as truecolor ANSI; that is what `slab render --client tui` prints and what
// slab-tui paints. This pane runs the same path frame by frame out of the SAME
// kernel instance the canvas drives, so motion animates and the terminal never
// drifts from the document.
//
// The surface is input-transparent on purpose. Cell (c, r) covers logical
// (8c…8c+8, 16r…16r+16), so the VT canvas is stretched onto exactly that box
// and pointer events fall through to the design overlay and the live element
// underneath: clicking, dragging, scrolling, and typing in TUI mode drive the
// one kernel both views share.

import type { SlabElement } from '@stencil-hq/wslab';
import type { Terminal } from 'ghostty-web';

/** Layout units per cell (`slab_kernel::cells::CW` / `CH`). */
const CELL_W = 8;
const CELL_H = 16;

/** Starting font size; boot rescales it so one glyph advance ≈ `CELL_W`. */
const FONT_SIZE = 12;

/** Repaint budget. A frame re-solves the kernel and re-parses a whole grid
 * through the VT, so the terminal runs at ~30fps while the canvas keeps 60. */
const MIN_FRAME_MS = 32;

/** Address a row (1-based) and erase it before its content is written. */
const rowHome = (row: number): string => `\x1b[${row};1H\x1b[K`;
/** Erase from the addressed row to the bottom of the screen. */
const eraseBelow = (row: number): string => `\x1b[${row};1H\x1b[J`;
/** The kernel paints its own caret cell — the VT cursor would double it. */
const HIDE_CURSOR = '\x1b[?25l';

/** Cyanotype terminal palette — the tokens style.css paints the chassis with. */
const THEME = {
   background: '#050508',
   foreground: '#a3a3ac',
   selectionBackground: '#0b2833',
};

export interface TuiHost {
   /** Terminal layer over the canvas stage; hidden while the canvas shows. */
   surface: HTMLDivElement;
   /** Pane meta slot — carries the grid size while the terminal is up. */
   meta: HTMLSpanElement;
   /** Live preview element; its kernel instance is the frame source. */
   view: SlabElement;
}

/** Preview-pane terminal, driven by main.ts's mode control and frame stream. */
export interface Tui {
   /** Show or hide the terminal; showing paints the current frame. */
   setActive(on: boolean): void;
   /** Paint the element's current frame — call after every canvas frame. */
   paint(): void;
}

/** Round half to even — the kernel's cell quantization (`cells::rhe`). */
function rhe(value: number): number {
   const floor = Math.floor(value);
   const fraction = value - floor;
   return fraction > 0.5 || (fraction === 0.5 && floor % 2 !== 0) ? floor + 1 : floor;
}

export function createTui(host: TuiHost): Tui {
   let booting: Promise<Terminal> | null = null;
   let term: Terminal | null = null;
   let active = false;
   /** Last grid written — repeat frames of a settled document cost nothing. */
   let painted = '';
   let lastPaint = 0;
   let trailing = 0;

   /** Stretch the VT's font cell onto the kernel's 8×16 one, so terminal
    * cells and document coordinates address the same pixels. Terminal fonts
    * are taller than wide by less than 2:1, which a real 8×16 terminal cell
    * corrects the same way. */
   function fitCell(): void {
      const metrics = term?.renderer?.getMetrics();
      if (!metrics || metrics.width <= 0 || metrics.height <= 0) return;
      host.surface.style.setProperty('--tui-sx', String(CELL_W / metrics.width));
      host.surface.style.setProperty('--tui-sy', String(CELL_H / metrics.height));
   }

   function resize(cols: number, rows: number): void {
      if (!term) return;
      term.resize(cols, rows);
      fitCell();
      host.meta.textContent = `${cols}×${rows} CELLS`;
      painted = '';
   }

   /** Solve, serialize, and overwrite the grid in place. */
   function draw(): void {
      const inst = host.view.instance;
      const frame = host.view.lastFrame;
      if (!inst || !frame) return;
      if (!term) {
         void boot();
         return;
      }
      const ansi = inst.cells_ansi(performance.now());
      const cols = Math.max(1, rhe(frame.width / CELL_W));
      const rows = Math.max(1, rhe(frame.height / CELL_H));
      if (term.cols !== cols || term.rows !== rows) resize(cols, rows);
      if (ansi === painted) return;
      painted = ansi;
      // Trailing blank rows collapse out of the ANSI; the grid keeps them, so
      // erase forward instead of scrolling a newline past the last row.
      // Each row is addressed absolutely and erased BEFORE its content: a
      // full-width row leaves the cursor on the last column in the deferred-
      // wrap state, where a trailing erase would eat the just-written cell
      // (the document's right border column).
      const body = ansi.endsWith('\n') ? ansi.slice(0, -1) : ansi;
      const lines = body.split('\n');
      let out = '';
      for (let i = 0; i < lines.length; i++) {
         out += rowHome(i + 1) + lines[i];
      }
      if (lines.length < term.rows) out += eraseBelow(lines.length + 1);
      term.write(out);
   }

   function paint(): void {
      if (!active) return;
      const wait = MIN_FRAME_MS - (performance.now() - lastPaint);
      if (wait > 0) {
         // Trailing edge: the frame that settles an animation must still land.
         if (trailing === 0) {
            trailing = window.setTimeout(() => {
               trailing = 0;
               paint();
            }, wait);
         }
         return;
      }
      lastPaint = performance.now();
      draw();
   }

   /** Boot the VT once: wasm, canvas, and a cell-matched font size.
    *
    * The emulator inlines its ~400KB wasm as a data URL, so it is imported
    * dynamically: the playground only pays for it when the pane is opened. */
   function boot(): Promise<Terminal> {
      booting ??= (async () => {
         const ghostty = await import('ghostty-web');
         await ghostty.init();
         const booted = new ghostty.Terminal({
            cursorBlink: false,
            disableStdin: true,
            fontSize: FONT_SIZE,
            fontFamily: '"Berkeley Mono", ui-monospace, "JetBrains Mono", "SF Mono", monospace',
            scrollback: 0,
            theme: THEME,
         });
         booted.open(host.surface);
         booted.write(HIDE_CURSOR);
         // A glyph advance of ~CELL_W keeps the cell stretch close to 1:1,
         // whatever font actually resolves on the visitor's machine.
         const metrics = booted.renderer?.getMetrics();
         if (metrics && metrics.width > 0) {
            booted.options.fontSize = Math.max(6, Math.round((FONT_SIZE * CELL_W) / metrics.width));
         }
         term = booted;
         fitCell();
         paint();
         return booted;
      })();
      return booting;
   }

   return {
      setActive(on: boolean): void {
         if (active === on) return;
         active = on;
         host.surface.hidden = !on;
         if (!on) {
            host.meta.textContent = '';
            return;
         }
         painted = '';
         paint();
      },
      paint,
   };
}
