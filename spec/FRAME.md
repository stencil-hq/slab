# Kernel public API, Event, Effects, and Frame contract

The hand-maintained Rust crate `crates/slab-kernel` is the normative kernel
implementation and public API. Its modules expose ordinary Rust structs,
enums, and functions; native clients decode SLIR into `slir::Doc`, initialize
`frame::Instance`, and call this API directly. The browser and Node runtimes
use the wasm-bindgen bridge in `crates/slab-kernel-wasm`, which owns the same
Rust `Instance` and preserves the contract across the WebAssembly boundary.
SDP exposes this same instance and frame contract to deterministic automation;
see [SDP.md](SDP.md) for framing, canonical key addressing, input, queries,
rendering, and host-mount policy.


## Instance API

The native Rust surface uses borrowed documents, instances, events, and frame
values in the usual way. Native clients normally call `slab_slir::instance`
and receive an initialized `Instance`. Its decoder performs the low-level
construction contract exposed by the kernel: create an empty instance, assign
the decoded public `Doc`, then initialize it:

```rust
fn inst_shell() -> Instance
    // Returns an empty instance. The host assigns i.doc to its decoded Doc.
fn inst_init(i: &mut Instance)
    // Initializes persistent param state after the host assigned i.doc.
fn inst_font_register(i: &mut Instance, family: &str, weight: u32, upem: u32,
                      ascent: i32, descent: i32, line_gap: i32,
                      default_adv: u32, cmap_cp: &[u32], cmap_gid: &[u32],
                      adv: &[u32]) -> i32
    // Appends a runtime metric table, marks i dirty, and returns its FONT index.
    // Equal-name candidates override compiled tables by their later index.
fn inst_img_register(i: &mut Instance, name: &str, w: u32, h: u32,
                     format: u32, data: &[u8]) -> i32
    // Register or replace a runtime image and return its unified image index.
    // format: 0 PNG | 1 straight-alpha sRGB RGBA8. Invalid input returns -1.
fn inst_img_unregister(i: &mut Instance, name: &str) -> bool
    // Deactivate a runtime image by name while reserving its index.
fn inst_img_info(i: &Instance, img: i32) -> Option<(u32, u32, u32, u32)>
    // (width, height, format, generation); None = unknown or inactive.
fn inst_img_bytes(i: &Instance, img: i32) -> &[u8]
    // Immutable compiled or active-runtime payload; unknown/inactive = empty.
fn inst_set_env(i: &mut Instance, vw: f64, vh: f64, client: u32,
                dark: bool, coarse: bool)
    // client: 0 web | 1 gpu | 2 tui | 3 svg | 4 png (SLIR Client syms).
    // portrait/landscape derive from vw < vh. vh <= 0 = unbounded height
    // (static render invocation). Env starts unset; vw/vh are 0 until the
    // host calls this — the first frame after it solves.
fn inst_set_state(i: &mut Instance, name: &str, on: bool)
    // Global state set (drives State(sym) when-conds; dispatch feeds the
    // per-node overlay). Unknown names are no-ops (they cannot affect any
    // cond).
fn inst_set_theme(i: &mut Instance, name: &str) -> bool
    // Select a compiled theme. Empty selects the authored base. An unknown
    // non-empty name returns false and leaves the current theme unchanged.
fn inst_theme(i: &Instance) -> String
    // Current selected theme; empty means the authored base.
fn inst_set_node_state(i: &mut Instance, key: &str, name: &str, on: bool) -> bool
    // Toggle a named state on ONE node addressed by its full key path.
    // Dispatch owns hover/pressed/focus/focus-visible/composing; hosts
    // drive app states (disabled, selected, …) here. false = unknown key.
fn inst_set_focus(i: &mut Instance, key: &str, visible: bool) -> bool
    // Move focus to a keyed node in the CURRENT painted scene. false means
    // unknown/ambiguous, absent, inert, disabled, collapsed, clipped away, or
    // otherwise non-focusable. visible selects the keyboard-grade ring. This
    // explicit API does not reveal; use inst_reveal or inst_focus_item.
fn inst_clear_focus(i: &mut Instance) -> bool
    // Clear focus and any cross-field range while retaining EditState; true
    // when either retained interaction state changed.
fn inst_focus_note(i: &Instance) -> &str
    // Actionable explanation of the last failed focus/focus_item request.
    // A successful focus or clear request resets it.
fn inst_set_field_text(i: &mut Instance, key: &str, text: &str) -> bool
    // Replace or create the keyed field EditState while focused or blurred.
    // Reset composition, selection, and undo/redo; place the caret at the end.
    // Synchronize a same-named Text param. A changed value marks dirty and
    // queues Change in Effects for inst_take_signals. false = unknown/non-field.
fn inst_field_text(i: &Instance, key: &str) -> Option<String>
    // Committed EditState text, or resolved content before first bind.
    // None = unknown key or a node without field=.
struct FieldStyle { start: i32, end: i32, rgba: u32, flags: u32 }
    // Paint-only codepoint range; flags bit 0 requests synthetic italic.
fn inst_set_field_styles(i: &mut Instance, key: &str,
                         styles: &[FieldStyle]) -> bool
    // Atomically replace ascending, non-overlapping ranges after clamping to
    // committed text bounds. false = invalid ranges or unknown/non-field key.
struct FieldRun { style: u32, start: i32, end: i32 }
    // style: 0 bold | 1 italic | 2 underline | 3 strike | 4 code.
    // start/end are a non-empty half-open codepoint range on grapheme
    // boundaries.
struct FieldRuns { revision: u64, runs: Vec<FieldRun> }
fn inst_field_runs(i: &Instance, key: &str) -> Option<FieldRuns>
    // Return style-major, sorted, disjoint normalized runs and the field's
    // monotonic local revision. An unbound field is revision 0 with no runs.
fn inst_set_field_runs(i: &mut Instance, key: &str, runs: &FieldRuns) -> bool
    // Replace all five span sets without touching text, as one undo step.
    // Clamp offsets to bounds and nearest grapheme boundaries. Reject unknown
    // styles or reversed ranges atomically; normalize overlap and adjacency.
    // The supplied revision is informational and is not adopted.
struct FieldSnapshotEntry {
    locator: String, text: String, runs: FieldRuns,
    caret: i32, anchor: i32, goal_x: f64,
}
struct FieldSnapshot { fields: Vec<FieldSnapshotEntry> }
fn inst_snapshot_fields(i: &Instance,
                        locators: &[&str]) -> Option<FieldSnapshot>
    // Resolve every locator using the ordinary field-key rules, require every
    // target to have a bound EditState, and return entries in caller order with
    // escaped canonical full locators. Unknown, non-field, unbound, or
    // duplicate targets reject the whole capture. Capture is pure: success and
    // failure leave text, composition, selection, and both histories untouched,
    // so an aborted host transaction requires no rollback.
fn inst_commit_fields(i: &mut Instance, locators: &[&str]) -> bool
    // Commit a successfully applied host structural mutation. Resolve every
    // currently retained affected field before changing any history, then
    // empty undo and redo for all of them. false writes no partial barrier.
    // Removed fields need not be listed because they retain no EditState.
fn inst_restore_fields(i: &mut Instance,
                       snapshot: &FieldSnapshot) -> bool
    // Resolve and validate every entry before changing any field. Every
    // locator must still resolve to a field and duplicate targets are invalid;
    // false is all-or-nothing, with no partial restore, signal, or repaint.
    // Restore committed text, normalized runs, caret/anchor, goal_x, and the
    // captured revision exactly; clear active composition and cross-field
    // range state; synchronize bound Text params; queue Change for changed
    // content/runs; and request relayout/repaint. Reset both local history
    // directions to empty: the restored state is the new field baseline.

    // The derived JSON shape is deterministic and contains no kernel handles:
    // {"fields":[{"locator":"#root/#block","text":"hello",
    //   "runs":{"revision":7,"runs":[{"style":0,"start":0,"end":5}]},
    //   "caret":5,"anchor":5,"goal_x":-1.0}]}
fn inst_toggle_style(i: &mut Instance, key: &str, style: u32) -> bool
    // Toggle over the current non-empty selection as one undo step. Fully
    // covered means remove; otherwise add and normalize. Empty selection is a
    // deliberate no-op: the kernel does not infer or expand to a word.
struct CaretState { caret: i32, anchor: i32, composing: bool, goal_x: f64 }
    // caret is the active selection end; anchor is the fixed end. Both are
    // codepoint offsets at grapheme-cluster boundaries in committed text.
    // goal_x is the visual-x target retained by vertical movement; negative
    // means no active target.
fn inst_set_caret(i: &mut Instance, key: &str, caret: i32, anchor: i32) -> bool
    // Resolve key by the field API's canonical locator conventions. The target
    // must have field= and be focusable in the CURRENT painted scene; otherwise
    // false with no caret change. Focus it with a pointer-grade (hidden) ring.
    // Clamp offsets to text bounds and then to the nearest grapheme boundary
    // (an equal-distance tie chooses the preceding boundary), preserve their
    // direction, cancel active composition, reset goal_x, and mark dirty.
fn inst_set_caret_goal(i: &mut Instance, key: &str, caret: i32,
                       anchor: i32, goal_x: f64) -> bool
    // As inst_set_caret, but caret first chooses a visual line in the retained
    // TextLayout, then resolves to its nearest shaped stop at goal_x. A
    // collapsed selection follows the resolved caret. Retain goal_x for the
    // next vertical move. false = invalid goal or no retained text layout, in
    // addition to inst_set_caret failures.
fn inst_get_caret(i: &Instance, key: &str) -> Option<CaretState>
    // Return caret, anchor, composing, and goal_x for the target's EditState.
    // None = unknown/non-field key or no EditState (before first focus/write).
struct FieldLocator { key: String, offset: i32 }
    // One endpoint field's escaped canonical full key and grapheme-boundary
    // committed-text codepoint offset. The key includes stable list-item
    // identity and is never a retained raw node or scene index.
fn inst_get_range(i: &Instance) -> Option<(FieldLocator, FieldLocator)>
    // Return (fixed anchor, active head) for the retained cross-field range.
    // Keys survive keyed reorder and virtual de-materialization; a permanently
    // unresolvable endpoint invalidates the range. None means field-local or
    // collapsed selection.
fn inst_clear_range(i: &mut Instance) -> bool
    // Clear only cross-field metadata, retaining every field EditState and its
    // local selection. true only when a range existed.
fn inst_focus(i: &Instance) -> u32
    // Current focused node; 0xFFFFFFFF means no focus.
fn inst_param_json(i: &Instance, name: &str) -> Option<String>
    // Current scalar or recursive list value as deterministic JSON.
    // Text/enum/color are strings; num/pct are numbers; bool is boolean;
    // lists are arrays with stable key plus every schema field. None = unknown.
fn inst_set_scroll(i: &mut Instance, key: &str, axis: u32, off: f64) -> bool
    // Set a keyed active scroll axis: 0 main | 1 cross. The offset clamps to
    // retained geometry when available. false = unknown key/axis/inactive axis.
fn inst_get_scroll(i: &Instance, key: &str, axis: u32) -> f64
    // Stored offset for axis 0 or 1; unknown keys/axes read as 0.
fn inst_reveal(i: &mut Instance, key: &str, margin: f64) -> bool
    // Minimally scroll every main-axis ancestor in the retained scene so the
    // target plus margin is visible. false = target absent from current scene.
fn inst_reveal_item(i: &mut Instance, each_key: &str, index: i32,
                    align: u32) -> bool
    // Virtual item alignment: 0 start | 1 center | 2 end | 3 nearest.
    // false = unknown/non-virtual each, invalid index, or invalid alignment.
fn inst_focus_item(i: &mut Instance, each_key: &str, index: i32) -> bool
    // Reveal a virtual item with nearest alignment and keyboard-focus its first
    // active focusable descendant, materializing it when needed. Failure is
    // explained by inst_focus_note.
fn inst_each_window(i: &Instance, each_key: &str) -> (i32, i32)
    // Half-open materialized virtual range; (-1, -1) for unknown/non-virtual.
fn inst_set_item_extent(i: &mut Instance, each_key: &str, index: i32,
                        extent: f64) -> bool
    // Enable retained per-item extents and set one finite positive main extent.
    // Changes above the viewport preserve the first visible item's anchor.
fn inst_set_param(i: &mut Instance, param: u32, v: &ParamValue) -> bool
    // false = unknown param, type mismatch, or unknown enum member.
fn inst_list_len(i: &Instance, param: u32, path: &str) -> i32
    // Selected list length; -1 when the param or path cannot be resolved.
fn inst_set_list_len(i: &mut Instance, param: u32, path: &str, n: i32) -> bool
    // false for an unresolved list or n < 0. Extending seeds schema defaults;
    // truncating drops descendant values and identities.
fn inst_set_list_field(i: &mut Instance, param: u32, path: &str, index: i32,
                       field: &str, v: &ParamValue) -> bool
    // Typed scalar write. false for unresolved list/item/field, type mismatch,
    // a list-typed field, or an unknown enum member.
fn inst_set_list_key(i: &mut Instance, param: u32, path: &str, index: i32,
                     key: &str) -> bool
    // Set stable item identity; default identity is decimal index. Empty keys
    // and unresolved items fail. Equal successful writes do not dirty.
fn inst_set_divider(i: &mut Instance, key: &str, extent: f64) -> bool
    // Store a finite extent overlay for a structurally valid keyed divider.
fn inst_get_divider(i: &Instance, key: &str) -> f64
    // Stored overlay, or -1 for an unknown, invalid, or unset divider.
fn inst_set_hole_size(i: &mut Instance, hole: u32, w: f64, h: f64)
    // Persist the host content's reported natural width and height. Invalid
    // hole indices are ignored. Mark dirty iff either stored float changes;
    // an equal re-report is a no-op. A Hole node consults the corresponding
    // dimension only on a hug axis, starting from 0 before the first report,
    // then applies the ordinary min/max and layout-constraint clamps.
fn inst_frame(i: &mut Instance, t_ms: f64) -> Frame
    // Solves iff needed: dirty (env/param/state/scroll/edit/focus change),
    // or the clock moved while the doc has ANIM binds / transitions
    // ("interpolate inputs, re-solve" — an animating instance stays live,
    // an idle one solves once). Post-solve, matching research App.build():
    // scroll offsets re-clamp against the fresh scene and vanished focus
    // restores (§15.3) — both mark the instance dirty for the NEXT frame.
fn inst_holes(i: &mut Instance) -> Vec<HoleRect>
    // Last-solved host viewports. For a hug hole, the sanctioned host loop is:
    // solve -> read viewport -> measure natural host content -> report with
    // inst_set_hole_size -> re-solve once. The reported size is a persistent
    // input, and an equal subsequent report does not dirty the instance, so
    // stable natural content converges without another demand frame.
fn inst_hit(i: &Instance, x: f64, y: f64) -> Vec<u32>
    // Node path root -> target against the retained scene (§15.2):
    // reverse paint order, rotation transform (deterministic kernel
    // sin/cos — quadrant reduction + fixed Taylor polynomial), rounded
    // rects, clip chains; inert subtrees never hit.
fn inst_dispatch(i: &mut Instance, ev: &dispatch::Event) -> dispatch::Effects
    // §15.4 dispatch (see below). Effects.repaint doubles as the dirty
    // mark: the next inst_frame re-solves.
fn inst_take_signals(i: &mut Instance) -> dispatch::Effects
    // Destructively drains signals queued by settled-frame gesture
    // cancellation. Live hosts call once immediately after inst_frame.
fn text_glyphs(i: &Instance, fr: &Frame, op: i32) -> Vec<GlyphPos>
    // Per-codepoint advance walk for fr.ops[op] (must be a Text op):
    // gid from the FONT cmap, x advances by advance(gid)·size/upem +
    // tracking. For GPU drivers (P7).
```

