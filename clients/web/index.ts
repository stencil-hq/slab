// @stencil-hq/wslab — hand-written web driver over the Rust WASM kernel.
// `slab gen wc` modules import { SlabElement } and subclass it.

export {
   coerceParam,
   parseColor,
   type SignalMeta,
   type SlabDebugEntry,
   type SlabDiagnostic,
   type SlabDiagnosticsDetail,
   SlabElement,
   type SlabSignalDetail,
} from './element.ts';
export type { Frame, FrameDiagnostic, SceneNode, SignalDef, Statics } from './kernel.ts';
export { type FontCss, gradientCss, Painter, rgbaCss } from './painter.ts';
