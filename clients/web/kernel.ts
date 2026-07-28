/** A parameter value serialized by the kernel snapshot API. */
export interface ParamValue {
   kind: number;
   num: number;
   s: string;
   rgba: number;
   sym: string;
}

export interface ParamDef {
   name: string;
   ty: number;
   enum_symbols: string[];
}

export interface SignalDef {
   name: string;
   trigger: number;
}

export interface HoleDef {
   name: string;
   node: number;
   scroll: boolean;
}

export interface ListFieldDef {
   name: string;
   ty: number;
   enum_symbols: string[];
   default: ParamValue;
}

export interface ListDef {
   param: number;
   fields: ListFieldDef[];
}

/** Cold-path document tables returned by `KInst.statics_json()`. */
export interface Statics {
   strs: string[];
   font_family: number[];
   font_class: number[];
   font_weight: number[];
   font_upem: number[];
   font_ascent: number[];
   font_descent: number[];
   font_default_adv: number[];
   font_underline_position: number[];
   font_underline_thickness: number[];
   font_cmap_off: number[];
   font_cmap_len: number[];
   font_cmap_cp: number[];
   font_cmap_gid: number[];
   font_adv: number[];
   img_src: number[];
   grad_kind: number[];
   grad_angle: number[];
   grad_stop_off: number[];
   grad_stop_len: number[];
   grad_stop_pos: number[];
   grad_stop_rgba: number[];
   path_verb_off: number[];
   path_verb_len: number[];
   path_coord_off: number[];
   path_coord_len: number[];
   path_verbs: number[];
   path_coords: number[];
   shdw_x: number[];
   shdw_y: number[];
   shdw_blur: number[];
   shdw_spread: number[];
   shdw_rgba: number[];
   shdw_inset: number[];
   params: ParamDef[];
   signals: SignalDef[];
   holes: HoleDef[];
   lists: ListDef[];
}

/** One normalized keyframe of a lifted animation binding. Positions live in
 * the time domain; `ctrl` holds the exact `cubic-bezier(1/3, y1, 2/3, y2)`
 * y-controls for the segment leaving this stop. */
export interface LiftedStop {
   pos: number;
   ctrl: [number, number];
   offset: [number, number] | null;
   opacity: number | null;
   rotate: number | null;
   scale: [number, number] | null;
   bg: number | null;
   color: number | null;
}

/** A CSS-replayable animation binding from `KInst.lift_animations_json()`.
 * The kernel stops driving lifted bindings; the driver owns their playback.
 * Transform stops are absolute — replay is a delta against `base_rotate` /
 * `base_scale` / `base_offset`; color stops map onto the paint channel the
 * node `kind` uses (rect background, path fill, text ink). */
export interface LiftedAnimation {
   binding: number;
   node: number;
   kind: number;
   dur: number;
   delay: number;
   mode: number;
   base_offset: [number, number];
   base_rotate: number;
   base_scale: [number, number];
   stops: LiftedStop[];
}

/** Metadata captured for one emitted signal, parallel to `Effects.sig_name`. */
export interface SigMeta {
   x: number;
   y: number;
   dx: number;
   dy: number;
   drag_dx: number;
   drag_dy: number;
   mods: number;
   button: number;
   clicks: number;
   key: string;
   src_key: string;
   src_item: string;
   cancelled: boolean;
   dropped: boolean;
   /** Deepest hit-target canonical key on pointer-derived signals. */
   hit_key?: string;
   /** Pressed key name on keyboard-driven activation. */
   pressed_key?: string;
}

/** One retained scroll offset changed by a dispatch. */
export interface ScrollChange {
   key: string;
   /** `0` is the main axis and `1` is the cross axis. */
   axis: number;
   off: number;
}

/** One normalized inline-style run in a rich field. */
export interface FieldRun {
   /** `0` bold, `1` italic, `2` underline, `3` strike, or `4` code. */
   style: number;
   start: number;
   end: number;
}

/** Canonical rich-field payload parallel to a Change signal. */
export interface FieldRuns {
   rev: number;
   runs: FieldRun[];
}

/** One canonical endpoint in a host-owned cross-field edit. */
export interface RangeEndpoint {
   key: string;
   offset: number;
}