### Browser and Node WebAssembly surface

Instance editing APIs are available at both public host boundaries. SDP exposes
`field.caret.get/set`, `field.runs.get/set`, `field.style.toggle`,
`field.styles`, and `field.range.get/clear`; SDP.md defines their strict request
schemas and protocol errors.

`KInst` is the wasm-bindgen owner of a decoded and initialized Rust `Instance`.
Its constructor accepts SLIR bytes and returns an error if Rust cannot decode
them. The field bridge exposes `set_caret` and `get_caret_json`,
`field_runs_json` and `set_field_runs_json`, `toggle_style`,
`get_range_json`, and `clear_range`. Caret JSON maps the native negative
`goal_x` sentinel to null. Runs JSON uses the exact Change payload
`{"rev":u64,"runs":[{"style":u32,"start":i32,"end":i32}]}`; malformed JSON
rejects `set_field_runs_json` without mutation. Range JSON is either null or
`{"anchor":{"key","offset"},"head":{"key","offset"}}` with canonical
`FieldLocator` keys.
Paint-only styles cross WASM as
`set_field_styles(key: string, flat: &[i32])`, where `flat` contains repeated
`start,end,rgba,flags` quads.

The bridge otherwise mirrors the native contract with JavaScript-safe
arguments. Its generic `set_param` and `set_list_field` transports take
`kind, num, value, rgba, boolean, symbol` payload arguments: kind 0 selects
`value` (Text), 1 selects `num` (Num), 2 selects `num` (Pct), 3 selects `rgba`
(Color), 4 selects the distinct `boolean` payload (Bool), and 5 selects
`symbol` (Enum). Payloads not selected by `kind` are ignored. Rust callers
instead pass the corresponding typed `ParamValue` variant, including
`ParamValue::Bool(bool)`. List methods (`list_len`, `set_list_len`,
`set_list_key`, `set_list_field`) include `path`; `set_scroll` and
`get_scroll` include `axis`. It also exposes `reveal`, `reveal_item`,
`focus_item`, `each_window_json`, `set_divider`, `get_divider`,
`img_register`, `img_unregister`, `image_info_json`, and unified-index
`image_data`, plus `clear_focus` and `focus_note` alongside the existing
environment, parameter, state, focus, theme, font, and hole methods.

