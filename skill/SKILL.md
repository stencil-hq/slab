---
name: slab
description: "Writing, editing, and rendering Slab documents (.slab) — the declarative design language for app screens, posters, terminal UIs, and interactive components. Use when authoring or modifying .slab files, rendering via the slab CLI (`bunx @stencil-hq/slab` or `slab-cli`: svg/png/apng/tui), embedding via generated web components (`slab gen wc`) or typed Rust modules (`slab gen rust`), declaring the typed host surface (params, list/each, holes, signals, themes), working on the conformance corpus, or debugging Slab diagnostics and layout."
---

# Slab

A declarative language for designed surfaces that renders faithfully to web,
GPU, TUI, SVG, and PNG from one source. One pipeline: the Rust compiler
(`slab-compile`) lowers `.slab` text to **SLIR** (protobuf + Snappy binary IR);
one Rust kernel (`slab-kernel`) owns layout, motion, hit testing, focus,
editing, and dispatch on every platform (natively linked, or via WASM in
browsers). Thin drivers paint frames, forward events, and expose the retained
semantic scene through platform accessibility adapters. `spec/SPEC.md` is
normative; `conformance/` is executable (native and WASM match goldens byte
for byte). Hosts never parse `.slab` and never do layout.

The whole layout model in one sentence:

> Every node is a box. Every box receives constraints (min/max width and
> height), returns a size, and its **parent** places it. Containers differ
> only in how they place children.

No margins, no selectors/cascade, no z-index, no absolute positioning, no
display property. Paint order is tree order; overlap requires an explicit
opt-in container (`stack`, `canvas`, `offset`). Mistakes become diagnostics,
not silent overlap.

## Quick start

```slab
tokens {
  color { bg #0e1116; ink #e6edf3; accent oklch(72% 0.16 250) }
  text  { title { size 18; weight 650 } }
}

def Chip(label, tone=color.accent) {
  row pad=4,10 gap=6 radius=999 stroke=tone align=center w=hug {
    rect w=6 h=6 radius=3 bg=tone
    text label size=12 color=tone nowrap
  }
}

col#card w=360 pad=24 gap=12 bg=color.bg radius=12 {
  text "Pale Green Things" style=text.title color=color.ink
  row gap=8 align=center {
    Chip label="FLAC"
    spacer
    text "4:12" size=12 color=color.ink
  }
}
```

Render (no Rust toolchain needed; the compiler ships as WASM in the npm CLI):

```sh
bunx @stencil-hq/slab render doc.slab -o out.png --width 800
bunx @stencil-hq/slab check doc.slab      # ALWAYS run after editing
```

Output kind infers from the extension (`.svg .png .apng .txt`); `--client tui`
with no `-o` prints cells to stdout. `check` validates the main document and
each `export` definition through its standalone path. Diagnostics keep the
source filename and identify the export. In this repo the native CLI is
`cargo run -p slab-cli --` (adds `fmt`, `conformance`, `lsp`, `--theme`,
`--font`).

Fresh releases: bun's minimum-release-age gate blocks packages younger than
24h (`… blocked by minimum-release-age`). Fix: add `[install]
minimumReleaseAge = 0` to a project `bunfig.toml`. `bunx` ignores a cwd
bunfig — use `bun add @stencil-hq/slab` then `./node_modules/.bin/slab`.

## Core vocabulary (memorize; details in references/language.md)

- **Containers**: `row col wrap grid stack canvas para group` — `stack`
  (layers) and `canvas` (`at=x,y`) are the ONLY overlap opt-ins.
- **Leaves/controls**: `text`, `span` (inside `para`), `rect`, `img`, `path`
  (canvas only), `icon`, `divider`, `spacer`, and `hole`.
- **Sizing** per axis: `w=240` (request) | `hug` | `fill` / `fill:2` | `40%`
  + clamps `min-w max-w min-h max-h`. Defaults: main = hug; cross = stretch
  for containers and `rect`, hug for other leaves; inside `stack`/`canvas`
  everything hugs.
- **Style**: `bg stroke stroke-w stroke-align stroke-sides stroke-dash radius
  smooth shadow blur backdrop backdrop-mask grain mask opacity color family
  size weight leading tracking style= align-text rotate scale tilt fit pad
  gap animate transition scrollbar scrollbar-w scrollbar-fg scrollbar-bg` —
  closed set, nothing else. `current` is icon-declaration paint, not a
  general color token.
- **Flags/modes**: `clip bleed scroll nowrap ellipsis inert focusable
  multiline drag-ghost`; use `drag-ghost` only with `drag=`, and use
  `scroll=cross|both`, `sticky`, and `each … virtual item-extent=N` only in
  their documented contexts.
- **Reserved attrs**: `key=`, signal binders (`act=`, pointer/drag lifecycle,
  field/submit, resize), `keys=`, a11y metadata/state/relation/value attrs,
  and overlay placement `attach= gravity= collide=`.
- **Conditionals**: `when hover|dragging|drop|tui|dark|w<600|prop|theme(name)
  { … }` patches its node with attrs/children. Signal binders are node-static
  and cannot be introduced inside `when`.
- **Components/data**: `def Name(params) { body }`, Capitalized calls,
  children splice at `slot`, and `export` defs become standalone documents
  and recursive `list(Def)` schemas. `each param.rows` consumes a root list;
  nested templates use `each child_prop`. Macro expansion has no arithmetic.
