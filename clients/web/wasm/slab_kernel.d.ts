/* tslint:disable */
/* eslint-disable */

/**
 * Binary frame payload decoded by `clients/web/frame-decode.ts`.
 *
 * The f64 stream starts with frame width and height. Each u32 operation tag
 * then selects fixed u32 and f64 payload arities, avoiding per-frame JSON
 * allocation for paint operations.
 */
export class FrameBuf {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Reports whether the solve dirtied the instance for another frame.
     */
    dirty(): boolean;
    /**
     * Returns frame dimensions followed by operation float payloads.
     */
    f64s(): Float64Array;
    /**
     * Reports whether animation or transition clocks remain active.
     */
    motion_active(): boolean;
    /**
     * Returns frame-local runtime paths as `[verbs, coords]` JSON pairs.
     */
    rt_paths_json(): string;
    /**
     * Returns the frame-local string pool as JSON.
     */
    strs_json(): string;
    /**
     * Returns operation tags and integer payloads.
     */
    u32s(): Uint32Array;
}

/**
 * One decoded, initialized kernel instance owned by JavaScript.
 */
export class KInst {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Solves once and reports capability degradation for a renderer client.
     *
     * Client indices follow `slab_kernel::caps::CLIENTS`; an invalid index is
     * rejected at the WASM boundary.
     */
    caps_report(time_ms: number, client: number): string;
    /**
     * Recomputes caret and IME geometry from the latest solve.
     */
    caret_effects_json(): string;
    /**
     * Solves once and emits the truecolor ANSI grid a terminal client paints,
     * caret included — the live counterpart of `slab render --client tui`.
     */
    cells_ansi(time_ms: number): string;
    /**
     * Solves once and emits plain TUI cells for conformance comparison.
     */
    cells_text(time_ms: number): string;
    /**
     * Returns one retained ancestor chain from root to target as JSON.
     */
    chain_json(scene_index: number): string;
    /**
     * Dispatches one platform event and emits canonical conformance effects JSON.
     */
    dispatch_dump_json(event_type: number, x: number, y: number, dx: number, dy: number, button: number, key: string, text: string, modifiers: number, clicks: number): string;
    /**
     * Dispatches one platform event and returns all effects as JSON.
     */
    dispatch_json(event_type: number, x: number, y: number, dx: number, dy: number, button: number, key: string, text: string, modifiers: number, clicks: number): string;
    /**
     * Returns a virtual list's materialized window as JSON.
     */
    each_window_json(each: string): string;
    /**
     * Registers runtime font metrics and returns the selected font-table index.
     */
    font_register(family: string, weight: number, upem: number, ascent: number, descent: number, line_gap: number, default_advance: number, codepoints: Uint32Array, glyphs: Uint32Array, advances: Uint32Array): number;
    /**
     * Solves and lowers one frame into compact typed streams.
     */
    frame(time_ms: number): FrameBuf;
    /**
     * Emits canonical `frame.json` for native/WASM conformance checks.
     */
    frame_json(time_ms: number): string;
    /**
     * Returns one keyed divider extent overlay.
     */
    get_divider(key: string): number;
    /**
     * Returns one retained scroll offset by node key and axis.
     */
    get_scroll(key: string, axis: number): number;
    /**
     * Tests a point against one retained scene index, including clips and rotations.
     */
    hit_contains(scene_index: number, x: number, y: number): boolean;
    /**
     * Runs retained-scene hit testing and emits its canonical conformance JSON.
     */
    hit_json(x: number, y: number): string;
    /**
     * Returns absolute hole rectangles for the current solve as JSON.
     */
    holes_json(): string;
    /**
     * Returns one embedded or runtime image payload by unified table index.
     */
    image_data(image: number): Uint8Array;
    /**
     * Returns image dimensions, format, and generation as JSON.
     */
    image_info_json(image: number): string;
    /**
     * Registers or replaces a named runtime image.
     */
    img_register(name: string, width: number, height: number, format: number, bytes: Uint8Array): number;
    /**
     * Unregisters one named runtime image.
     */
    img_unregister(name: string): boolean;
    /**
     * Marks every CSS-liftable animation binding driver-owned and returns
     * their normalized keyframes as JSON. The caller MUST replay them
     * (e.g. as CSS animations); lifted bindings no longer drive kernel
     * motion. Idempotent.
     */
    lift_animations_json(): string;
    /**
     * Returns the item count for a root or nested list.
     */
    list_len(param: number, path: string): number;
    /**
     * Decodes SLIR bytes and creates an initialized kernel instance.
     */
    constructor(slir: Uint8Array);
    /**
     * Scrolls ancestors minimally to reveal a keyed node.
     */
    reveal(key: string, margin: number): boolean;
    /**
     * Reveals one item in a virtual list.
     */
    reveal_item(each: string, index: number, align: number): boolean;
    /**
     * Returns retained scene geometry and resolved keys as JSON.
     */
    scene_json(): string;
    /**
     * Solves once and returns decoded-pool and frame-operation counts as JSON.
     */
    selftest_counts_json(time_ms: number): string;
    /**
     * Sets one keyed divider extent overlay.
     */
    set_divider(key: string, extent: number): boolean;
    /**
     * Updates viewport and client environment inputs.
     */
    set_env(vw: number, vh: number, client: number, dark: boolean, coarse: boolean): void;
    /**
     * Moves focus to a keyed focusable node; an empty key clears focus.
     */
    set_focus(key: string, visible: boolean): boolean;
    /**
     * Updates measured slot content for one hole.
     */
    set_hole_size(hole: number, width: number, height: number): void;
    /**
     * Assigns one typed list field.
     */
    set_list_field(param: number, path: string, index: number, field: string, kind: number, num: number, value: string, rgba: number, symbol: string): boolean;
    /**
     * Assigns one root or nested list item's stable key.
     */
    set_list_key(param: number, path: string, index: number, key: string): boolean;
    /**
     * Changes the item count for a root or nested list.
     */
    set_list_len(param: number, path: string, length: number): boolean;
    /**
     * Enables or disables one node-local state, returning false for an unknown key.
     */
    set_node_state(key: string, name: string, on: boolean): boolean;
    /**
     * Assigns one scalar parameter by document parameter index.
     */
    set_param(param: number, kind: number, num: number, value: string, rgba: number, symbol: string): boolean;
    /**
     * Changes one retained scroll offset by node key and axis.
     */
    set_scroll(key: string, axis: number, offset: number): boolean;
    /**
     * Enables or disables one document-level state by name.
     */
    set_state(name: string, on: boolean): void;
    /**
     * Selects a compiled theme by name.
     */
    set_theme(name: string): boolean;
    /**
     * Returns immutable document pools and host schemas as JSON.
     */
    statics_json(): string;
    /**
     * Drains settled-frame signals in the canonical conformance dump shape.
     */
    take_signals_dump_json(): string;
    /**
     * Drains signals queued while solving the preceding frame.
     */
    take_signals_json(): string;
    /**
     * Returns the current theme name.
     */
    theme(): string;
    /**
     * Emits the canonical summary of retained trace state.
     */
    trace_summary_json(): string;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_kinst_free: (a: number, b: number) => void;
    readonly kinst_caps_report: (a: number, b: number, c: number) => [number, number, number, number];
    readonly kinst_caret_effects_json: (a: number) => [number, number];
    readonly kinst_cells_ansi: (a: number, b: number) => [number, number];
    readonly kinst_cells_text: (a: number, b: number) => [number, number];
    readonly kinst_chain_json: (a: number, b: number) => [number, number];
    readonly kinst_dispatch_dump_json: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number) => [number, number];
    readonly kinst_dispatch_json: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number) => [number, number];
    readonly kinst_each_window_json: (a: number, b: number, c: number) => [number, number];
    readonly kinst_font_register: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number, o: number) => number;
    readonly kinst_frame: (a: number, b: number) => number;
    readonly kinst_frame_json: (a: number, b: number) => [number, number];
    readonly kinst_get_divider: (a: number, b: number, c: number) => number;
    readonly kinst_get_scroll: (a: number, b: number, c: number, d: number) => number;
    readonly kinst_hit_contains: (a: number, b: number, c: number, d: number) => number;
    readonly kinst_hit_json: (a: number, b: number, c: number) => [number, number];
    readonly kinst_holes_json: (a: number) => [number, number];
    readonly kinst_image_data: (a: number, b: number) => [number, number];
    readonly kinst_image_info_json: (a: number, b: number) => [number, number];
    readonly kinst_img_register: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => number;
    readonly kinst_img_unregister: (a: number, b: number, c: number) => number;
    readonly kinst_lift_animations_json: (a: number) => [number, number];
    readonly kinst_list_len: (a: number, b: number, c: number, d: number) => number;
    readonly kinst_new: (a: number, b: number) => [number, number, number];
    readonly kinst_reveal: (a: number, b: number, c: number, d: number) => number;
    readonly kinst_reveal_item: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly kinst_scene_json: (a: number) => [number, number];
    readonly kinst_selftest_counts_json: (a: number, b: number) => [number, number];
    readonly kinst_set_divider: (a: number, b: number, c: number, d: number) => number;
    readonly kinst_set_env: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
    readonly kinst_set_focus: (a: number, b: number, c: number, d: number) => number;
    readonly kinst_set_hole_size: (a: number, b: number, c: number, d: number) => void;
    readonly kinst_set_list_field: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number) => number;
    readonly kinst_set_list_key: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => number;
    readonly kinst_set_list_len: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly kinst_set_node_state: (a: number, b: number, c: number, d: number, e: number, f: number) => number;
    readonly kinst_set_param: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => number;
    readonly kinst_set_scroll: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly kinst_set_state: (a: number, b: number, c: number, d: number) => void;
    readonly kinst_set_theme: (a: number, b: number, c: number) => number;
    readonly kinst_statics_json: (a: number) => [number, number];
    readonly kinst_take_signals_dump_json: (a: number) => [number, number];
    readonly kinst_take_signals_json: (a: number) => [number, number];
    readonly kinst_theme: (a: number) => [number, number];
    readonly kinst_trace_summary_json: (a: number) => [number, number];
    readonly __wbg_framebuf_free: (a: number, b: number) => void;
    readonly framebuf_dirty: (a: number) => number;
    readonly framebuf_f64s: (a: number) => [number, number];
    readonly framebuf_motion_active: (a: number) => number;
    readonly framebuf_rt_paths_json: (a: number) => [number, number];
    readonly framebuf_strs_json: (a: number) => [number, number];
    readonly framebuf_u32s: (a: number) => [number, number];
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