Cold structured results cross the boundary as JSON: `holes_json`,
`dispatch_json`, `caret_effects_json`, `statics_json`, `scene_json`,
`chain_json`, field/caret/range queries, and active-theme `get_token_json`;
retained-scene queries use `hit_contains`. `dispatch_json` takes
all ten `Event` fields as flat arguments and returns the complete `Effects`
object described below. In `scene_json`, accessibility references are resolved
to strings rather than exposing the native scene-string pool.

The WASM `EffectSnapshot` preserves `sig_runs` as JSON strings parallel to
`sig_name` and preserves an optional structured `range_edit`. The web adapter
parses non-empty `sig_runs` into each named signal event's `detail.runs` and
emits a bubbling, composed `slab-range-edit` event whose detail is the complete
`RangeEdit`; hosts apply that request atomically to their block model.

The paint hot path is `frame(t_ms) -> FrameBuf`, not frame JSON. `FrameBuf`
provides an operation-tag/payload `u32s()` stream, an `f64s()` stream beginning
with frame width and height, the frame-local text pool through `strs_json()`,
the flat uncovered-glyph run pool through `uncovered_u32s()`,
runtime paths through `rt_paths_json()`, complete current-solve layout and
runtime evidence through `diagnostics_json()`, and the `dirty()` and
`motion_active()` liveness flags. The cumulative per-instance diagnostic set is
`KInst.diags_json()` (same `{code, line, msg}` entry shape). ScalePush and
ScalePop use tags 11 and 12;
ScalePush contributes four floats (`cx, cy, sx, sy`) and ScalePop contributes
none. TiltPush and TiltPop use tags 13 and 14; TiltPush contributes five
floats (`cx, cy, rx, ry, depth`) and TiltPop contributes none. The FX-kit
fields extend existing payloads at the END of each op's record: RECT appends
`smooth, grain_amount, grain_size` (floats); TEXT appends `color_kind` (u32),
`strike` (u32), `uncov_off` and `uncov_len` (signed u32 words) after its fixed
words, and `gx, gy, gw, gh` (floats) — a TEXT record is ten u32s
(`tag, node, str_ref, font, weight, color, color_kind, strike, uncov_off,
uncov_len`) and ten floats; IMAGE and CLIP_PUSH append `smooth`;
GROUP_PUSH contributes `node, mask_kind, mask` (u32s) and
`opacity, blur, mx, my, mw, mh` (floats); BACKDROP appends
`mask_kind, mask` (u32s) and `brightness, smooth` (floats). Every operation
tag otherwise retains its fixed integer and float arity
and decodes to the `FrameOp` payload fields below. Canonical `frame_json`,
dispatch/hit/trace dumps, `cells_text`, capability reports, and self-test
counts are exported for the Node conformance runner.

Layout-time diagnostics accumulate per solve in `Instance.st.diag_code` /
`diag_msg` / `diag_line` (parallel arrays): `squeeze`, `clipped`,
`pct-unbounded`, `fill-unbounded`, `attr` — research wording, numbers
rendered with the canonical `fmt3` formatter.

```rust
pub enum ParamValue {
    Text(String),
    Num(f64),
    Pct(f64),
    Color(u32),
    Bool(bool),
    Enum(String),
}
```

`inst_set_param` accepts only the variant matching the declared scalar
parameter type; a List parameter cannot be replaced through this setter.
`inst_set_list_field` likewise requires the variant matching the selected
scalar field and rejects a List-typed field. For either setter, an Enum symbol
must be a member declared by that parameter or field. An unknown WASM `kind`
and every resolution, type, or enum-validation failure return `false` without
mutation or a dirty mark. A valid write returns `true`; if its value is equal
to the retained value it is a no-op and does not dirty the instance, while an
actual value change is applied atomically and marks the instance dirty for the
next `inst_frame`.

```rust
struct HoleRect { hole: u32, x: f64, y: f64, w: f64, h: f64, clip: bool }
struct GlyphPos { font: i32, gid: u32, x: f64, y: f64, size: f64 }
```

The unified image index space contains every compiled `IMGS` row first,
followed by append-only runtime slots. A runtime name keeps its slot when
replaced, unregistered, or registered again. Compiled rows report generation
zero; a new runtime row starts at one, and replacement, unregister, and
re-registration each advance its generation. Re-registering an active image
with identical dimensions, format, and bytes is a no-op: its generation and
instance dirty bit do not change. RGBA8 requires exactly `w * h * 4` bytes
with checked arithmetic. Any rejected registration is atomic.