/** A host-owned edit deferred by the kernel for an active cross-field range. */
export interface RangeEdit {
   /** `0` text, `1` paste, `2` cut, `3` backspace, `4` delete, `5` IME, `6` copy. */
   kind: 0 | 1 | 2 | 3 | 4 | 5 | 6;
   anchor: RangeEndpoint;
   head: RangeEndpoint;
   text: string;
}

/** Event effects returned by the kernel dispatch snapshot API. */
export interface Effects {
   repaint: boolean;
   sig_name: number[];
   sig_text: string[];
   sig_runs: string[];
   sig_item: string[];
   sig_meta: SigMeta[];
   range_edit?: RangeEdit;
   has_caret: boolean;
   caret_x: number;
   caret_y: number;
   caret_w: number;
   caret_h: number;
   has_ime: boolean;
   ime_x: number;
   ime_y: number;
   ime_w: number;
   ime_h: number;
   cursor: number;
   focus: number;
}

export interface HoleRect {
   hole: number;
   x: number;
   y: number;
   w: number;
   h: number;
   clip: boolean;
}

/** Retained scene entry returned by `KInst.scene_json()`. */
export interface SceneNode {
   key: string;
   node: number;
   parent: number;
   kind: number;
   x: number;
   y: number;
   w: number;
   h: number;
   radius: number;
   rotation: number;
   cx: number;
   cy: number;
   flags: number;
   content_main: number;
   scroll_off: number;
   is_row: boolean;
   scroll: boolean;
   src_line: number;
   scroll_cross: number;
   content_cross: number;
   /** Resolved role, label, and description; empty strings mean absent. */
   role: string;
   label: string;
   desc: string;
   /** Optional authored checked/disclosure/selection states. */
   checked: boolean | 'mixed' | null;
   expanded: boolean | null;
   selected: boolean | null;
   /** Full stable scene keys for accessibility relationships, or empty. */
   active_descendant: string;
   controls: string;
   /** Optional numeric range and its human-readable value. */
   value_now: number | null;
   value_min: number | null;
   value_max: number | null;
   value_text: string;
   /** Optional dialog and live-region semantics. */
   modal: boolean | null;
   live: 'off' | 'polite' | 'assertive' | null;
   live_atomic: boolean | null;
   /** Optional hierarchy and collection position metadata. */
   level: number | null;
   pos_in_set: number | null;
   set_size: number | null;
   /** Effective kernel disabled and focus ownership for this frame. */
   disabled: boolean;
   focused: boolean;
   /** Whether the node carries an ACTIVE `field=` binder this frame. */
   editable: boolean;
   /** Painted subtree text in scene order (driver-annotated), lines joined with `\n`. */
   text: string;
}

export interface OpRect {
   node: number;
   x: number;
   y: number;
   w: number;
   h: number;
   radius: number;
   bg_kind: number;
   bg: number;
   stroke_kind: number;
   stroke: number;
   stroke_w: number;
   stroke_align: number;
   stroke_sides: number;
   dash_on: number;
   dash_off: number;
   has_dash: boolean;
   shadow_off: number;
   shadow_len: number;
   opacity: number;
   /** Corner smoothing 0..1 — squircle ink/clip when >0 and radius>0. */
   smooth: number;
   /** Grain speckle strength 0..1 (0 = off, contract §6.2). */
   grain_amount: number;
   /** Grain speckle cell size in logical units (1 when grain is off). */
   grain_size: number;
}

export interface OpText {
   node: number;
   x: number;
   y_baseline: number;
   str_ref: number;
   measured_w: number;
   font: number;
   size: number;
   weight: number;
   tracking: number;
   color: number;
   opacity: number;
   strike: boolean;
   /** Whether this run uses a real or synthesized oblique face. */
   italic: boolean;
   /** Whether this run paints an underline. */
   underline: boolean;
   /** Underline center offset below the baseline in layout units. */
   underline_offset: number;
   /** Underline thickness in layout units. */
   underline_thickness: number;
   /** Whether this operation shapes right-to-left. */
   rtl: boolean;
   /** Offset of this op's uncovered-run pairs in `Frame.uncovered`. */
   uncov_off: number;
   /** Number of uncovered runs (0 = every cluster covered by `font`). */
   uncov_len: number;
   /** Offset of this op's shaped glyphs in `Frame.glyphs`. */
   glyph_off: number;
   /** Number of shaped glyphs (0 = default/legacy per-codepoint advances). */
   glyph_len: number;
   /** 1 = solid (`color` is packed RGBA), 2 = gradient (`color` is a GRAD handle). */
   color_kind: number;
   /** Gradient box = the text node's content box (all 0 when `color_kind` is 1). */
   gx: number;
   gy: number;
   gw: number;
   gh: number;
}

