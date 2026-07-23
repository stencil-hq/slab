/* @ts-self-types="./slab_kernel.d.ts" */

/**
 * Binary frame payload decoded by `clients/web/frame-decode.ts`.
 *
 * The f64 stream starts with frame width and height. Each u32 operation tag
 * then selects fixed u32 and f64 payload arities, avoiding per-frame JSON
 * allocation for paint operations.
 */
export class FrameBuf {
    static __wrap(ptr) {
        const obj = Object.create(FrameBuf.prototype);
        obj.__wbg_ptr = ptr;
        FrameBufFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        FrameBufFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_framebuf_free(ptr, 0);
    }
    /**
     * Reports whether the solve dirtied the instance for another frame.
     * @returns {boolean}
     */
    dirty() {
        const ret = wasm.framebuf_dirty(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * Returns frame dimensions followed by operation float payloads.
     * @returns {Float64Array}
     */
    f64s() {
        const ret = wasm.framebuf_f64s(this.__wbg_ptr);
        var v1 = getArrayF64FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 8, 8);
        return v1;
    }
    /**
     * Reports whether animation or transition clocks remain active.
     * @returns {boolean}
     */
    motion_active() {
        const ret = wasm.framebuf_motion_active(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * Returns frame-local runtime paths as `[verbs, coords]` JSON pairs.
     * @returns {string}
     */
    rt_paths_json() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.framebuf_rt_paths_json(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Returns the frame-local string pool as JSON.
     * @returns {string}
     */
    strs_json() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.framebuf_strs_json(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Returns operation tags and integer payloads.
     * @returns {Uint32Array}
     */
    u32s() {
        const ret = wasm.framebuf_u32s(this.__wbg_ptr);
        var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
}
if (Symbol.dispose) FrameBuf.prototype[Symbol.dispose] = FrameBuf.prototype.free;

/**
 * One decoded, initialized kernel instance owned by JavaScript.
 */
export class KInst {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        KInstFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_kinst_free(ptr, 0);
    }
    /**
     * Solves once and reports capability degradation for a renderer client.
     *
     * Client indices follow `slab_kernel::caps::CLIENTS`; an invalid index is
     * rejected at the WASM boundary.
     * @param {number} time_ms
     * @param {number} client
     * @returns {string}
     */
    caps_report(time_ms, client) {
        let deferred2_0;
        let deferred2_1;
        try {
            const ret = wasm.kinst_caps_report(this.__wbg_ptr, time_ms, client);
            var ptr1 = ret[0];
            var len1 = ret[1];
            if (ret[3]) {
                ptr1 = 0; len1 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred2_0 = ptr1;
            deferred2_1 = len1;
            return getStringFromWasm0(ptr1, len1);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * Recomputes caret and IME geometry from the latest solve.
     * @returns {string}
     */
    caret_effects_json() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_caret_effects_json(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Solves once and emits the truecolor ANSI grid a terminal client paints,
     * caret included — the live counterpart of `slab render --client tui`.
     * @param {number} time_ms
     * @returns {string}
     */
    cells_ansi(time_ms) {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_cells_ansi(this.__wbg_ptr, time_ms);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Solves once and emits plain TUI cells for conformance comparison.
     * @param {number} time_ms
     * @returns {string}
     */
    cells_text(time_ms) {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_cells_text(this.__wbg_ptr, time_ms);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Returns one retained ancestor chain from root to target as JSON.
     * @param {number} scene_index
     * @returns {string}
     */
    chain_json(scene_index) {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_chain_json(this.__wbg_ptr, scene_index);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Dispatches one platform event and emits canonical conformance effects JSON.
     * @param {number} event_type
     * @param {number} x
     * @param {number} y
     * @param {number} dx
     * @param {number} dy
     * @param {number} button
     * @param {string} key
     * @param {string} text
     * @param {number} modifiers
     * @param {number} clicks
     * @returns {string}
     */
    dispatch_dump_json(event_type, x, y, dx, dy, button, key, text, modifiers, clicks) {
        let deferred3_0;
        let deferred3_1;
        try {
            const ptr0 = passStringToWasm0(key, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(text, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            const ret = wasm.kinst_dispatch_dump_json(this.__wbg_ptr, event_type, x, y, dx, dy, button, ptr0, len0, ptr1, len1, modifiers, clicks);
            deferred3_0 = ret[0];
            deferred3_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred3_0, deferred3_1, 1);
        }
    }
    /**
     * Dispatches one platform event and returns all effects as JSON.
     * @param {number} event_type
     * @param {number} x
     * @param {number} y
     * @param {number} dx
     * @param {number} dy
     * @param {number} button
     * @param {string} key
     * @param {string} text
     * @param {number} modifiers
     * @param {number} clicks
     * @returns {string}
     */
    dispatch_json(event_type, x, y, dx, dy, button, key, text, modifiers, clicks) {
        let deferred3_0;
        let deferred3_1;
        try {
            const ptr0 = passStringToWasm0(key, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ptr1 = passStringToWasm0(text, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            const ret = wasm.kinst_dispatch_json(this.__wbg_ptr, event_type, x, y, dx, dy, button, ptr0, len0, ptr1, len1, modifiers, clicks);
            deferred3_0 = ret[0];
            deferred3_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred3_0, deferred3_1, 1);
        }
    }
    /**
     * Returns a virtual list's materialized window as JSON.
     * @param {string} each
     * @returns {string}
     */
    each_window_json(each) {
        let deferred2_0;
        let deferred2_1;
        try {
            const ptr0 = passStringToWasm0(each, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ret = wasm.kinst_each_window_json(this.__wbg_ptr, ptr0, len0);
            deferred2_0 = ret[0];
            deferred2_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * Registers runtime font metrics and returns the selected font-table index.
     * @param {string} family
     * @param {number} weight
     * @param {number} upem
     * @param {number} ascent
     * @param {number} descent
     * @param {number} line_gap
     * @param {number} default_advance
     * @param {Uint32Array} codepoints
     * @param {Uint32Array} glyphs
     * @param {Uint32Array} advances
     * @returns {number}
     */
    font_register(family, weight, upem, ascent, descent, line_gap, default_advance, codepoints, glyphs, advances) {
        const ptr0 = passStringToWasm0(family, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passArray32ToWasm0(codepoints, wasm.__wbindgen_malloc);
        const len1 = WASM_VECTOR_LEN;
        const ptr2 = passArray32ToWasm0(glyphs, wasm.__wbindgen_malloc);
        const len2 = WASM_VECTOR_LEN;
        const ptr3 = passArray32ToWasm0(advances, wasm.__wbindgen_malloc);
        const len3 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_font_register(this.__wbg_ptr, ptr0, len0, weight, upem, ascent, descent, line_gap, default_advance, ptr1, len1, ptr2, len2, ptr3, len3);
        return ret;
    }
    /**
     * Solves and lowers one frame into compact typed streams.
     * @param {number} time_ms
     * @returns {FrameBuf}
     */
    frame(time_ms) {
        const ret = wasm.kinst_frame(this.__wbg_ptr, time_ms);
        return FrameBuf.__wrap(ret);
    }
    /**
     * Emits canonical `frame.json` for native/WASM conformance checks.
     * @param {number} time_ms
     * @returns {string}
     */
    frame_json(time_ms) {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_frame_json(this.__wbg_ptr, time_ms);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Returns one keyed divider extent overlay.
     * @param {string} key
     * @returns {number}
     */
    get_divider(key) {
        const ptr0 = passStringToWasm0(key, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_get_divider(this.__wbg_ptr, ptr0, len0);
        return ret;
    }
    /**
     * Returns one retained scroll offset by node key and axis.
     * @param {string} key
     * @param {number} axis
     * @returns {number}
     */
    get_scroll(key, axis) {
        const ptr0 = passStringToWasm0(key, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_get_scroll(this.__wbg_ptr, ptr0, len0, axis);
        return ret;
    }
    /**
     * Tests a point against one retained scene index, including clips and rotations.
     * @param {number} scene_index
     * @param {number} x
     * @param {number} y
     * @returns {boolean}
     */
    hit_contains(scene_index, x, y) {
        const ret = wasm.kinst_hit_contains(this.__wbg_ptr, scene_index, x, y);
        return ret !== 0;
    }
    /**
     * Runs retained-scene hit testing and emits its canonical conformance JSON.
     * @param {number} x
     * @param {number} y
     * @returns {string}
     */
    hit_json(x, y) {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_hit_json(this.__wbg_ptr, x, y);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Returns absolute hole rectangles for the current solve as JSON.
     * @returns {string}
     */
    holes_json() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_holes_json(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Returns one embedded or runtime image payload by unified table index.
     * @param {number} image
     * @returns {Uint8Array}
     */
    image_data(image) {
        const ret = wasm.kinst_image_data(this.__wbg_ptr, image);
        var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
        return v1;
    }
    /**
     * Returns image dimensions, format, and generation as JSON.
     * @param {number} image
     * @returns {string}
     */
    image_info_json(image) {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_image_info_json(this.__wbg_ptr, image);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Registers or replaces a named runtime image.
     * @param {string} name
     * @param {number} width
     * @param {number} height
     * @param {number} format
     * @param {Uint8Array} bytes
     * @returns {number}
     */
    img_register(name, width, height, format, bytes) {
        const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_img_register(this.__wbg_ptr, ptr0, len0, width, height, format, ptr1, len1);
        return ret;
    }
    /**
     * Unregisters one named runtime image.
     * @param {string} name
     * @returns {boolean}
     */
    img_unregister(name) {
        const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_img_unregister(this.__wbg_ptr, ptr0, len0);
        return ret !== 0;
    }
    /**
     * Marks every CSS-liftable animation binding driver-owned and returns
     * their normalized keyframes as JSON. The caller MUST replay them
     * (e.g. as CSS animations); lifted bindings no longer drive kernel
     * motion. Idempotent.
     * @returns {string}
     */
    lift_animations_json() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_lift_animations_json(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Returns the item count for a root or nested list.
     * @param {number} param
     * @param {string} path
     * @returns {number}
     */
    list_len(param, path) {
        const ptr0 = passStringToWasm0(path, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_list_len(this.__wbg_ptr, param, ptr0, len0);
        return ret;
    }
    /**
     * Decodes SLIR bytes and creates an initialized kernel instance.
     * @param {Uint8Array} slir
     */
    constructor(slir) {
        const ptr0 = passArray8ToWasm0(slir, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_new(ptr0, len0);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        this.__wbg_ptr = ret[0];
        KInstFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Scrolls ancestors minimally to reveal a keyed node.
     * @param {string} key
     * @param {number} margin
     * @returns {boolean}
     */
    reveal(key, margin) {
        const ptr0 = passStringToWasm0(key, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_reveal(this.__wbg_ptr, ptr0, len0, margin);
        return ret !== 0;
    }
    /**
     * Reveals one item in a virtual list.
     * @param {string} each
     * @param {number} index
     * @param {number} align
     * @returns {boolean}
     */
    reveal_item(each, index, align) {
        const ptr0 = passStringToWasm0(each, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_reveal_item(this.__wbg_ptr, ptr0, len0, index, align);
        return ret !== 0;
    }
    /**
     * Returns retained scene geometry and resolved keys as JSON.
     * @returns {string}
     */
    scene_json() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_scene_json(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Solves once and returns decoded-pool and frame-operation counts as JSON.
     * @param {number} time_ms
     * @returns {string}
     */
    selftest_counts_json(time_ms) {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_selftest_counts_json(this.__wbg_ptr, time_ms);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Sets one keyed divider extent overlay.
     * @param {string} key
     * @param {number} extent
     * @returns {boolean}
     */
    set_divider(key, extent) {
        const ptr0 = passStringToWasm0(key, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_set_divider(this.__wbg_ptr, ptr0, len0, extent);
        return ret !== 0;
    }
    /**
     * Updates viewport and client environment inputs.
     * @param {number} vw
     * @param {number} vh
     * @param {number} client
     * @param {boolean} dark
     * @param {boolean} coarse
     */
    set_env(vw, vh, client, dark, coarse) {
        wasm.kinst_set_env(this.__wbg_ptr, vw, vh, client, dark, coarse);
    }
    /**
     * Moves focus to a keyed focusable node; an empty key clears focus.
     * @param {string} key
     * @param {boolean} visible
     * @returns {boolean}
     */
    set_focus(key, visible) {
        const ptr0 = passStringToWasm0(key, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_set_focus(this.__wbg_ptr, ptr0, len0, visible);
        return ret !== 0;
    }
    /**
     * Updates measured slot content for one hole.
     * @param {number} hole
     * @param {number} width
     * @param {number} height
     */
    set_hole_size(hole, width, height) {
        wasm.kinst_set_hole_size(this.__wbg_ptr, hole, width, height);
    }
    /**
     * Assigns one typed list field.
     * @param {number} param
     * @param {string} path
     * @param {number} index
     * @param {string} field
     * @param {number} kind
     * @param {number} num
     * @param {string} value
     * @param {number} rgba
     * @param {string} symbol
     * @returns {boolean}
     */
    set_list_field(param, path, index, field, kind, num, value, rgba, symbol) {
        const ptr0 = passStringToWasm0(path, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(field, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ptr2 = passStringToWasm0(value, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len2 = WASM_VECTOR_LEN;
        const ptr3 = passStringToWasm0(symbol, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len3 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_set_list_field(this.__wbg_ptr, param, ptr0, len0, index, ptr1, len1, kind, num, ptr2, len2, rgba, ptr3, len3);
        return ret !== 0;
    }
    /**
     * Assigns one root or nested list item's stable key.
     * @param {number} param
     * @param {string} path
     * @param {number} index
     * @param {string} key
     * @returns {boolean}
     */
    set_list_key(param, path, index, key) {
        const ptr0 = passStringToWasm0(path, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(key, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_set_list_key(this.__wbg_ptr, param, ptr0, len0, index, ptr1, len1);
        return ret !== 0;
    }
    /**
     * Changes the item count for a root or nested list.
     * @param {number} param
     * @param {string} path
     * @param {number} length
     * @returns {boolean}
     */
    set_list_len(param, path, length) {
        const ptr0 = passStringToWasm0(path, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_set_list_len(this.__wbg_ptr, param, ptr0, len0, length);
        return ret !== 0;
    }
    /**
     * Enables or disables one node-local state, returning false for an unknown key.
     * @param {string} key
     * @param {string} name
     * @param {boolean} on
     * @returns {boolean}
     */
    set_node_state(key, name, on) {
        const ptr0 = passStringToWasm0(key, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_set_node_state(this.__wbg_ptr, ptr0, len0, ptr1, len1, on);
        return ret !== 0;
    }
    /**
     * Assigns one scalar parameter by document parameter index.
     * @param {number} param
     * @param {number} kind
     * @param {number} num
     * @param {string} value
     * @param {number} rgba
     * @param {string} symbol
     * @returns {boolean}
     */
    set_param(param, kind, num, value, rgba, symbol) {
        const ptr0 = passStringToWasm0(value, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passStringToWasm0(symbol, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len1 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_set_param(this.__wbg_ptr, param, kind, num, ptr0, len0, rgba, ptr1, len1);
        return ret !== 0;
    }
    /**
     * Changes one retained scroll offset by node key and axis.
     * @param {string} key
     * @param {number} axis
     * @param {number} offset
     * @returns {boolean}
     */
    set_scroll(key, axis, offset) {
        const ptr0 = passStringToWasm0(key, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_set_scroll(this.__wbg_ptr, ptr0, len0, axis, offset);
        return ret !== 0;
    }
    /**
     * Enables or disables one document-level state by name.
     * @param {string} name
     * @param {boolean} on
     */
    set_state(name, on) {
        const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        wasm.kinst_set_state(this.__wbg_ptr, ptr0, len0, on);
    }
    /**
     * Selects a compiled theme by name.
     * @param {string} name
     * @returns {boolean}
     */
    set_theme(name) {
        const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.kinst_set_theme(this.__wbg_ptr, ptr0, len0);
        return ret !== 0;
    }
    /**
     * Returns immutable document pools and host schemas as JSON.
     * @returns {string}
     */
    statics_json() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_statics_json(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Drains settled-frame signals in the canonical conformance dump shape.
     * @returns {string}
     */
    take_signals_dump_json() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_take_signals_dump_json(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Drains signals queued while solving the preceding frame.
     * @returns {string}
     */
    take_signals_json() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_take_signals_json(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Returns the current theme name.
     * @returns {string}
     */
    theme() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_theme(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Emits the canonical summary of retained trace state.
     * @returns {string}
     */
    trace_summary_json() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.kinst_trace_summary_json(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
}
if (Symbol.dispose) KInst.prototype[Symbol.dispose] = KInst.prototype.free;
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg___wbindgen_throw_344f42d3211c4765: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbindgen_cast_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./slab_kernel_bg.js": import0,
    };
}

const FrameBufFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_framebuf_free(ptr, 1));
const KInstFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_kinst_free(ptr, 1));

function getArrayF64FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getFloat64ArrayMemory0().subarray(ptr / 8, ptr / 8 + len);
}

function getArrayU32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedFloat64ArrayMemory0 = null;
function getFloat64ArrayMemory0() {
    if (cachedFloat64ArrayMemory0 === null || cachedFloat64ArrayMemory0.byteLength === 0) {
        cachedFloat64ArrayMemory0 = new Float64Array(wasm.memory.buffer);
    }
    return cachedFloat64ArrayMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint32ArrayMemory0 = null;
function getUint32ArrayMemory0() {
    if (cachedUint32ArrayMemory0 === null || cachedUint32ArrayMemory0.byteLength === 0) {
        cachedUint32ArrayMemory0 = new Uint32Array(wasm.memory.buffer);
    }
    return cachedUint32ArrayMemory0;
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function passArray32ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 4, 4) >>> 0;
    getUint32ArrayMemory0().set(arg, ptr / 4);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passArray8ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 1, 1) >>> 0;
    getUint8ArrayMemory0().set(arg, ptr / 1);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedFloat64ArrayMemory0 = null;
    cachedUint32ArrayMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('slab_kernel_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