Scroll axis 0 requires effective `F_SCROLL`, uses `content_main`, and has
viewport width for a row or height for a column. Axis 1 requires
`F_SCROLL_CROSS`, uses `content_cross`, and takes the opposite viewport
dimension. A retained write clamps to
`[0, max(0, content - viewport)]`. A write before the first solve is retained
and re-clamped against the newly solved scene; either axis may therefore mark
the instance dirty for one settling frame.

`inst_reveal` operates only on the current retained scene. Negative or
non-finite margin is treated as zero. It processes scroll ancestors from inner
to outer, carrying each inner displacement into the next calculation, so every
write is the minimal main-axis move. A known already-visible target still
returns `true`. `inst_reveal_item`, `inst_focus_item`, and `inst_each_window`
apply only to a virtual `each`; a non-virtual or unknown key fails without
mutation. Alignment is relative to the scroll node's **content box**, not its
outer border. Start alignment can therefore leave a nonzero raw scroll offset
for leading padding or preceding in-flow content; assert visibility/alignment,
not `offset == 0`.

A divider key is valid only when its base node is a non-first, non-last direct
child of a row or column. The divider API stores a finite keyed extent overlay;
it does not treat an arbitrary keyed node as a divider. An unset or invalid
divider reads as `-1`, rejected writes are atomic, and an equal valid write
does not dirty.

List `path` is `""` for the root list. A nonempty path is a dot-separated
sequence of `<index>.<field>` pairs selecting descendant list fields:
`3.segments` and `3.segments.0.points` are examples. Indices are nonnegative
decimal item indices; each field must be list-typed in the selected schema.
Malformed paths, scalar fields, missing schemas, and out-of-range items fail
atomically (`-1` from `inst_list_len`, `false` from setters). The separate
`index` argument to field/key setters addresses an item in the list selected
by `path`.

List values, keys, and the synthetic-node registry are persistent Instance
state. Retained Bool parameter and list-field values are bit-packed; this
runtime storage detail does not change their typed host payloads. Synthetic
ids are stable by `(Each node, template node, item key)`;
state maps retain that synthetic id while every SLIR/document read uses its
template base node. The public descendant key is
`<each-key>~<item-key>/<template-relative-key>`. Truncation prunes values,
keys, registry entries, and keyed state for removed identities.

### Canonical scene keys and locators

Every scene node has one canonical full key. Its grammar is:

```text
full-key       = segment *("/" segment)
segment        = explicit | "#" id | kind "@" index
item-node-key  = each-full-key "~" item-key "/" template-relative-key
nested-item    = item-node-key "~" item-key "/" template-relative-key
```

For each authored sibling, segment precedence is explicit `key=v`, then
`#id`, then the anonymous lower-case node kind plus its zero-based ordinal
among unkeyed same-kind siblings (`col@0`, `rect@2`, `each@0`). A component
call contributes its own segment; every expanded body root and slotted child
continues beneath it. Thus `Button#save` whose first body root is an anonymous
row produces a path containing `#save/row@0`: the call id is a segment, while
the actual focus/click target is the body root.

An `each` uses the same explicit/`#id`/`each@index` precedence. A concrete
descendant inserts `~<item-key>/` between the each's full key and the detached
template-relative key. Every nested each repeats that marker. Before a host
assigns a stable item key, its decimal item index is used. `sig_item` remains
the raw innermost item key; `SigMeta.key`, scene snapshots, and all node APIs
use the canonical escaped full key.

`%`, `/`, and `~` are structural bytes. Literal occurrences inside explicit
`key=` values or stable item keys are emitted as uppercase `%25`, `%2F`, and
`%7E`; a literal `%2F` value therefore appears as `%252F`. `#id` and generated
`kind@index` segments are grammar-safe. Hosts should copy returned/generated
canonical keys rather than manually concatenate them.

Kernel key locators accept an exact canonical full key. For author ergonomics,
a bare `#id` or `id` may resolve only when unique, and an authored suffix rooted
at an id (for example `#list/rows`) may resolve only when unique. Component-call
ids resolve to their actual first body root. Ambiguous shorthands fail rather
than selecting arbitrarily; `scene::resolve_key` returns deterministic
`Found`, `Missing { candidates }`, or `Ambiguous { candidates }`, and failed
focus calls expose the same actionable result through `inst_focus_note`.

## Event (module `dispatch`)

```rust
struct Event {
    etype: u32,
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
    button: u32,
    clicks: u32,
    key: String,
    text: String,
    clauses: Vec<(i32, i32)>,
    mods: u32,
}
```

- `etype`: 0 pointer-move | 1 pointer-down | 2 pointer-up | 3 wheel |
  4 key-down | 5 text | 6 paste | 7 copy | 8 cut | 9 composition-start |
  10 composition-update | 11 composition-end | 12 blur | 13 resize |
  14 close | 15 inspect | 16 activate (synthesized internally; ignored
  from outside).
- `x`/`y` are document-space pointer coordinates. Pointer-move `dx`/`dy` are
  event-local deltas. Supplied deltas are authoritative; when both are zero and
  coordinates changed during capture, dispatch derives them from the previous
  coordinates. `dx`/`dy` are wheel deltas for type 3 and the new viewport
  width/height for type 13.
- `button` is the host pointer-button code. `clicks` is the host-computed click
  count on pointer-down; 0 and 1 are single clicks, any count >= 2 is a
  multi-click and drives Dblclick.
- `key` is the host named key (`"Tab"`, `"Enter"`, `" "`, `"ArrowLeft"`,
  `"Backspace"`, `"Home"`, `"End"`, `"a"`, …), not a document STRS reference.
  `text` carries text, paste, and composition payloads. For
  `composition-update`, `clauses` is the ordered list of `(start, end)`
  codepoint-offset ranges within `text`. Empty and single-entry lists mean one
  whole-preedit clause, providing the required fallback for hosts without
  clause support. Multi-clause ranges are clamped to the preedit bounds.
- `mods` bitset: 1 shift | 2 alt | 4 ctrl | 8 meta.

JSON trace and SDP `input.event` ingress spells a composition update as
`{"type":"composition-update","text":"…","clauses":[[start,end],...]}`;
`clauses` is optional. Malformed clause metadata degrades atomically to an
empty list rather than rejecting the event. The WASM `dispatch_json` and
`dispatch_dump_json` methods expose the same optional data as their trailing
`clauses_json` string argument, whose value is the JSON array above; omitted or
malformed JSON likewise means an empty list.

Dispatch is kernel-owned: no capture/bubble (the kernel routes internally and
reports Effects); pointer capture lasts from pointer-down until release;
`pressed` lands on the nearest focusable in the hit path, else the raw target;
hover enter/leave states cover the whole hit path. Pointer-up over the
still-pressed focusable, or Enter/Space on a focused non-edit node, synthesizes
Activate; `disabled` suppresses it. A pointer-down with `clicks >= 2` fires
Dblclick on the deepest enabled binding and suppresses that gesture's
Activate. Escape first cancels an armed or active
drag, emits cancelled DragEnd without Drop, clears capture, and is consumed.
Otherwise an editable node with authored `escape-blur` consumes Escape, fires
its `cancel=` binder (trigger 14) with the retained committed buffer as
`sig_text`, and clears focus while retaining its EditState; without that
explicit opt-in, Escape remains available to authored `keys=` mappings and app
semantics. Key-down routing with empty focus starts at the document root's
`keys=` map, and an unhandled focused walk falls back to that root map.
With a field focused, its own `keys=` map preempts editing for plain
(unmodified), non-printable keys; unmodified printable keys never leave the
editor, while modified printable chords (for example Cmd+B) bubble with
`SigMeta.mods` set. A field-edit command that changes nothing at a boundary
(Backspace/Delete at a text edge, an arrow clamped at a text or visual-line
edge) is not consumed and bubbles through `keys=`; commands that mutate
text, move the caret, or emit an effect never bubble. PageUp/PageDown/Home/End scroll
the nearest scroll-container ancestor of the focus (or the primary root
scroller when focus is empty) by exactly one viewport extent per page step.
Wheel scrolls the deepest `scroll` node in the path with the retained-scene
clamp. `resize` (dx/dy > 0) updates env; `blur` clears hover and pressed;
`copy` / `inspect` are host territory.