- **Host surface** (typed, compiler-checked): scalar params, recursive lists,
  holes, runtime image registration, signals, keyed scroll/divider/reveal
  APIs, and the retained scene. There is NO tree injection or selector API.

## Rules that prevent 90% of mistakes

1. `fill` is a SIZE keyword, never a color — backgrounds use `bg=`.
2. `align=` positions a node's CHILDREN; to position the node in its parent
   use `self=` (stack children: `self=bottom-end offset=4,-4`).
3. `%` needs a determinate parent axis; against `hug` it degrades to hug with
   `pct-unbounded`. Progress bars live inside `fill`/fixed tracks.
4. Node headers end at newlines. End each continued header line with `\`;
   indentation alone does not continue it.
5. Bare idents in value position are keywords or component props — token
   references are ALWAYS dotted (`color.accent`, never `accent`).
6. Numbers are unitless `u` (1u = 1px on web/svg). Durations are plain ms.
7. One shadow inline (`shadow=0,2,6,#0004`); layered shadows must be a list
   of presets/token refs (`shadow=shadow.crisp,shadow.lift`).
8. TUI paints one cell per grapheme, but layout uses vector font metrics.
   Use cell-multiple geometry and `pad=16,8`+ inside borders. Set
   `when tui { family="mono" size=13.333 }`: the 600/1000em mono advance
   becomes exactly 8u, so measured text matches the cell grid.
9. Quarter-turn `rotate` (±90/270) participates in layout; any other angle is
   ink-only. TUI skips rotated subtrees.
10. Dynamic rows come from `list(Def)` + `each`, including recursive child
   lists. Give items stable keys; path-address nested lists by index/field.
   Use kernel virtualization only for a uniform-height top-level `each`.
11. Diagnostics are the contract: `squeeze` = fixed size clamped, `clipped`
    = content truncated, and `glyph-missing` = static text is absent from its
    resolved embedded family. Fix the named source; never silence by guessing
    coordinates. `cap-*` names a declared client degradation.
12. Keep policy in the host: Slab owns layout, gesture mechanics and optional
    drag ghosts, focus, scrolling, scene export, and shipped web/native
    accessibility adapters. The host owns app state, popover dismissal, and
    focus traps.
13. Treat `spec/SPEC.md` as normative and `spec/FRAME.md` as the exact host
    ABI. Skill references are procedural guidance, not replacement specs.

## Feature selection cues

- Use a recursive `list(Def)` plus nested `each child_prop` for trees and
  grouped rows. Add `virtual item-extent=N` only to a direct root-list `each`
  under a main-axis scrolling `row`/`col`; use `revealItem` for navigation.
- Use `para { each param.runs }` for host-supplied rich text; make the run
  schema body exactly one `span`.
- Use `path d=param.route` (inside `canvas`) for runtime geometry. Declare
  reusable static `icon` assets at top level and tint them through `current`.
- Use `img src=param.name` plus host image registration for runtime pixels;
  do not encode changing image data into document params.
- Use `press/context/dblclick` for gesture starts, `pointer-move/pointer-up`
  for routed raw motion/releases, and `drag/drag-update/drag-end/drop` for a
  complete drag lifecycle. Consume every signal's typed `SignalMeta`; add
  `drag-ghost` when the kernel should paint the moving source duplicate.
  Use `act=` for ordinary keyboard-and-pointer activation.
- Use `scroll=cross|both` for two-axis overflow, `sticky` only on direct
  main-scroll children, and keyed `reveal` instead of host-computed offsets.
- Use `divider` between two panes; let it own pointer/keyboard resizing.
  Supply initial/restored extents through the keyed API, not bespoke dragging.
- Use `attach=param.anchor` on a `stack`/`canvas` child for popovers. Feed a
  signal's full `meta.key` back as the anchor; keep dismissal/focus policy in
  the host.
- Author complete a11y roles, names, state, relations, values, and live-region
  metadata. Shipped web/native adapters build the platform semantic tree;
  application hosts do not rebuild it from scene records.

## References — load on demand

- **references/language.md** — grammar and node semantics; dynamic
  paths/icons; para runs; scrolling/sticky; dividers; anchored overlays;
  accessibility attrs; layout, components, tokens, keys, and diagnostics.
  Read when authoring beyond basic screens or when a diagnostic is unclear.
- **references/styling.md** — gradients, icon `current` paint,
  layered/inset shadows, glass, blur, stroke geometry, transforms,
  interaction states/drag ghosts, and motion. Read when styling effects,
  icons, interactions, or animation.
- **references/hosts.md** — recursive/virtual lists, runtime images, pointer
  and drag signals with `SignalMeta`, generated web/Rust bindings, the exact
  clean-cutover Instance APIs, scroll/reveal/divider state, popovers, and
  framework accessibility adapters. Read when building an interactive or
  data-driven app.
- **references/rendering.md** — SLIR → kernel → Frame, runtime path/image and
  scale ops, scene semantics, per-client degradations, TUI rules, CLI, and
  conformance. Read when implementing a driver, targeting a client, or
  debugging cross-client differences.

When the repo is available, `spec/SPEC.md` is normative (`spec/SLIR.md` and
`spec/FRAME.md` for the machine interfaces) and `examples/*.slab` are quality
references — `examples/10-settings.slab` is the canonical interactive app
(params, signal buttons, kernel-edited field, hole);
`examples/12-tracklist.slab` shows `list`/`each`, themes, and scrollbars;
`examples/01-settings.slab` and `06-jcard.slab` are the visual-quality bar
for static documents. Live playground: https://stencil-hq.github.io/slab/
