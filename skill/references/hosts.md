# Embedding Slab in your app

Contents: [The embedding model](#the-embedding-model) · [Params](#params) ·
[Lists, runs & virtualization](#lists-runs--virtualization) · [Holes](#holes) ·
[Signals & gestures](#signals--gestures) · [Exported defs](#exported-defs) ·
[Runtime images](#runtime-images) · [Web components](#web-components) ·
[Rust hosts](#rust-hosts) · [The kernel Instance API](#the-kernel-instance-api) ·
[Dispatch model](#dispatch-model) · [Focus](#focus) · [Scroll](#scroll) ·
[Divider state](#divider-state) · [Popover anchoring](#popover-anchoring) ·
[Accessibility adapters & scene](#accessibility-adapters--scene) · [Editing](#editing)

## The embedding model

The document declares its host contract in the language: scalar params and
recursive lists (inputs), holes (host-filled viewports), named runtime image
lookups, signals (outputs), exported defs, and accessibility metadata. The
retained scene exports geometry and node metadata. Hosts never parse `.slab`,
mutate its tree, or inject ill-typed values. The 0.5 selector/injection API
(`tpl.frame()`, `f["#id"].set(…)`, `el()`) is removed.

The kernel owns hover, gestures, drag ghosts, focus, scroll, editing, layout,
and dispatch. A driver translates platform input into `Event`, paints `Frame`,
and consumes `Effects`; shipped web/native drivers also maintain the platform
accessibility tree. App policy stays in the host and reacts to signals.

```slab
def Row(label) export { text label }

params {
  title   text = "Settings"
  draft   text = ""
  level   pct  = 30%
  tone    color = #4FC7E0
  compact bool = false
  density enum(cozy, compact) = cozy
  rows    list(Row) = [Row(label="Alpha"), Row(label="Beta")]
}

text param.title size=22 weight=600
text#field param.draft field=draft w=300 h=32     // kernel-edited
col { each param.rows }                            // typed list instancing
col#panel clip { hole extra w=fill h=336 scroll }  // host-filled viewport
```

`examples/10-settings.slab` (buttons + field + hole) and
`examples/12-tracklist.slab` (list/each + themes + scrollbar) are the
canonical references.

## Params

Seven param types: `text num pct color bool enum(a,b,…) list(Def)`. Every
default is required and type-checked (`err[param-type]`).

- Reference a scalar as `param.NAME` at a whole-value site. Numeric `num`/`pct`
  refs may also occupy numeric tuple members such as `offset=param.x,param.y`.
  Wrong-type uses are `err[ref]`; a List param is consumed only by root `each`.
- Use a Bool param directly in `when compact { … }`; non-Bool conditions are
  `err[param-type]`.
- A successful changed setter dirties the instance; equal writes are no-ops.
- Web exposes observed kebab-case attributes plus typed properties. Rust emits
  typed setters. CLI/TUI use `--set param=value`; invalid names, values, or
  enum members reject the write.

**Display strings are host-computed.** `when` patches attrs and injects
conditional children; the language has no ternary or content swap. Precompute
every conditional display string — checkbox glyphs, priority labels, timer
text — into list fields or params in the host, and treat a re-skin of those
strings as a host change. Keep the document declarative over the data it is
given.

## Lists, runs & virtualization

Declare nested list fields on exported defs with `list(Def)`. Schemas may be
mutually recursive or self-recursive:

```slab
def Tree(label="", children=list(Tree)) export {
  col gap=4 {
    text label
    col pad=0,0,0,12 { each children }
  }
}
params {
  roots list(Tree) = [
    Tree(label="src", children=[
      Tree(label="main.rs"),
      Tree(label="ui", children=[Tree(label="panel.rs")])
    ])
  ]
}
col#tree { each param.roots }
```

Use `each param.roots` only for a root List param; inside its template use
`each children` for a List-typed item prop. A template may contain nested
`each` but never `hole`. Nested defaults use the same typed calls recursively.
Data depth, not macro expansion, bounds recursion.

Use a direct `each` in `para` for rich runs. Its schema def must expand to
exactly one `span`; see language.md for the minimal run example.

Virtualize a uniform root list in the kernel:

```slab
def FeedRow(label="") export { row h=20 { text label } }
params { rows list(FeedRow) = [] }
col#feed h=320 scroll scrollbar=auto {
  each param.rows key=rows virtual item-extent=20 overscan=8
}
```

`virtual` is legal only on a direct, non-nested root-param `each` under a
main-axis scrolling `row`/`col`. A positive numeric-literal `item-extent` is
required; nonnegative literal `overscan` defaults to 4. With retained offset
`off`, viewport `vp`, and extent `e`, the half-open window is
`[floor(off/e)-overscan, ceil((off+vp)/e)+overscan)`, clamped to `[0,len)`.
Implicit leading/trailing extent keeps `content_main = len*item-extent`.
Before viewport geometry exists, the first frame uses at most `overscan*2`
items, then settles.
Unmaterialized items keep identity/state but cannot receive focus;
truncation alone prunes them. V1 has uniform extents, no variable-height
measurement, and no automatic scroll anchoring after list mutation.

Low-level list APIs require a `path` on every call. Use `""` for the root;
otherwise use `<index>.<field>` pairs such as `"3.children"` or
`"3.children.0.tags"`. Paths use indices, not item keys. A scalar path hop,
malformed path, out-of-range item, bad value type, or attempt to write a List
field through `inst_set_list_field` fails atomically.
`inst_list_len` returns `-1` on resolution failure; setters return `false`.
Growing a list seeds recursive schema defaults; truncation prunes descendants
and their keyed state.
Complete key swaps before solving. An omitted key defaults to the decimal
index; stable explicit nonempty keys preserve state across reorder.

Bulk inputs recursively prevalidate the complete value. Web root properties
and JSON attributes accept nested plain objects; use
`setList(name, path, value)` for a subtree. Generated Rust emits recursive
`Vec<...Item>` fields and `set_<param>(&[...])`. CLI/TUI accept nested JSON.
`sig_item` remains the innermost item key; `SignalMeta.key` carries the full
nested synthetic path.

For virtual navigation, call `inst_reveal_item` / `revealItem` with alignment
`0 start | 1 center | 2 end | 3 nearest`. Query the materialized half-open
range with `inst_each_window` / `eachWindow`. Unknown/non-virtual keys,
invalid indices, and invalid alignments make reveal return `false`; an
unknown/non-virtual window is `(-1,-1)`.

## Holes

`hole NAME` reserves a rectangle the HOST fills: web slots real DOM into it,
native mounts a child kernel instance, static exporters leave it empty, TUI
reports `cap-hole`. Duplicate names are `err[dup-hole]`.

- Sizing: fixed, `fill`, `%`, param, or `hug`. A `hug` axis uses the host's
  persistently reported natural content size (0 before the first report),
  then ordinary min/max clamps.
- The sanctioned loop: solve → read `inst_holes()` →
  `HoleRect { hole, x, y, w, h, clip }` → measure host content → report via
  `inst_set_hole_size` → re-solve once. Equal re-reports don't dirty, so a
  stable size converges without a loop.
- Web: each hole is a named `<slot name="NAME">` positioned over the rect;
  `scroll` holes scroll natively in the host DOM.
- Native: the `HoleContent` trait; the shipped `InstanceHole` mounts a child
  `Instance` clipped into the hole rect.

## Signals & gestures

Signals are the document's only app outputs. Bind them on the authored node,
never inside `when`; deferred binders warn `attr` and are ignored.

- `act=NAME`: Activate (trigger 0), ordinary keyboard/pointer activation.
- `field=NAME`, `submit=NAME`: Change (1) / Submit (2), committed text.
- `press=NAME`: Press (3), primary pointer-down before capture.
- `context=NAME`: Context (4), secondary down without focus/pressed effects.
- `dblclick=NAME`: Dblclick (5), double down; suppresses that gesture's Activate.
- `drag=NAME`: DragStart (6), after captured movement exceeds 4u.
- `drop=NAME`: Drop (7), on the deepest eligible target.
- `resize=NAME`: Resize (8), `fmt3(final_extent)` as text.
- `pointer-move=NAME`: PointerMove (9), every dispatched move on the deepest
  enabled hit binding, or the captured owner's path while captured.
- `pointer-up=NAME`: PointerUp (10), once on primary release, routed through
  the captured path when present and otherwise the current hit path.
- `drag-update=NAME`: DragUpdate (11), on the threshold-crossing move and each
  later move, emitted by the drag source.
- `drag-end=NAME`: DragEnd (12), exactly once from the source on release or
  cancellation.

Drivers may coalesce hardware motion, so “every move” means every forwarded
dispatch. Use `when hover` for paint-only feedback. On an ordinary click,
PointerUp precedes Activate; it still follows capture when released outside,
while Activate requires the release hit path to contain the pressed node.

`act`/`field`/`press`/`drag` imply `focusable`. Change, Submit, and Resize
carry `text`; all signals carry `item` and typed `meta`. The exact metadata is:

```text
SignalMeta {
  x,y,dx,dy,drag_dx,drag_dy: f64;
  mods,button,clicks: u32;
  key,src_key,src_item: String;
  cancelled,dropped: bool;
}
```

`key` is the emitter's full node path. `dx/dy` are this event's deltas;
`drag_dx/drag_dy` are cumulative from the arming down and DragEnd carries the
final displacement, including on cancellation. Nonapplicable numbers are zero
except keyboard `x/y=-1`; booleans default false. `item` is the emitter's
innermost item, so Drag* source identity is `key` + `item`. Only Drop fills
`src_key/src_item`; both Drop and a successful DragEnd set `dropped=true`.

```slab
row#card press=select pointer-move=card_move pointer-up=card_up \
    drag=drag_started drag-update=drag_moved drag-end=drag_finished drag-ghost {
  text "release/1.0"
  when dragging { opacity=0.55 }
}
col#trash context=trash_menu drop=dropped {
  text "Drop to delete"
  when drop { stroke=#EF4444 stroke-w=2 }
}
```

```js
host.addEventListener('drag_moved', ({ detail: { meta } }) =>
  updateDragTelemetry(meta.x, meta.y, meta.drag_dx, meta.drag_dy));
host.addEventListener('drag_finished', ({ detail: { meta } }) => {
  if (meta.cancelled) rollback();
  else if (meta.dropped) commitDrop();
});
```

Drag arms on primary down, starts only beyond 4u, and suppresses Activate.
Targets exclude the source subtree. Ordinary primary release emits DragEnd
with `cancelled=false`; `dropped` says whether Drop accepted. A new down,
blur, close, source invalidation, or list pruning emits one cancelled DragEnd
(`cancelled=true,dropped=false`). Release/cancel always clears
`dragging`/`drop`.
Signal order is stable: move emits PointerMove, then optional DragStart, then
DragUpdate; active moves emit PointerMove then DragUpdate. Release emits
PointerUp, optional Drop, then DragEnd. Abnormal cancellation contributes only
its cancelled DragEnd; unrelated signals from that host event are unaffected.

Add `drag-ghost` only with `drag=` to duplicate the source subtree at the
pointer while preserving its grab offset. The kernel paints it at opacity
0.72 above ordinary ops and excludes it from scene, hit testing, and a11y.
Do not implement a parallel host ghost. Web signals remain bubbling, composed
`CustomEvent`s with `detail={item,meta[,text]}`; generated Rust uses one shared
`SignalMeta` on every `Signal` variant.

## Exported defs

`def Row(label, tone) export { … }` compiles a standalone document and makes
the def a List schema. Scalar props infer fields from use sites; explicit
`child=list(Child)` props create nested schemas and may recurse. Generated
web/Rust types mirror the entire recursive shape. Prefer explicit defaults;
otherwise scalar fields use their type-zero value and list fields use `[]`.

## Runtime images

Bind `img src` to a Text param or item prop, then register matching bytes:

```slab
params { avatar text = "user:42" }
img src=param.avatar w=40 h=40 radius=20 fit=cover
```

`w` and `h` must be nonzero. `format=0` is PNG; it must fully decode to those
dimensions. `format=1` is straight-alpha sRGB RGBA8 and requires exactly
`w*h*4` bytes. Runtime names win over compiled images. Re-registering
a name keeps its unified index and bumps generation only when content or
active state changes; an equal registration is a clean no-op. Unregistering
preserves the slot and falls back to a same-name compiled image. An unresolved
name keeps layout/scene, suppresses the Image op, and warns `img-missing` once
per unique name. See rendering.md for client caching and TUI degradation.
`inst_img_info` returns `(w,h,format,generation)` only for active unified
indices; compiled images have generation zero. `inst_img_bytes` returns an
empty slice for unknown/inactive indices.

## Web components

```sh
bunx @stencil-hq/slab gen wc doc.slab -o dist --tag my-doc
```

Keep the generated module, `slab-runtime.js`, and kernel WASM together. The
element loads asynchronously and fires `slab-frame` after each paint; wait for
the first one before image registration, reveal, window, or scene queries:

```html
<my-doc id="host" style="display:block;width:800px;height:640px"></my-doc>
<script type="module">
  import './dist/doc.js';
  const host = document.getElementById('host');
  await new Promise(resolve =>
    host.addEventListener('slab-frame', resolve, { once: true }));

  host.roots = [{
    key: 'src', label: 'src',
    children: [{ key: 'main', label: 'main.rs', children: [] }]
  }];
  host.setList('roots', '0.children', [
    { key: 'lib', label: 'lib.rs', children: [] }
  ]);

  host.addEventListener('dropped', ev => {
    const { item, meta } = ev.detail;
    console.log(item, meta.key, meta.src_key, meta.src_item, meta.mods);
  });

  host.imgRegister('user:42', 1, 1, 1,
    new Uint8Array([79, 199, 224, 255]));
  const rowsPainted = new Promise(resolve =>
    host.addEventListener('slab-frame', resolve, { once: true }));
  host.rows = Array.from({ length: 100 }, (_, i) =>
    ({ key: `row-${i}`, label: `Row ${i}` }));
  await rowsPainted;
  host.setScroll('#feed', 0, 120);       // 0 main, 1 cross
  host.revealItem('#feed/rows', 90, 3);  // nearest; row 90 need not be mounted
</script>
```

The generated element surface uses these exact clean-cutover signatures:

```ts
setParam(name: string, value: unknown): boolean
setList(name: string, path: string, value: unknown): boolean
getList(name: string, path: string): unknown
setFieldText(key: string, text: string): boolean
fieldText(key: string): string | undefined
getToken(path: string): string | number | undefined
imgRegister(name: string, width: number, height: number,
            format: number, bytes: Uint8Array): number
imgUnregister(name: string): boolean
imgInfo(index: number): readonly [number, number, number, number] | null
imgBytes(index: number): Uint8Array
setScroll(key: string, axis: number, off: number): boolean
getScroll(key: string, axis: number): number
reveal(key: string, margin: number): boolean
revealItem(each: string, index: number, align: number): boolean
eachWindow(each: string): readonly [start: number, end: number]
setDivider(key: string, extent: number): boolean
getDivider(key: string): number
setFocus(key: string, visible?: boolean): boolean
sceneSnapshot(): readonly SceneNode[]
```

`imgInfo` tuple order is width, height, format, generation. `imgRegister`
returns `-1` before the instance exists or for invalid bytes.
Param/list/scroll/divider writes made before initialization are buffered.
`setFieldText` requires a mounted field; `fieldText` returns `undefined` for
an unknown or non-editable key. `getToken` reads the generated component's
fully resolved active-theme table. It returns CSS color strings, numbers for
numeric tokens, and `undefined` for unknown paths. Arbitrary SLIR loaded with
`loadSlir` has no token metadata until a later SLIR revision.
`hole`s remain named slots; scene snapshots resolve a11y fields to strings.

Bundlers: `slab-runtime.js` resolves the kernel WASM via
`new URL('./wasm/slab_kernel_bg.wasm', import.meta.url)`. After bundling,
`import.meta.url` is the bundle URL, so the relative fetch can 404. Either
copy the generated `wasm/` directory next to the served bundle, or add a
server route mapping `/wasm/*` to the generated output's `wasm/` directory.
A load failure logs the attempted URL and this bundler remedy, then renders a
visible `role=alert` error inside the element.

Web editing uses an invisible textarea at the kernel IME rectangle. The
component forwards `compositionstart`, `compositionupdate`, and
`compositionend`, suppresses composing key events and the browser's duplicate
post-composition insertion, and refreshes the textarea from kernel field
state. Cmd/Ctrl-A, C, X, and V keep the hidden selection, system clipboard,
and kernel field synchronized.

Secondary pointer down reaches the kernel as `button=2` and does not clear
field focus or selection. Before `contextmenu`, the component places the
invisible textarea under the pointer while preserving its selection. The
uncancelled browser event therefore targets the native editor and exposes the
browser's text actions. The textarea returns to the kernel IME rectangle after
the event. An authored `context=field_menu` still emits its named
`CustomEvent`; do not add a second pointer handler or cancel that signal.

## Rust hosts

`slab gen rust FILE -o OUT.rs` emits a typed `Doc` with scalar setters,
recursive `<Param>Item` structs plus `set_<param>`, a typed `Signal` enum with
shared `SignalMeta`, and these clean-cutover methods:
`set_scroll(key,axis,off)`, `get_scroll(key,axis)`,
`set_field_text(key,text)`, `field_text(key)`,
`img_register(name,w,h,format,data)`, `img_unregister(name)`,
`reveal(key,margin)`, `reveal_item(each,index,align)`, `each_window(each)`,
`set_divider(key,extent)`, `get_divider(key)`, `set_focus`, `holes`, `frame`,
and `dispatch`. `Doc.inst` remains public for the complete kernel API and
scene-string pool. `crates/slab-native` is the reference winit/wgpu driver;
`slab-tui` is the reference terminal driver.

### Depending on Slab

Pin every Slab crate to the same Git revision. The generated Rust document
imports `slab-kernel` and `slab-slir`. Native hosts import `slab-native`.
Terminal hosts import `slab-tui`. Add `slab-compile` only when the host
compiles source or uses `apply_sets`:

```toml
[dependencies]
slab-native = { git = "https://github.com/stencil-hq/slab", rev = "<SAME_COMMIT>" }
slab-tui = { git = "https://github.com/stencil-hq/slab", rev = "<SAME_COMMIT>" }
slab-kernel = { git = "https://github.com/stencil-hq/slab", rev = "<SAME_COMMIT>" }
slab-slir = { git = "https://github.com/stencil-hq/slab", rev = "<SAME_COMMIT>" }
slab-compile = { git = "https://github.com/stencil-hq/slab", rev = "<SAME_COMMIT>", optional = true }
slab-drive = { git = "https://github.com/stencil-hq/slab", rev = "<SAME_COMMIT>", optional = true }
```

Replace `<SAME_COMMIT>` with one full commit hash. Do not mix revisions because
the generated document, kernel event constants, and frame structures form one
contract.

### Driving and testing

Use `slab-drive` to mount the Slab Drive Protocol (SDP) on the application's
live `Instance`. `RequestPump` borrows the instance for one request only.
The host keeps ownership between requests and runs its normal signal handler:

```rust
let mut pump = slab_drive::RequestPump::new("app.slab", slir, images);
let result = pump.request(&mut doc.inst, request_line);
for effects in result.effects {
    app.handle_effects(&mut doc.inst, effects)?;
}
write_response(result.response)?;
```

This pattern tests real app policy. Input methods dispatch through the live
kernel, then the host applies emitted signals to its model. Use
`slab_drive::serve` for a blocking NDJSON loop. Use `RequestPump::request` from a
window or terminal event loop.

`param.set` remains a deferred input write. It does not solve immediately.
A transition flip starts at the first solve that observes the changed value.
For a settled snapshot, use `render` → `clock.advance` → `render`. The first
render observes the flip, the advance moves its clock, and the second render
captures the new transition position.

### Terminal hosts

`slab-tui` is an embeddable library and a command. `Terminal` owns raw mode,
the alternate screen, mouse capture, and safe teardown. `Painter` emits cell
diffs and preserves terminal-default colors. `Translator` maps crossterm input
to kernel events. Its retained state supplies click counts and pointer deltas.
The `resize` helper applies cell dimensions to the kernel environment.

Use `run` when the host only needs signal and clock callbacks:

```rust
use slab_tui::{Host, ImageMode, Images, Signal, Ui};

struct App {
    last_signal: String,
}
impl Host for App {
    fn on_signal(
        &mut self,
        inst: &mut slab_kernel::frame::Instance,
        signal: &Signal,
    ) -> Result<(), String> {
        self.last_signal.clone_from(&signal.name);
        let _ = inst;
        Ok(())
    }
}

let mut app = App { last_signal: String::new() };
let images = Images::new(
    ImageMode::Off,
    &inst.doc,
    &[],
    std::path::Path::new("."),
);
let ui = Ui {
    fps: 30.0,
    debug: false,
    dark: true,
    coarse: false,
    gallery: None,
};
slab_tui::run(&mut inst, &mut app, images, &ui)?;
```

Use `Terminal`, `Painter`, `translate`, and `resize` directly for a custom
event loop. The library keeps layout, editing, focus, and hit testing inside
the kernel.

### Native input, IME, clipboard, and accessibility

Use `slab_native::input` instead of copying reference-host code.
`ClickCounter`, `key_name`, `mouse_button_id`, `cursor_delta`, and
`cursor_icon` define the native input mapping. Compute coordinates and deltas
in document units. Forward secondary pointer-down with `button=2` before
applying any host context-menu policy.

Keep one `input::ImeState` per window. Pass every `WindowEvent::Ime` to
`ImeState::on_ime` and dispatch each returned `(etype, text)` pair in order.
While `composing()` is true, suppress raw key events. Forward
`KeyboardInput.text` only when `forwards_key_text()` is true. This rule prevents
an `Ime::Commit` and its raw key event from delivering the same text twice.
After each dispatch, use this effects recipe:

```rust
ime.set_allowed(window, slab_native::input::focus_in_field(&doc.inst));
ime.sync_rect(window, &effects);
```

`ImeState` translates Enabled, Preedit, Commit, and Disabled into composition
start/update/end or plain text events. Commit ends the composition and clears
preedit state. `sync_rect` passes changed kernel candidate rectangles to winit.
Translation tests cannot select a macOS input method. Before release, a human
must select a CJK input method and smoke-test preedit, candidate placement,
commit, cancellation, blur, and refocus in a real window.

The kernel edits selection but never accesses the operating-system clipboard.
Use `input::selection_text` plus `input::Clipboard::write` for copy. For cut,
write the selection first and dispatch `E_CUT`. For paste, read the clipboard
and dispatch `E_PASTE` with its text. Cmd/Ctrl shortcuts and the visual context
menu remain host policy. The reference native player shows a title-bar
affordance after right-click: C copies, X cuts, V pastes, and Escape closes it.

Mount accessibility with `slab_native::a11y::WindowAccessibility`. Create an
`EventLoop<a11y::Event>`, then create the bridge after the window in
`ApplicationHandler::resumed`. Forward every `WindowEvent` through
`process_event`. After each settled frame, call `refresh` with one or more
`SceneLayer` values, then call `update(false)`. Handle
`EventKind::InitialTreeRequested` with `update(true)`. Resolve
`EventKind::ActionRequested` through `resolve_action`, apply the returned action
to its identified document, and dispatch `ActionResult::Dispatch` through the
generated document wrapper so typed signals remain available.

For untyped bulk input from a Rust host, use
`slab_compile::input::apply_sets(&mut inst, &sets)` with
`("name", "value")` pairs — the same path as CLI `--set`. It coerces strings
per param type (colors like `"#4FC7E0"`, nested list JSON with per-item
`key`), validates the complete value, and applies atomically; use
`slab_compile::input::coerce_scalar(kind, raw)` for one scalar. Prefer it
over raw `inst_set_list_*` when values arrive as strings/JSON; prefer
`gen rust` typed setters when the host owns typed state.

## The kernel Instance API

`spec/FRAME.md` is normative. The changed native Rust signatures are:

```rust
fn inst_list_len(i: &Instance, param: u32, path: &str) -> i32
fn inst_set_list_len(i: &mut Instance, param: u32, path: &str, n: i32) -> bool
fn inst_set_list_field(i: &mut Instance, param: u32, path: &str, index: i32,
                       field: &str, value: &ParamValue) -> bool
fn inst_set_list_key(i: &mut Instance, param: u32, path: &str, index: i32,
                     key: &str) -> bool

fn inst_img_register(i: &mut Instance, name: &str, w: u32, h: u32,
                     format: u32, data: &[u8]) -> i32
fn inst_img_unregister(i: &mut Instance, name: &str) -> bool
fn inst_img_info(i: &Instance, image: i32) -> Option<(u32,u32,u32,u32)>
fn inst_img_bytes(i: &Instance, image: i32) -> &[u8]

fn inst_set_scroll(i: &mut Instance, key: &str, axis: u32, off: f64) -> bool
fn inst_get_scroll(i: &Instance, key: &str, axis: u32) -> f64
fn inst_reveal(i: &mut Instance, key: &str, margin: f64) -> bool
fn inst_reveal_item(i: &mut Instance, each_key: &str, index: i32,
                    align: u32) -> bool
fn inst_each_window(i: &Instance, each_key: &str) -> (i32, i32)
fn inst_set_divider(i: &mut Instance, key: &str, extent: f64) -> bool
fn inst_get_divider(i: &Instance, key: &str) -> f64
fn inst_set_field_text(i: &mut Instance, key: &str, text: &str) -> bool
fn inst_field_text(i: &Instance, key: &str) -> Option<String>
fn inst_focus(i: &Instance) -> u32
fn inst_param_json(i: &Instance, name: &str) -> Option<String>
```

Root-list `path=""`, scroll `axis`, and Event `clicks` are required; old
pathless/axisless shapes have no compatibility overload. Setters are total,
atomic, and dirty only on an actual change. `inst_set_field_text` returns
`false` for unknown or non-field keys. It works while focused or blurred,
resets composition, selection, and undo/redo, places the caret at the end,
synchronizes a same-named Text param, and queues Change for
`inst_take_signals`.

```rust
struct Event {
  etype: u32, x: f64, y: f64, dx: f64, dy: f64,
  button: u32, clicks: u32, key: String, text: String, mods: u32,
}
struct SigMeta {
  x: f64, y: f64, dx: f64, dy: f64, drag_dx: f64, drag_dy: f64,
  mods: u32, button: u32, clicks: u32,
  key: String, src_key: String, src_item: String,
  cancelled: bool, dropped: bool,
}
struct ScrollChange { key: String, axis: u32, off: f64 }
struct Effects {
  repaint: bool,
  sig_name: Vec<u32>,       // document STRS refs
  sig_text: Vec<String>,
  sig_item: Vec<String>,
  sig_meta: Vec<SigMeta>,   // all four arrays have equal length
  scrolls: Vec<ScrollChange>,
  has_caret: bool, caret_x: f64, caret_y: f64, caret_w: f64, caret_h: f64,
  has_ime: bool, ime_x: f64, ime_y: f64, ime_w: f64, ime_h: f64,
  cursor: u32, focus: u32,
}
```

`mods` bits are `1 shift | 2 alt | 4 ctrl | 8 meta`; cursors are
`0 default | 1 pointer | 2 text | 3 col-resize | 4 row-resize`.
`focus=0xFFFFFFFF` means none; honor caret/IME rectangles only when their
`has_*` flag is true.
The WASM `KInst` mirrors these as snake-case methods (`set_field_text`,
`field_text`, `focus`, `param_json`, `set_list_len`, `img_register`,
`set_scroll`, `reveal_item`, `each_window_json`, …). Its exact event call is
`dispatch_json(type,x,y,dx,dy,button,key,text,modifiers,clicks)`; the returned
JSON contains the complete Effects shape.

## Dispatch model

Drivers forward pointer, wheel, key, text, paste/cut, composition, blur,
resize, close, and inspect events; Activate is synthesized internally. There
is no DOM-style capture/bubble or handler registration.

`etype` codes are `0 move | 1 down | 2 up | 3 wheel | 4 key-down | 5 text |
6 paste | 7 copy | 8 cut | 9 composition-start | 10 composition-update |
11 composition-end | 12 blur | 13 resize | 14 close | 15 inspect`;
`16 activate` is internal and ignored from outside.

Forward host-computed `clicks` on pointer-down (`0/1` single, exactly `2`
double); web uses `PointerEvent.detail`. A native counter should match the
reference window: same button, at most 500ms, and at most 4u from the previous
down.

Primary (`button=0`) down fires `press`, arms the deepest `drag`, captures,
and focuses. Secondary (`button=2`) down fires `context` without press/focus.
A bound double down fires `dblclick` and suppresses later Activate. Forward
each move's event-local `dx/dy`; the kernel routes PointerMove, computes
cumulative drag displacement, starts DragStart beyond 4u, and emits
DragUpdate. Primary up routes PointerUp, may Drop, then emits DragEnd and
clears gesture state. Blur/close cancel an active drag. Always forward
document-space coordinates and current modifier/button/click fields so
`SignalMeta` is trustworthy.

Key routing precedence is field editing → focused divider adjustment →
focused scrolling → `keys=` → Enter/Space activation → focus navigation.
`keys=Escape,F2` walks from the focused node through ancestors to the first
enabled match. Interaction styling stays in the template with
`when hover/pressed/focus-visible/disabled/dragging/drop`.

## Focus

Document order is tab order. Tab/Shift-Tab walk the ring; arrows also walk
when the focused node is neither an edit field, divider, nor scrollable on
that axis. Keyboard focus sets `focus-visible`; pointer focus sets only
`focus`. Vanished focus restores to the nearest following, then preceding,
focusable. Virtual-list traversal includes only materialized items. Use
`inst_set_focus(i,key,visible)` / web `setFocus` for host-driven dialogs;
focus traps and restoration policy remain host-owned.

**Host key layer** (per-key actions on the focused row, the TUI list-app
staple): `keys=` only adds activation keys for one `act=` signal, so distinct
per-key actions are a host recipe. Intercept printable keys before dispatch
when focus is not in an edit field; query focus with `inst_focus(i)` / web
`focus()` (`0xFFFFFFFF` = none); resolve the focused node to its item with
`list::item_key(&inst.st.lists, &inst.doc, node)` and to its full key with
`scene::key_of`. Note host `inst_set_focus`/`setFocus` does NOT auto-reveal
(Tab traversal does): call `inst_reveal(key, margin)` / `reveal` after
focusing an off-screen row.

## Scroll

Offsets are kernel-owned and key-addressed per axis: `0` main, `1` cross.
Bare `scroll` activates main; `scroll=cross` cross; `scroll=both` both. Wheel
routes `dy` to the deepest main owner and `dx` to the deepest cross owner;
Shift swaps the deltas. Focused keyboard scrolling remains main-axis only.
Main-axis arrows step 40u (200u with Shift); PageUp/Down use
`viewport-40u`, and Home/End select zero/maximum.
Every actual dispatch change appends
`ScrollChange {key,axis,off}` to `Effects.scrolls`; direct setters do not.

Use `inst_set_scroll(i,key,axis,off)` / web `setScroll`; it returns false for
an unknown key, invalid axis, or inactive axis. Reads return `0` for unknown
keys/axes, and valid offsets clamp to retained `content_main` or
`content_cross`. Prefer `inst_reveal(i,key,margin)` / web `reveal`: it
minimally moves both active axes through every scroll ancestor. It returns
false unless the target exists in the retained scene; negative/non-finite
margin behaves as zero. Use `reveal_item` for an unmaterialized virtual row.

`sticky` is a direct main-scroll child only. It paints above normal siblings,
is pushed by the next sticky child, and keeps painted geometry for hit tests.
Cross/end sticky is unsupported. SVG/PNG do not dispatch scroll and report
`cap-scroll`; read rendering.md for static scrollbar behavior.

## Divider state

`divider` controls its previous sibling. Set an initial/restored finite extent
with `inst_set_divider(i,key,extent)` / web `setDivider`; read `-1` for unknown
or unset. The kernel clamps to the previous pane's min/max and preserves the
next pane's minimum. Pointer moves re-solve continuously; pointer-up and every
keyboard step emit `resize` with `detail.text` / `sig_text` as the final
extent. Double-click clears the overlay and emits optional `dblclick`.
Do not implement a parallel host drag loop. Express collapse with params and
`when`; compute content-aware initial allocation in the host.

## Popover anchoring

Author the overlay as a direct `stack`/`canvas` child with
`attach=param.anchor`. On its opening signal, feed back the emitter's exact
full key and toggle a Bool param:

```js
host.addEventListener('open_menu', ev => {
  host.setParam('anchor', ev.detail.meta.key);
  host.setParam('menu_open', true);
});
```

The kernel follows scrolled anchors and omits a missing-anchor subtree from
paint and hit testing. Keep outside-click dismissal, focus trapping, and
focus restoration in the host; use `setFocus` and the retained scene rather
than inventing overlay coordinates.

## Accessibility adapters & scene

Author the full semantic contract (`role`, name/description, state, relations,
values, set metadata, modal/live metadata) in Slab; see language.md. This does
not add visuals, focusability, or actions—bind those explicitly.

Application hosts do **not** rebuild platform nodes from `sceneSnapshot()`.
The shipped web component maintains a retained, pointer-transparent,
opacity-zero shadow semantic DOM, maps scene state to ARIA, assigns
deterministic DOM ids, resolves exact-key relationships, and mirrors kernel
focus. The shipped native client maintains the equivalent AccessKit tree.
Native publication also includes parent/children, bounds/scale,
focusability/inertness, and keyed scroll offsets/ranges from the same scene.

The native bridge maps AccessKit Focus to `inst_set_focus`; default Click to
focus plus the existing Enter activation dispatch; divider Increment/
Decrement to orientation-correct Arrow dispatch; ScrollIntoView to
`inst_reveal`; and directional/SetScrollOffset actions to keyed active-axis
`inst_get_scroll`/`inst_set_scroll`. It omits unsupported SetValue. Authored
activation/resize still arrives as ordinary Slab signals for app policy.
The web semantic layer publishes tree/state and mirrors focus; it does not
invent generic default, increment, or set-value behavior.

Custom drivers must wire an equivalent platform adapter from the retained
parent hierarchy, bounds, focus, and these exact `SceneNode` fields:

```text
role label desc checked expanded selected active_descendant controls
value_now value_min value_max value_text modal live live_atomic
level pos_in_set set_size disabled focused
```

Native `role/label/desc/active_descendant/controls/value_text` are refs into
`inst.st.scene_strs` (`0` empty). Stable codes:
`checked` = `0 absent, 1 false, 2 true, 3 mixed`;
`expanded/selected/modal/live_atomic` = `0 absent, 1 false, 2 true`;
`live` = `0 absent, 1 off, 2 polite, 3 assertive`.
Value/range/level/set numbers are `Option<f64>`; `disabled/focused` are
kernel-derived Bool.
WASM scene JSON resolves refs to strings and optionals to values/null;
generated `sceneSnapshot()` exposes the typed form.
Web uses `boolean|'mixed'|null` for checked, `boolean|null` for optional
Bool state, the named live union or null, `number|null` for optional numbers,
and `""` for absent strings.
Use snapshots for inspectors/app queries, not to duplicate the shipped AT
tree. `spec/FRAME.md` is the normative custom-driver ABI.

## Editing

Kernel-owned on `field=` text nodes; single-line unless flagged `multiline`.

- Grapheme-cluster caret/selection (UAX #29 subset), bounded undo/redo
  (100 snapshots, coalesced same-kind groups), horizontal viewport scroll
  with an 8u caret inset; multiline scrolls the nearest `scroll` ancestor
  to reveal the caret instead.
- Enter matrix: multiline without `submit=` → newline (plain/Shift/Alt);
  multiline with `submit=` → Enter submits, Shift/Alt-Enter newline;
  single-line with `submit=` → Enter submits; without → inert. Submit
  carries the full text and does not also fire Change.
- Word ops: Ctrl/Alt-Backspace/Delete delete words; Ctrl-K kills to
  visual-line end, Ctrl-U to start; Ctrl/Meta-A selects all; Ctrl/Meta-Z /
  Shift-Z undo/redo. Arrows move by cluster/word/document; multiline
  Up/Down move by visual line with goal-x.
- The embedding owns IME plumbing and the clipboard. Web positions a hidden
  textarea from each refreshed IME rectangle and forwards the full composition
  lifecycle without duplicate key/text delivery. Native forwards winit IME;
  kernel cut/copy touch no system clipboard, and GPU clipboard degradation is
  charted. Composition drives the `composing` node state. Caret/IME rects in
  `Effects` describe the LAST solve — refresh after the next frame.
- Clearing or seeding a field: `field=draft` binds an INITIAL value. The
  EditState is keyed persistent state, so ordinary param writes never reseed
  it. Blur/refocus also preserves it. Use
  `inst_set_field_text(i, key, text)` / web `setFieldText(key, text)` /
  generated Rust `set_field_text` to replace the buffer. The call works while
  focused or blurred. It resets selection and undo/redo, moves the caret to
  the end, synchronizes the same-named Text param, and emits Change through
  the next `inst_take_signals` Effects. Normal field mutations also synchronize
  that param. Do not echo Change into the param or rotate item keys.