Tab, Shift-Tab, and kernel-owned directional traversal use materialized
authored order, except that an `attach=` overlay subtree inserts into
traversal immediately after its anchor node (nested overlays resolve through
their anchors recursively). When an overlay containing the focus leaves the
scene, focus restoration returns to the overlay's anchor before the ordinary
nearest-neighbor rule. Effective inert/disabled/non-focusable nodes, empty
painted rectangles, and descendants wholly removed by a non-scroll clip are
excluded. An off-screen descendant of a scroll viewport remains eligible:
after traversal changes focus, the kernel minimally reveals it through every
scroll ancestor before the next solve. Virtual-list overscan plus that scroll
update materializes the continuing focus ring without host-computed offsets.
Explicit `inst_set_focus` intentionally does not reveal.

Signal trigger codes (SPEC §13) are `0 Activate`, `1 Change`, `2 Submit`,
`3 Press`, `4 Context`, `5 Dblclick`, `6 DragStart`, `7 Drop`, `8 Resize`,
`9 PointerMove`, `10 PointerUp`, `11 DragUpdate`, `12 DragEnd`, and
`14 Cancel` (`13` is the internal typed-`keys=` activation discriminator).
`sig_text` carries committed text for Change/Submit, the retained buffer for
Cancel, and the canonical final extent for Resize; other triggers use `""`.
`sig_runs` is parallel to `sig_text`. For a field Change it is one compact JSON
object string with this exact schema (object keys and run keys are emitted in
the shown order):

```json
{"rev":3,"runs":[{"style":0,"start":1,"end":4}]}
```

The schema is `{"rev": u64, "runs": [{"style": u32, "start": i32,
"end": i32}, ...]}`. Runs are style-major (`0` through `4`), then ascending
range order. Non-field signals carry `""`.
Every signal carries the innermost synthetic item key of its emitting node
(or `""`) and the `SigMeta` below.

Focusing a field binds an EditState seeded from CONTENT on FIRST bind only.
The EditState persists across blur/refocus. A host write to the field's
same-named synced Text param resets the buffer to the written value while the
field is not composing (caret at the end, one undo step, no Change echo); an
active IME composition keeps kernel priority.
`inst_set_field_text` replaces it while focused or blurred, resets selection
and undo/redo, places the caret at the end, synchronizes a same-named Text
param, and queues Change in Effects when the value changes. Normal field
mutations also synchronize that parameter. Item-key change discards retained
edit identity.
Rich spans live beside, not inside, the string field value. Every committed
local text splice, style toggle/write, undo, or redo increments a monotonic
`revision`; text and spans restore atomically on undo/redo but the revision
itself never rolls back. A same-named Text-param reset is host synchronization,
so it clears spans with the replacement text but neither emits Change nor
increments revision. A host can remember the revision from its own last write
and ignore a returned Change payload with that revision, while accepting later
revisions. `inst_set_field_text` clears existing spans and host paint styles;
rich hosts replace text and then call `inst_set_field_runs`. Paint-only styles
do not participate in measurement, wrapping, revisions, Change payloads, caret,
selection, or IME geometry. Text splices adjust their codepoint endpoints like
inline spans; paint emission splits runs at their boundaries, overrides color,
and ORs synthetic italic without selecting a different face.
Backspace/Delete delete grapheme clusters; Ctrl/Meta/Alt word deletion and
Ctrl-K/U kills use the visual line; Ctrl/Meta-Z and
Ctrl/Meta-Shift-Z traverse bounded grouped
undo/redo. Multiline ArrowUp/Down and Home/End use visual-line source offsets
from the retained TextLayout with goal-x preservation. Enter inserts or
submits by SPEC §15.6's modifier/flag matrix. Text, paste, cut, kill, word
delete, undo/redo, and composition all flow through the same committed-change
path. Single-line display text owns horizontal scroll; multiline caret follow
may adjust the nearest scroll ancestor.
Composition update text remains an uncommitted marked overlay: it does not
enter the field buffer or rich spans. Flattening emits one font-derived
underline segment per non-empty clause on each intersected visual line;
composition end clears the clause overlay before committing through the normal
text-and-span splice path. Caret and IME candidate rectangles are unchanged.

`inst_set_caret` focuses a painted field with a hidden focus ring, cancels
uncommitted composition without changing committed text, installs the directed
selection at clamped grapheme boundaries, resets vertical-movement `goal_x`,
and repaints. `inst_set_caret_goal` instead resolves the active end on the
selected visual line at its supplied non-negative finite `goal_x`, retains that
goal for vertical movement, and keeps a collapsed selection collapsed at the
resolved position. This permits a host to carry the visual target across block
boundaries. `inst_get_caret` reports the directed selection, composition state,
and goal only after an EditState exists.

Cross-field range endpoints retain escaped canonical full keys, including
stable list-item identity, rather than node IDs or scene indices. Shift-primary
down across fields creates the range directly. After a boundary key bubbles,
an edge-anchored `inst_set_caret` on the next field composes the source and
destination local selections. `inst_get_range` returns `(anchor, head)` even
when a virtual window temporarily de-materializes an endpoint. Every dispatch
and solve resolves the keys afresh: keyed reorder changes range direction and
paint order, a de-windowed endpoint emits no band, and a genuinely missing key
invalidates the range. Flatten paints partial endpoint bands and full-text
bands for materialized middle fields. Whole-row tint remains host state paint,
not a kernel Frame op.

Secondary pointer-down emits Context with pointer metadata and never presses
or arms drag. On an editable focusable field, it applies pointer-grade focus.
It preserves selection when the hit caret lies inside that selection.
Otherwise, it collapses selection at the hit caret.

## Effects (module `dispatch`)

```rust
struct SigMeta {
  x: f64,
  y: f64,
  dx: f64,
  dy: f64,
  drag_dx: f64,
  drag_dy: f64,
  mods: u32,
  button: u32,
  clicks: u32,
  key: String,
  hit_key: String,
  pressed_key: String,
  src_key: String,
  src_item: String,
  cancelled: bool,
  dropped: bool,
}
struct ScrollChange { key: String, axis: u32, off: f64 }
struct RangeEndpoint { key: String, offset: i32 }
struct RangeEdit {
  kind: u32, // 0 text | 1 paste | 2 cut | 3 Backspace |
             // 4 Delete | 5 composition | 6 copy
  anchor: RangeEndpoint,
  head: RangeEndpoint,
  text: String,
}
struct Effects {
  repaint: bool,         // document state changed; next inst_frame re-solves
  sig_name: Vec<u32>,    // document STRS refs
  sig_text: Vec<String>, // committed text/extent where defined; else ""
  sig_runs: Vec<String>,// rich-field {"rev":N,"runs":[...]}; else ""
  sig_item: Vec<String>, // innermost list item key; "" for a real node
  sig_meta: Vec<SigMeta>,
  scrolls: Vec<ScrollChange>,
  range_edit: Option<RangeEdit>,
  copy_text: Option<String>, // selected static text requested by E_COPY
  has_static_selection: bool,
  has_caret: bool, caret_x: f64, caret_y: f64, caret_w: f64, caret_h: f64,
  has_ime: bool, ime_x: f64, ime_y: f64, ime_w: f64, ime_h: f64,
  cursor: u32,           // 0 default | 1 pointer | 2 text |
                         // 3 col-resize | 4 row-resize
  focus: u32,            // node id; 0xFFFFFFFF = none
}
```