export interface OpImage {
   node: number;
   x: number;
   y: number;
   w: number;
   h: number;
   img: number;
   fit: number;
   radius: number;
   opacity: number;
   /** Corner smoothing 0..1 — squircle clip when >0 and radius>0. */
   smooth: number;
}

export interface OpPath {
   node: number;
   dx: number;
   dy: number;
   path: number;
   bg_kind: number;
   bg: number;
   stroke_kind: number;
   stroke: number;
   stroke_w: number;
   dash_on: number;
   dash_off: number;
   has_dash: boolean;
   opacity: number;
}

export interface OpClip {
   x: number;
   y: number;
   w: number;
   h: number;
   radius: number;
   /** Corner smoothing 0..1 — squircle clip when >0 and radius>0. */
   smooth: number;
}
export interface FrameGlyph {
   font: number;
   gid: number;
   cluster: number;
   x: number;
   y: number;
   size: number;
}

export interface OpGroup {
   /** Document node owning this group; `0xffffffff` marks a host envelope. */
   node: number;
   opacity: number;
   blur: number;
   mask_kind: number;
   mask: number;
   /** Compositing box = the owning node's border box. */
   mx: number;
   my: number;
   mw: number;
   mh: number;
}

export interface OpRotate {
   cx: number;
   cy: number;
   deg: number;
}

/** Centered scale applied until the matching `ScalePop`. */
export interface OpScale {
   cx: number;
   cy: number;
   sx: number;
   sy: number;
}

export interface OpBackdrop {
   x: number;
   y: number;
   w: number;
   h: number;
   radius: number;
   blur: number;
   saturate: number;
   /** Backdrop RGB multiplier (1 = neutral). */
   brightness: number;
   /** Corner smoothing 0..1 — squircle clip when >0 and radius>0. */
   smooth: number;
   /** Progressive-blur mask paint: 0 = none, 1 = solid, 2 = gradient. */
   mask_kind: number;
   mask: number;
}

/** Ink-only 3D perspective tilt (CSS `perspective·rotateX·rotateY` about
 * `cx,cy`, contract §6.5) applied until the matching `TiltPop`. */
export interface OpTilt {
   cx: number;
   cy: number;
   rx: number;
   ry: number;
   depth: number;
}

export type FrameOp =
   | { tag: 'Rect'; v: OpRect }
   | { tag: 'Text'; v: OpText }
   | { tag: 'Image'; v: OpImage }
   | { tag: 'PathDraw'; v: OpPath }
   | { tag: 'ClipPush'; v: OpClip }
   | { tag: 'ClipPop' }
   | { tag: 'GroupPush'; v: OpGroup }
   | { tag: 'GroupPop' }
   | { tag: 'RotatePush'; v: OpRotate }
   | { tag: 'RotatePop' }
   | { tag: 'ScalePush'; v: OpScale }
   | { tag: 'ScalePop' }
   | { tag: 'Backdrop'; v: OpBackdrop }
   | { tag: 'TiltPush'; v: OpTilt }
   | { tag: 'TiltPop' };

/** One frame-local normalized path referenced by a negative `OpPath.path`. */
export interface RtPath {
   verbs: number[];
   coords: number[];
}

/** Immutable image metadata: width, height, format, and generation. */
export type ImageInfo = readonly [
   width: number,
   height: number,
   format: number,
   generation: number,
];

/** Immutable materialized range `[start, end)` for a virtual list. */
export type EachWindow = readonly [start: number, end: number];

/** One layout or runtime diagnostic produced by the current solve. */
export interface FrameDiagnostic {
   code: string;
   line: number;
   msg: string;
}

/** Decoded hot-path frame ready for the DOM painter. */
export interface Frame {
   width: number;
   height: number;
   ops: FrameOp[];
   strings: string[];
   pathsRt: RtPath[];
   dirty: boolean;
   motionActive: boolean;
   diagnostics: FrameDiagnostic[];
   /** Flat `[start, end)` codepoint-offset pairs indexed by `OpText.uncov_off`. */
   uncovered: Uint32Array;
   /** Shaped glyph pool indexed by `OpText.glyph_off`/`glyph_len`. */
   glyphs: FrameGlyph[];
}
