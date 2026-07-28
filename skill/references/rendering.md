# Slab pipeline, clients & CLI

Contents: [Pipeline](#pipeline) · [Frame ops](#frame-ops) ·
[Dynamic driver obligations](#dynamic-driver-obligations) · [Units](#units) ·
[Support chart & degradation](#support-chart--degradation) ·
[TUI authoring](#tui-authoring) · [CLI](#cli) · [Conformance](#conformance) ·
[Repo map](#repo-map)

## Pipeline

```
.slab text ──slab-compile──▶ SLIR (protobuf + raw Snappy, 8-byte envelope)
          ──slab-kernel────▶ Frame (draw ops + scene)
          ──thin driver────▶ web (WASM) | wgpu | tui | svg | png | apng
```

Compile time folds token refs, styles, component expansion, static paths/icons,
and shadow presets. Complete FONT metric/coverage tables keep dynamic host text
deterministic. The kernel retains everything else dynamic: `when`, animation,
scalar params, recursive list schemas/templates, virtual windows, runtime path
strings, image-name lookups, holes, signals, scroll,
divider overlays, anchored placement, and accessibility strings. Clients
receive SLIR and frames, never `.slab`; a driver performs no layout policy.

Normative docs: `spec/SLIR.md` (compiled wire), `spec/FRAME.md` (Instance API,
Event/Effects/Frame structs, text-metric formulas, frame.json canonicalization),
and `spec/SDP.md` (deterministic live-session automation, scene addressing,
input, inspection, and rendering). `slab dump FILE.slir` prints the canonical
slir-dump text.

## Frame ops

`Frame { width, height, ops, scene, strings, uncovered, glyphs, paths_rt }`
uses absolute logical coordinates and is already sampled at `t`; there are no
animation ops.

```rust
struct Frame {
  width: f64, height: f64, ops: Vec<FrameOp>, scene: Vec<SceneNode>,
  strings: Vec<String>, uncovered: Vec<u32>, glyphs: Vec<FrameGlyph>,
  paths_rt: Vec<RtPath>, diagnostics: Vec<FrameDiagnostic>,
}
struct FrameGlyph { font: i32, gid: u32, cluster: i32, x: f64, y: f64, size: f64 }
struct RtPath { verbs: Vec<u8>, coords: Vec<f64> }
// Retained scene also carries main/cross scroll geometry and the complete
// role/name/state/relation/value/live accessibility contract.
```

```
Rect      {node x y w h radius; bg/stroke; dash/shadows; opacity; smooth; grain}
Text      {node x y_baseline string-ref measured-w font size weight tracking color
           strike italic underline(+offset,thickness) rtl glyph-run uncovered-run}
Image     {node x y w h img fit radius opacity smooth}        // unified image index
PathDraw  {node dx dy path bg stroke stroke-w dash opacity}   // signed path ref
ClipPush {x y w h radius smooth} / ClipPop
GroupPush {opacity blur mask} / GroupPop
RotatePush {cx cy deg} / RotatePop
ScalePush  {cx cy sx sy} / ScalePop
TiltPush   {cx cy rx ry depth} / TiltPop
Backdrop {x y w h radius blur saturate brightness smooth mask}
```

Paint is `(kind, handle)`: `0 none | 1 solid rgba8 | 2 gradient index`.
Every paint op carries its node id. `PathDraw.path >= 0` indexes the document
PATH table; a negative value indexes `paths_rt[!path]`, whose entry is
`RtPath { verbs, coords }`. Runtime entry zero is therefore `-1`.
Scale/rotate/tilt/group/clip stacks are balanced.
`strike` and `underline` paint decorations over the kernel-provided
`measured-w` (underline offset/thickness come from the font); `italic` means
select or synthesize an oblique face. None of them changes text layout.
`rtl` marks right-to-left glyph order; each Text op addresses its positioned
glyphs (`Frame.glyphs`) and uncovered-glyph codepoint runs
(`Frame.uncovered`) by offset/len — fallback painters draw exactly the
uncovered slices inside kernel-charged advances. Field selection bands,
cross-field range bands, rich-field `code-bg` runs, and per-clause IME
underlines all arrive as ordinary kernel-emitted ops in paint order; drivers
never synthesize them. `diagnostics` carries current layout findings and
one-shot runtime notes such as `glyph-missing`.

`drag-ghost` introduces no driver API or semantic node. While active, flatten
appends a 0.72-opacity duplicate of the resolved source subtree at the cursor,
preserving the grab offset. Paint those ordinary ops in order; never make them
hittable or accessible.

Native `SceneNode` retains node id, parent index, kind, painted rect, radius,
rotation, flags, source line, main-axis `content_main/scroll_off`, cross-axis
`content_cross/scroll_cross`, orientation, and full accessibility semantics.
Those include role/name/description; checked/expanded/selected; exact-key
relations; value/range/text; modal/live/atomic; level/set position/size; and
disabled/focused. Frame ops and scene come from one flatten pass, so paint
order, sticky/anchored geometry, and hit order agree.

## Dynamic driver obligations

Resolve dynamic resources from the frame/instance, not by re-parsing authored
values:

```text
PathDraw(path < 0) -> frame.paths_rt[!path]
Image(img)         -> inst_img_info(img) + inst_img_bytes(img)
ScalePush          -> save; translate(cx,cy); scale(sx,sy); translate(-cx,-cy)
ScalePop           -> restore
```

Cache an image by `(unified_index, generation)`. Replace decoded/uploaded
resources when generation changes and release them when the index becomes
inactive. PNG (`format=0`) stays encoded; RGBA8 (`format=1`) is straight-alpha
sRGB. Web/GPU upload/decode, SVG embeds (converting RGBA8 to PNG), and PNG
composites. Do not retain frame-local runtime-path indices across frames.
If decode/upload is asynchronous, keep the layout box and schedule a repaint
when the resource becomes drawable.

On WASM, read frame-local paths from `FrameBuf.rt_paths_json()` as
`[[verbs],[coords]]` pairs. `ScalePush` is op tag `11` with four floats
`(cx,cy,sx,sy)`; `ScalePop` is tag `12` with no payload. `scene_json()` adds
main/cross scroll geometry plus every accessibility field; it resolves scene
string refs and serializes absent optional state/value fields as `null`.

Use `SceneNode.content_cross/scroll_cross` for two-axis chrome. Shipped web
and native clients maintain the semantic DOM/AccessKit tree themselves; a
custom driver must map the same retained accessibility fields and stable
codes. Never synthesize layout, sticky positions, popover collision, virtual
spacers, divider geometry, or drag-ghost semantics—the frame already contains
the paint/geometry result. `spec/FRAME.md` is the exact ABI, including fixed
FrameBuf tag/arities and accessibility code values.

## Units

| client | mapping |
|---|---|
| web / svg | 1u = 1 px (logical) |
| gpu | 1u = 1 pt × device scale |
| png | 1u = 1 px × `--scale` |
| tui | 1 cell = 8u × 16u; geometry snaps to the grid at paint |

Ops stay logical end to end; only the driver multiplies by device scale.

## Support chart & degradation

Normative support is machine-readable in `spec/support.toml` and generated
into SPEC/capability code. The table below is only operational guidance; a
driver must report the generated `cap-*` note once, never invent a fallback.

| feature | web | gpu | tui | svg | png |
|---|---|---|---|---|---|
| shadow | full | outset ≈ SDF, inset skipped | `cap-shadow` | spread ignored | full |
| blur / backdrop / masks | full | full except charted corner limits | `cap-blur`/`cap-backdrop`; masks per cell | backdrop/mask approximations | full |
| gradients / grain / smooth | full | ≤8 stops; charted corner limits | gradients per cell; `cap-grain`; arc corners | grain ≈ turbulence | full |
| compiled/runtime path | full | full | cell-center fill + slope-charred strokes | full | full |
| icon | full | full | scale ignored; design-box path coordinates | full | full |
| compiled/runtime image | full | full | shaded basename placeholder | full | full |
| rotation / scale / tilt | full | full | `cap-transform` | tilt is affine approximation | full |
| animation / transition | full | full | full | paint animation only; no transition | APNG animation; no transition |
| input / gestures / signals | full | full | pointer quantized to cell centers | `cap-input`/`cap-signal` | `cap-input`/`cap-signal` |
| main/cross scroll, reveal, sticky | full | full | full | `cap-scroll`; always-scrollbars at offset 0 | same |
| divider | full | full | full | authored/host-preset geometry, no dispatch loop | same |
| recursive/virtual lists, para runs, themes | full | full | full | full | full |
| anchored popover / a11y scene | full | full | full | full | full |
| holes | full | full | `cap-hole` | empty box | empty box |
| glyph fallback (uncovered runs) | system font stack | tofu boxes | raw codepoint passthrough (terminal fonts) | viewer-resolved text | blank at charged advance |

“A11y full” means complete retained scene semantics on every client. Shipped
web and native/GPU adapters materialize platform trees; TUI and static output
do not expose one. “Virtual lists full” means every client
paints the same kernel-selected bounded window; static clients still lack
scroll dispatch. A missing image name suppresses its Image op and warns
`img-missing`; TUI placeholders are intentional. Cross-media redesign belongs
in `when tui { … }`, not renderer heuristics. Consult `spec/support.toml`
before claiming support.

## TUI authoring

The cell grid quantizes at paint: every grapheme cluster is one cell except
East Asian Width W/F, emoji-presentation, and regional-indicator pairs, which
take two. Every line is one row regardless of `size`/`leading`. Layout still
uses proportional vector metrics. Keep pads, gaps, and fixed sizes in 8u/16u
multiples. Borders paint inside shared cells, so bordered boxes need
`pad=16,8`+. Radius ≥4u uses arc glyphs. Sub-cell boxes become hairline runs.
Omitted text color uses the terminal foreground without an SGR color. Authored
low-alpha color can vanish when the terminal background is unknown.

Match glyph advance to the cell grid on BOTH axes or text drifts off it: set
the full triplet `when tui { family="mono" size=13.333 leading=1.2 }` at the
root. Columns: JetBrains Mono's 600/1000em advance × 13.333 = 8.0u = exactly
one cell, so vector metrics match the grid. Rows: 13.333 × leading 1.2 = 16.0u
= exactly one row; the default leading 1.4 yields 18.67u line boxes, so any
stacked text column straddles cell boundaries and degrades (a progress track
astride a row boundary paints as a hairline `────`). Wider spacing belongs in
cell-multiple `gap`/`pad`, not leading.

Codepoints missing from the resolved embedded family pass through to the
terminal raw — the terminal paints CJK/emoji with its own font stack while the
grid charges East-Asian-Width cell advances — and still emit `glyph-missing`
diagnostics.

Runtime paths use the same cell approximation as compiled paths. Icons ignore
their scale transform and render in design-box coordinates; provide a
`when tui` text/glyph alternative when that geometry matters. Runtime and
compiled images use the same shaded basename placeholder. Nested/virtual
lists, two-axis scrolling, sticky, dividers, popover placement, gestures, and
a11y scene metadata remain kernel features and are otherwise available; input
coordinates are cell-center quantized.

## CLI

npm (`bunx @stencil-hq/slab`, Node ≥ 20 or Bun, zero Rust) and the native
`slab` binary (`cargo run -p slab-cli --` in-repo, or
`cargo install --git https://github.com/stencil-hq/slab slab-cli`) share:

```
slab check FILE                            # compile, print diagnostics (exit 1 on errors)
slab build FILE -o OUT.slir [--no-embed-assets]
slab dump  FILE.slir                       # canonical slir-dump text
slab render FILE -o OUT.{svg,png,apng,txt}
     [--client web|gpu|tui|svg|png] [--theme NAME]
     [--width N --height N] [--scale N]
     [--t MS] [--dur S --fps N] [--state a,b] [--env portrait,dark,coarse]
     [--set param=value]... [--plain]
slab gen wc    FILE -o DIR [--tag NAME] [--separate-ir]  # web-component module
slab gen react FILE -o DIR [--tag NAME] [--separate-ir]  # typed React wrapper
slab gen rust  FILE -o OUT.rs                             # typed Rust module
```

Native-only: `slab conformance [--update]`, `slab lsp` (stdio LSP:
diagnostics, completion, hover, preview), and `--font NAME=PATH`.

- `check` is compile-only and env-independent (env flags accepted, ignored).
- `render` defaults 800u × unbounded; client infers from the extension
  (`.txt/.ansi` → tui, `.png/.apng` → png, else svg); `--client tui` with no
  `-o` prints cells to stdout; `--plain` is the ANSI-free golden format.
  `--t` samples one instant; `--dur` (seconds) + `--fps` encode an APNG.
  `--set` coerces scalar params and accepts recursive list JSON:
  `--set 'roots=[{"label":"src","children":[{"label":"main.rs"}]}]'`.
  Invalid JSON, nested fields, keys, or types reject the whole assignment.
- `--state` enables document-global preview states only. It cannot target a
  single node; use the host node-state API for that.
- Authored image assets resolve relative to `.slab`; missing files warn.
  Runtime image registration is an Instance/Web/Rust host API, not a CLI
  `--set` payload.

Driver binaries beside `slab` (in-repo):

```
cargo run -p slab-tui -- FILE.slab [--theme NAME] [--set …] [--script 'TOKENS'] [--dump-after PATH]
cargo run -p slab-native -- --demo settings [--headless-frame OUT.png]
```

`slab-tui` headless `--dump-after` matches `slab render --client tui
--plain` byte for byte. Editor support: `slab lsp`, `tree-sitter-slab/`,
`editors/vscode` + `editors/zed`.

## Conformance

`conformance/` is the executable contract: cases + shared goldens for SLIR
dumps, frame.json (canonical one-line JSON; numbers through `fmt3` —
round-half-even to 3 decimals), TUI cell grids, scripted interaction
traces, and capability reports. Native (`slab conformance`) and WASM
(`bun run tools/conformance-wasm.ts`) must match byte for byte. Update
goldens only via `slab conformance --update` + manifest review. In-repo dev
loop: `bun install`, `just gen` (regenerate derived artifacts), `just ci`
(fmt/lint/tests/conformance/freshness), `just pack` (npm tarballs),
`just site` (playground).

## Repo map

- `crates/slab-kernel` — THE kernel (layout, motion, interaction, editing,
  dispatch); hand-maintained, deterministic (IEEE f64, no hash-order
  leaks). `crates/slab-kernel-wasm` — browser bridge.
- `crates/slab-compile`, `slab-syntax`, `slab-slir` — compiler front/back.
- `crates/slab-cli`, `slab-tui`, `slab-native`, `slab-lsp`, `slab-wasm` —
  drivers and tools.
- `clients/web` — `@stencil-hq/wslab` web runtime (SlabElement, DOM
  painter, WASM glue + the one `slab_kernel_bg.wasm`, built untracked by
  `just web-runtime`). `packages/slab` — `@stencil-hq/slab` npm CLI.
- `gen/web-runtime` — untracked `slab-runtime.js` bundle embedded in
  `gen wc` output; `just web-runtime` (or `just gen`) rebuilds it, and
  cargo cannot compile `slab-compile` without it.
- `spec/` — SPEC.md (normative), SLIR.md, FRAME.md, support.toml.
- `examples/`, `conformance/`, `site/` (playground), `assets/fonts/`
  (vendored Inter + JetBrains Mono, OFL).