`range_edit` is a pre-mutation request for the host-owned block model. With an
active cross-field range, text, paste, cut, Backspace, Delete, composition, and
copy dispatch here instead of touching the focused field. The endpoints use
the same stable locators as `inst_get_range`; replacement text is empty for
deletion/cut/copy. All field bytes and range state remain unchanged until the
host atomically applies the structural edit and pushes its list/field updates
back to the instance.

`copy_text` is the clipboard payload for an active kernel-owned static-text
selection when `E_COPY` is dispatched without a focused edit field.
`has_static_selection` tells hosts whether such a non-collapsed selection is
retained; JSON snapshots omit both fields when absent/false so opt-in selection
does not change Effects produced by existing documents.
Structural undo is a **host transaction** because the kernel never owns the
host's block list or list parameters. Before Enter-split, Backspace-merge, or
another structural edit, the host purely calls `inst_snapshot_fields` for every
affected pre-mutation field. It then attempts its structure mutation and writes
the resulting field text/runs/carets. If that attempt aborts, the snapshot is
dropped and ordinary local undo remains byte-for-byte intact. If it succeeds,
the host calls `inst_commit_fields` for every affected field that remains bound
(including newly created fields), then pushes exactly one host undo entry
`{ structure_delta, field_snapshot }`.

Host undo applies `structure_delta` first, so every captured canonical locator
exists again, and only then calls `inst_restore_fields`. If any locator cannot
be resolved after the structure revert, restore returns `false` without
restoring a subset. Commit is the hard field-history boundary: field-local
Ctrl/Meta-Z cannot enter pre-transaction history. Restore discards intervening
history again and makes the restored state a fresh baseline with empty undo and
redo stacks. Ctrl/Meta-Z at that baseline is a kernel no-op, so normal boundary
fall-through can deliver a bound `keys=z` signal to the host's structural undo
handler. Composition preedit is not snapshot data; restore always clears it.
The host must use the snapshot/commit/restore triad with its own
structure-delta stack rather than coordinating independent field undo commands.

`sig_name`, `sig_text`, `sig_runs`, `sig_item`, and `sig_meta` always have equal length
and matching order. `SigMeta.key` is ALWAYS the full key path of the emitting
node, for every trigger and origin. Pointer-derived signals also carry
`hit_key`, the full key of the deepest hit-target node under the pointer;
keyboard-driven activation (default Enter/Space and authored `keys=`) carries
the fired key name in `pressed_key`. Both are `""` when they do not apply and
are omitted from JSON dump surfaces. Pointer-originated dispatch carries
document-space `x`/`y`; keyboard and direct helper emissions use `(-1, -1)`.
`dx`/`dy` are the authoritative or derived event-local deltas;
`drag_dx`/`drag_dy` are cumulative displacement from the armed pointer-down
origin while a drag is active. `mods`, `button`, and `clicks` come from the
current event. `src_key` and `src_item` identify a drag source only for Drop
and are otherwise empty. `cancelled` marks abnormal DragEnd; `dropped` marks
Drop and the corresponding successful DragEnd.

`scrolls` is ordered by dispatch execution and contains one entry for each
offset actually changed by wheel, scroll-key, page-key, or caret-follow
handling. `axis` is 0 main or 1 cross and `off` is the stored clamped offset.
Direct host
calls to `inst_set_scroll` do not synthesize an `Effects` value.

Caret/IME rects are emitted whenever the focused node is a bound field and
are geometry of the LAST solve. The kernel finds the caret's visual line from
that TextLayout's source offsets; x is the measured line-prefix width minus
the field's horizontal edit scroll, y is the line origin, h is line height,
and w is 1. Hosts refresh after the next frame. Web uses a hidden textarea for
every focused field and prevents its own Enter insertion after forwarding one
key event; native forwards the same line-aware rect to its IME candidate API.
Focus state names exposed to `when` are `focus` and `focus-visible`
(keyboard-driven focus only; restoration and pointer focus are ring-free,
research §15.3).

## Frame

```rust
struct RtPath {
    verbs: Vec<u8>,
    coords: Vec<f64>,
}
struct Frame {
    width: f64,
    height: f64,
    ops: Vec<FrameOp>,
    scene: Vec<SceneNode>,
    strings: Vec<String>,
    uncovered: Vec<u32>,
    paths_rt: Vec<RtPath>,
    diagnostics: Vec<FrameDiagnostic>,
}
```

`FrameDiagnostic` is `{ code: String, line: u32, msg: String }`. Layout
diagnostics describe the current solve. Runtime `glyph-missing` diagnostics are
one-shot notes: an `Instance` emits at most one for each `(authored family,
codepoint)` pair, on the first frame whose text uses that missing glyph. Hosts
MUST surface these notes even when they do not repaint a missing glyph.

Alongside the per-solve stream, every distinct diagnostic accumulates on the
instance: `frame::inst_diags(&Instance) -> &[FrameDiagnostic]` returns the
cumulative set — deduplicated by `(code, line, msg)`, in first-occurrence
order — queryable at any time. It resets only when a new document initializes
(`inst_init`). Hosts expose it as `doc.diags` (SDP), `KInst.diags_json()`
(wasm), and the web element's cumulative diagnostics list.

`width`/`height` are the solved root box. `strings` is the per-frame text pool
addressed by `OpText.str_ref`. `paths_rt` contains only runtime paths referenced
by this frame. `FrameOp` is the Rust enum in `slab_kernel::flatten`, with these
payload structs:

```
Rect(OpRect)         { node, x, y, w, h, radius,
                       bg_kind, bg, stroke_kind, stroke,   // Paint, see below
                       stroke_w, stroke_align (0 center|1 inside|2 outside),
                       stroke_sides (bitmask t1 r2 b4 l8; 15 = all),
                       dash_on, dash_off, has_dash,
                       shadow_off, shadow_len,             // SLIR SHDW run
                       opacity,
                       smooth (0 = off),
                       grain_amount (0 = off), grain_size }
Text(OpText)         { node, x, y_baseline, str_ref, measured_w,
                       font (FONT table index, -1 none), size,
                       weight (the selected table's weight),
                       tracking, color, opacity, strike,   // one op PER LINE
                       color_kind (1 solid: color = rgba8 |
                                   2 gradient: color = GRAD index),
                       gx, gy, gw, gh,    // gradient box = the text NODE's
                                          // content box, shared by every
                                          // line; all 0 when solid
                       uncov_off, uncov_len }  // uncovered-glyph runs, see below
Image(OpImage)       { node, x, y, w, h,
                       img (unified image index, -1 unresolved),
                       fit, radius, opacity, smooth }
PathDraw(OpPath)     { node, dx, dy, path (signed path reference),
                       bg_kind, bg, stroke_kind, stroke, stroke_w,
                       dash_on, dash_off, has_dash, opacity }
ClipPush(OpClip)     { x, y, w, h, radius, smooth } · ClipPop
GroupPush(OpGroup)   { node, opacity, blur,
                       mask_kind (0 none|1 solid|2 gradient), mask,
                       mx, my, mw, mh }    · GroupPop
RotatePush(OpRotate) { cx, cy, deg }       · RotatePop
ScalePush(OpScale)   { cx, cy, sx, sy }    · ScalePop
TiltPush(OpTilt)     { cx, cy, rx, ry, depth } · TiltPop
Backdrop(OpBackdrop) { x, y, w, h, radius, blur, saturate,
                       brightness (default 1.0), smooth,
                       mask_kind, mask }

SceneNode { node, parent_ix (scene index, -1 root), kind (SLIR kind),
            x, y, w, h, radius, rot_deg, rot_cx, rot_cy,
            flags, content_main, scroll_off,
            scroll_cross, content_cross, is_row, src_line,
            role, label, desc,
            checked, expanded, selected, active_descendant, controls,
            value_now, value_min, value_max, value_text,
            modal, live, live_atomic, level, pos_in_set, set_size,
            disabled, focused, editable }
```

`OpText.strike` defaults to false. When true, the renderer paints one
line-through across `measured_w`. A renderer without native deterministic text
decoration draws a horizontal rule centered at `y_baseline - 0.3·size`, with
thickness `max(1 device pixel, size/16)`, using the text paint and opacity.

`OpText.uncov_len` counts this op's uncovered-glyph runs; run `i` is the
half-open codepoint range `[Frame.uncovered[uncov_off + 2i],
Frame.uncovered[uncov_off + 2i + 1])` into `strings[str_ref]`. Runs are sorted,
non-overlapping, and coalesced at grapheme-cluster granularity: a cluster is
uncovered when any of its codepoints requires a glyph (`requires_glyph`) that
the op's font cmap does not map. `uncov_len` is `0` for fully covered strings
and whenever `font < 0`. Fallback painters draw exactly these slices — the web
driver with the platform font stack, the native GPU driver as tofu boxes —
inside the kernel-charged fallback advances, so layout never depends on host
fonts. Frame JSON dumps carry the resolved ranges as an optional
`"uncovered":[[start,end),…]` key on Text ops, present only when non-empty.

- **Paint** is a `(kind, handle)` pair instead of a nested enum:
  `0 none | 1 solid (handle = rgba8) | 2 gradient (handle = GRAD index)`.
- `OpPath.path >= 0` indexes the document PATH table. A negative value indexes
  `Frame.paths_rt` as `!path as usize`: runtime entry 0 is encoded as `-1`,
  entry 1 as `-2`, and so on. Each runtime entry uses the same normalized path
  verb/coordinate grammar as a compiled path and is local to this frame.
- Scale, rotation, and tilt commands are balanced, nested painter-stack
  operations. Scale transforms a point around `(cx, cy)` as
  `c + (point - c) * (sx, sy)` until the matching `ScalePop`. Until the
  matching `TiltPop`, the tilted subtree flattens into one plane and warps
  by CSS `perspective(depth)·rotateX(rx)·rotateY(ry)` about `(cx, cy)` —
  ink-only, like scale; hit testing keeps the layout rects.
- `OpGroup.mask_kind`/`mask`, `OpBackdrop.mask_kind`/`mask`, and a gradient
  `OpText.color_kind`/`color` reuse the Paint `(kind, handle)` convention
  above. A group mask multiplies the layer's alpha by the paint's alpha
  mapped over the mask box `(mx, my, mw, mh)`; a backdrop mask scales the
  backdrop effect strength (progressive blur, banded per the support chart).
- `OpGroup.node` owns the node-sized compositing box `(mx, my, mw, mh)`;
  `NONE` marks a host-generated envelope such as a drag ghost. Browser-native
  animation replay targets this stable group for `offset` and `opacity`.
- Every paint op and node-owned group carries `node` (retained-DOM diffing key
  for the web driver; GPU/TUI may ignore it). Node opacity composites through
  `GroupPush`.
- `SceneNode.flags` are the node's *effective* flags: `F_CLIP` is set iff this
  frame clips (authored clip/scroll or boundary-forced), and `F_INERT` is set
  for self-or-ancestor inert. Quarter-turned nodes contribute one scene entry
  carrying `rot_*`.
- `content_main` and `content_cross` are child extents including trailing pad,
  or zero for childless nodes. `scroll_off` and `scroll_cross` are the current
  offsets. `is_row` selects horizontal main/vertical cross when true and
  vertical main/horizontal cross when false; it is retained for dispatch but
  omitted from canonical `frame.json`.
- `role`, `label`, `desc`, `active_descendant`, `controls`, and `value_text`
  index the instance's append-only, deduplicated `St::scene_strs`; zero is
  always empty/absent. References remain stable across solves and dynamic
  value changes. Native hosts resolve numeric refs from that pool; wasm
  `scene_json` resolves them to strings.
- `checked` uses `0 absent | 1 false | 2 true | 3 mixed`.
  `expanded`, `selected`, `modal`, and `live_atomic` use
  `0 absent | 1 false | 2 true`; `live` uses
  `0 absent | 1 off | 2 polite | 3 assertive`. WASM exposes these as nullable
  booleans/string unions rather than numeric codes.
- `value_now`, `value_min`, `value_max`, `level`, `pos_in_set`, and `set_size`
  are `Option<f64>`; WASM and canonical frame JSON emit `null` when absent.
  `disabled` is the resolved per-node `disabled` state and `focused` is exact
  kernel focus ownership for this frame. `editable` is true for a text leaf
  whose `field=` binder is active this frame (conditional binders flip it);
  adapters derive textbox semantics from it without host boilerplate.

## Text metrics (normative for goldens)

Measurement uses SLIR FONT tables only: `advance(gid)·size/upem + tracking`
per codepoint (gid 0 → `default_advance`), `line_h = size·leading`, and the
baseline sits at CSS half-leading over the hhea box:

```
ascent_in_line = asc·size/upem + (line_h − (asc − desc)·size/upem) / 2
```

(research 0.5 centered a fictional 0.76-em box; the real box keeps kernel
baselines aligned with browser-painted glyphs in the web driver). The wrap
algorithm is the research metrics.py port verbatim: greedy break on spaces
(NBSP glues), hard-break of over-long words, ellipsis cut/append with
rstrip, `max_lines` from the height budget. A nowrap paragraph is one
composite line across its styled segments; ellipsis truncation crosses segment
boundaries and appends `…` to the last retained segment so paint style is
preserved without changing the line's layout model.

The selected SLIR `FONT` cmap is the authoritative glyph-coverage contract for
measurement and every rasterizer. A cmap miss has glyph id 0 and charges a
deterministic fallback advance: mono-class families (`FONT.class == 1`) charge
`default_advance` per East-Asian-Width cell — doubled for EAW wide codepoints —
so uncovered CJK and emoji reserve the two terminal cells the cell grid gives
them; every other family charges the single `default_advance`. Glyph modifiers
(ZWJ, variation selectors) still advance zero. The kernel paints no `.notdef`
box itself; instead it marks the affected clusters as uncovered runs on the
Text op (see Frame above) for driver-side fallback paint, and the TUI cell
medium passes the raw codepoints through so the terminal renders them natively.
Runtime text emits the one-shot `glyph-missing` frame diagnostic defined above,
and the cumulative instance set retains it. Compiler-known literal,
parameter-default, and list item-property-default text is checked against the
same cmap at compile time; host-supplied content remains runtime-checked.

### One coordinate space (as built in `textm`)

Every geometric consumer — wrapping, alignment origins, scroll extents,
caret and selection x, pointer hit mapping, and the paint ops' run
extents — lives in the normative advance space above. OpenType shaping
(rustybuzz) selects glyphs and places them *within* their cluster (marks,
ligature glyph position), but cluster and run x-extents are rebased onto
the advance folds after shaping: inter-cluster pair positioning never moves
geometry, RTL runs mirror their logical folds inside the run box, and a
ligature that merges several graphemes into one shaped cluster is split
back to grapheme grain so every caret stop stays addressable. A field's
measured line width therefore always equals the sum of its painted runs'
`measured_w`, and End/pointer round-trips can never leave the measured box.

Shaping is lazy and bounded: measurement never shapes, paint geometry fills
per line on first access (visible, focused, or caret-adjacent lines), plain
lines are retained weakly by the bounded two-generation shape cache, rich
lines by a bounded per-layout FIFO. Editor-shaped fields (no ellipsis,
unbounded lines) memoize wrap results per hard line — keyed by content and
the rebased inline-span signature, validated exactly — and a contiguous
edit re-measures only the hard lines it touched, splicing the untouched
prefix and suffix of the previous layout. All of this is a pure evaluation
strategy: the resulting frames are byte-identical to a cold full measure.

## Motion (as built in `motion`)

"Interpolate inputs, re-solve" (§14): before every solve, motion.apply
writes interpolated attribute INPUTS into the style overlay (attr_val
consults it ahead of patches and base), then layout runs normally — the
containment invariant holds at every instant for free.

- **Keyframes**: per ANIM bind, `p = ease(easing, cycle_progress(t, dur,
  mode, delay))` (research motion.py: once clamps and holds, loop wraps,
  alternate reverses odd cycles; whole-cycle easing); each attribute the
  anim mentions takes `keyframe_value` at p (clamps outside its first/last
  stop; segment-local lerp between stops).
- **Interpolation** (research lerp_raw): numbers/percents lerp; colors
  lerp in OKLab via the target `cbrt` intrinsic (alpha linear; SLIR r-low
  packing byte-swapped at the color-module boundary); equal-length tuples lerp
  elementwise; everything else — mismatched kinds, enum keywords,
  gradients — steps at the midpoint. Gradient STOP ramps stay sRGB
  (a render-time rule; gradients do not tween).
- **Transitions** run on kernel-tracked per-PATCH clocks: when a
  State-cond patch's activity flips between solves, the flip is stamped
  with that solve's `t_ms`; while `age = t − flip − delay < dur` the
  patch's attrs apply as `lerp(base, target, w)` with `w = ease(p)`
  entering and `1 − p` leaving (research resolver.py); attrs with no
  authored base step at the midpoint; flags and extra `when` children
  never tween. Env/Client/W-H conds re-solve without tweening (research
  parity: its prev-env kept renderer + viewport fixed).
  Parameter and state setters only dirty the instance; they do not solve it.
  The first later `inst_frame` stamps an observed flip at that frame's
  `t_ms`. Drive scripts that need a settled snapshot use
  `render` → `clock.advance` → `render`.
- **Liveness**: motion.apply reports "still animating" (running binds,
  in-flight tweens); inst_frame then re-solves whenever the clock moves.
  The manifest's `states_prev`/`state_age` cases realize the research
  build(states, states_prev, state_age) contract as a sequence: solve
  under states_prev at t=0, flip to states (stamped at t=0), sample at
  t=state_age.

## Trace conformance (P5)

`conformance/cases/traces/*.json` replay scripted interaction against both the
native Rust kernel (`slab conformance`) and the same Rust kernel compiled to
Node-bound WebAssembly (`bun tools/conformance-wasm.ts`). Every step first
runs `inst_frame(t)`, then applies exactly one action:

- `event` dispatches an `Event` and dumps one complete `Effects` line;
- `state`, `env`, and `param` call the corresponding host setter;
- `hit` dumps keyed scene ancestry and a bare `tick` performs no mutation;
- `img` registers `name,w,h,format` plus `rgba` or `png_b64`;
- `scroll` sets `key,axis,off`;
- `list` selects `param,path` and performs `op: "len" | "field" | "key"`;
- `divider` sets a keyed extent;
- `reveal` supplies `key,margin`;
- `reveal_item` supplies `each,index,align`; and
- `window` dumps `inst_each_window(each)`.

The output ends with `dumpjson.dump_trace_summary` and the final frame JSON.
The summary contains the focus key, committed field text keyed by Change-signal
name, and key-addressed scroll offsets with an explicit axis. Scroll rows use
scene/document order and main axis before cross axis for the same node.
Everything variable is formatted by the Rust kernel, so native and wasm output
is byte-identical against `conformance/expected/traces/*.trace.txt`.

Dumped signals serialize `name`, `text`, `item`, and the full `meta` object in
the field order specified above. Dumped Effects also serialize `scrolls` after
focus. Embedded signal expectations may select the signal triple, while the
shared golden defends metadata, scroll notifications, and final state.

## frame.json (conformance canonicalization)

`dumpjson::dump(&doc, &st, &frame) -> String` emits one line with no
whitespace and canonical key order:

```
{"width":W,"height":H,"ops":[…],"scene":[…],"strings":[…],"paths_rt":[…],"diags":[…]}
```

- ops use `{"op":"Rect",…}` with payload field order. Paints render as
  `null` | `"#rrggbbaa"` | `"grad:N"`; dash as `null` | `[on,off]`; shadows
  expand inline as `{"x","y","blur","spread","color","inset"}` objects.
  Scale operations serialize as `ScalePush`/`ScalePop`; tilt as
  `{"op":"TiltPush","cx":…,"cy":…,"rx":…,"ry":…,"depth":…}` and
  `{"op":"TiltPop"}`. A negative path
  reference serializes as `"path":"rt:N"` where `N = !path`.
- FX-kit keys are conditional so pre-FX goldens stay stable: Rect appends
  `"smooth":S` after `"opacity"` iff smooth > 0, then `"grain":[amount,size]`
  iff amount > 0. Text `"color"` uses the paint convention above (solid
  `"#rrggbbaa"`, gradient `"grad:N"`) and appends `"grad_box":[gx,gy,gw,gh]`
  after `"opacity"` iff the color is a gradient. Image appends `"smooth"`
  after `"opacity"` and ClipPush after `"radius"`, each iff smooth > 0.
  GroupPush always writes `"node":N` before `"opacity"` and appends
  `"mask":<paint>,"mask_box":[mx,my,mw,mh]` after `"blur"` iff masked.
  Backdrop ALWAYS emits `"brightness":B` after `"saturate"`,
  then `"smooth":S` iff smooth > 0, then `"mask":<paint>` iff masked.
- `paths_rt` entries use `{"verbs":[0,…],"coords":[0,…]}` in frame-local
  index order; the pool contains only geometry referenced by this frame.
- scene entries use `{"node","parent","kind","x","y","w","h","radius","rot",
  "cx","cy","flags","content_main","scroll_off","line","scroll_cross",
  "content_cross","role","label","desc","checked","expanded","selected",
  "active_descendant","controls","value_now","value_min","value_max",
  "value_text","modal","live","live_atomic","level","pos_in_set","set_size",
  "disabled","focused"}`, then append `"editable":true` iff the entry is an
  active kernel-editable field (conditional, so pre-editable goldens stay
  stable). String semantics are numeric `St::scene_strs` refs and
  optional-state enums retain the native codes above; optional numbers are
  numeric or `null`.
- diags use `{"code","line","msg"}` in emission order.
- numbers via `value.fmt3`: round-half-even to 3 decimals by integer math,
  trailing zeros trimmed, `-0 → 0`; panics on NaN/inf.
- strings JSON-escaped (`\" \\ \n \t \r`, `\u00XX` below 0x20).

The native Rust and Node-bound WASM runners emit byte-identical output for the
same SLIR and environment. `slab conformance` and
`bun tools/conformance-wasm.ts` compare frames, traces, capability reports,
and TUI cell output against the same files in `conformance/expected/`: every
manifest case has a `.frame.json` golden, and each TUI case also has a
`.cells.txt` golden.
