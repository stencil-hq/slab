# Slab — a design language for agents

Version 0.1.0 · status: pre-alpha

Slab 1.1 is one pipeline: **one Rust compiler** (`slab-compile`) lowers a
`.slab` document to **SLIR**, a binary intermediate representation;
**one hand-maintained Rust kernel** (`slab-kernel`) owns layout, styling,
motion, hit testing, focus, editing, and event dispatch. Native clients link
the kernel directly; the web executes the same kernel through
`slab-kernel-wasm`. **Thin drivers** — a web custom element, a native wgpu
renderer, an interactive TUI, and static SVG/PNG exporters — feed it events
and paint its frames. Clients never see `.slab` text and never do layout.

This document is normative for the language. The companion documents are
normative for the machine interfaces an implementation is built against:
the SLIR byte layout (spec/SLIR.md) and the kernel Instance API and Frame
contract (spec/FRAME.md). `conformance/` is the executable form of this
contract: native execution (`slab conformance`) and WASM execution
(`tools/conformance-wasm.ts`) must reproduce the same goldens byte for byte —
frame dumps, TUI cell grids, interaction traces, and capability reports.

Slab is a declarative language for describing designed surfaces — app screens,
posters, terminal UIs, cards — that renders faithfully to web, GPU, TUI,
SVG, and PNG from one source. It keeps the composable box model of HTML/CSS
and deletes the exception zoo (cascade, margins, floats, z-index, out-of-flow
positioning).

Slab's primary author is an **agent** — a program or an LLM. The design
consequences run through the whole spec: a closed vocabulary small enough to
hold in context, layout that is cohesive without hand-tuned coordinates,
diagnostics that name the problem AND the remedy, and deterministic output a
program can verify. Humans write Slab comfortably; agents write it safely.

The whole layout model in one sentence:

> Every node is a box. Every box receives constraints (min/max width and
> height), returns a size, and its **parent** places it. Containers differ
> only in how they place children.

## 1. Design goals

1. **Expressive as HTML/CSS** for real UI, via composition — not via a large
   property set.
2. **Uniform layout.** One solver, one sizing vocabulary, no modes.
3. **Cheap clients.** The language is specified by its lowering to SLIR and
   the kernel's ~10-op frame list; a driver paints those ops plus glyphs —
   the kernel has already measured, laid out, and hit-tested everything.
4. **Manual when needed.** Pixel-perfect `canvas` islands compose inside flow
   layout and vice versa.
5. **Mistakes can't overlap.** Containment is a solver invariant (§6.1), not a
   style discipline. Overlap requires an explicit opt-in container.
6. **Agent-legible.** Intent over coordinates; actionable diagnostics over
   silent breakage; pure-function renders (`(document, states, t) → pixels`)
   over hidden state.

### Non-goals (the anti-spec)

No margins. No selectors, specificity, or `!important`. No floats or BFCs.
No z-index integers — paint order is tree order; layering is the `stack`
container. No `position: absolute/relative/fixed` — use `canvas`. No
`display` property — the container *type* is the layout. No Turing-complete
templating. Application behavior stays out of the language: the kernel owns
the interaction mechanics (§15) and reports **signals**; app logic lives in
the host, wired through params, holes, and signals (§13).

## 2. Syntax

KDL-flavored: a node is a name, optional `#id`, attributes, flags, then an
optional `{ }` block of children. Newlines or `;` separate siblings.
Comments: `//` line, `/* */` block. A `\` at end of line continues a long
node header onto the next line.

```slab
col #card w=360 pad=24 gap=12 bg=color.surface radius=12 {
  text "Pale Green Things" style=text.title
  row gap=8 align=center {
    rect w=6 h=6 radius=3 bg=color.accent
    text "FLAC" size=12 color=color.muted
    spacer
    text "4:12" size=12 color=color.muted
  }
}
```

### 2.1 Grammar (EBNF)

```ebnf
document   := stmt*
stmt       := tokens | theme | params | def | anim | topwhen | node
tokens     := "tokens" "{" group* "}"
theme      := "theme" IDENT "{" group* "}"
group      := IDENT ( "{" entry* "}" )
entry      := IDENT ( value | "{" entry* "}" )   // value: tuples allowed (shadow tokens)
params     := "params" "{" pdecl* "}"            // typed host inputs (§13.1)
pdecl      := IDENT ptype "=" pdefault            // the default is required
ptype      := "text" | "num" | "pct" | "color" | "bool"
            | "enum" "(" IDENT ("," IDENT)* ")"
            | "list" "(" UIDENT ")"
pdefault   := scalar | "[" [ litem ("," litem)* ] "]"
litem      := UIDENT "(" [ attr ("," attr)* ] ")" // attrs only; no children
anim       := "anim" IDENT "{" ( PCT "{" attr* "}" )* "}"
def        := "def" UIDENT "(" [ param ("," param)* ] ")" [ "export" ] block
param      := IDENT [ "=" scalar ]
topwhen    := "when" cond "{" tokens* "}"
each       := "each" REF [ "#" IDENT ] attr*
node       := NAME [ "#" IDENT ] ( arg | attr | flag )* [ block ]
attr       := IDENT "=" value
flag       := IDENT
arg        := STRING | NUMBER | IDENT          // positional, e.g. text "hi"
block      := "{" (node | each | when | transition | vline | export | newline)* "}"
scalar     := NUMBER | PERCENT | STRING | HASHCOLOR | REF | IDENT
            | IDENT ":" NUMBER                 // fill:2
            | IDENT "(" ... ")"                // color fn: oklch(...), rgb(...)
block      := "{" ( node | STRING | when )* "}"
when       := "when" cond block
cond       := IDENT | "!" IDENT | ("w"|"h") ("<"|"<="|">"|">=") NUMBER
            | "theme" "(" IDENT ")"
```

- `NAME`: lowercase = builtin node; Capitalized = component call (§9).
- `REF`: dotted path (`color.bg`, `text.title`) — a token reference, except
  the reserved head `param.`: `param.title` reads a declared param (§13.1).
  Bare idents in value position are keywords or component props, never tokens.
- `#word` after a node name is an id; `#hex` in value position is a color.
- Bare `STRING` children are text runs (meaningful inside `para`).
- `key=` is a **reserved attribute** on every node (builtin or component
  call): it names the node's identity segment for per-node state (§15.1) and
  never reaches styling. Values may be strings, numbers, or idents.
- `act=`, `field=`, `submit=`, `press=`, `context=`, `dblclick=`, `drag=`,
  `drop=`, `resize=`, `pointer-move=`, `pointer-up=`, `drag-update=`, and
  `drag-end=` are **reserved attributes** binding signals (§13.3). `act=`,
  `field=`, `press=`, and `drag=` imply `focusable`; `submit=` is legal only
  on a `field=` text node, `resize=` is emitted by a divider (§6.11), and the
  two drag companion bindings require `drag=` on the same node.
- `multiline` is a closed-vocabulary flag, legal only on a `field=` text
  node. Other placements produce `warn[attr]`.
- `drag-ghost` is a closed-vocabulary paint flag legal only with `drag=`; it
  duplicates the source subtree above normal content while the drag is active.
- `keys=` is an authorable reserved-meaning attribute: a comma-separated
  list of activation keys. It implies `focusable`; its runtime routing is
  specified in §15.4.
- A `cond` ident resolves, in order: renderer classes `web gpu tui svg png`
  (1.0 renames 0.5's `gui` to `gpu`, §18), environment idents
  `portrait landscape dark coarse`, component props (§9), **bool params**
  (§13.1; a non-bool param here is `err[param-type]`), else a state ident
  (§10, §15).
- `export` between a def's parameter list and its body marks the def for
  standalone compilation (§13.4).

## 3. Units

All lengths are **logical units `u`** — plain numbers. Each renderer declares
a density mapping:

| client | mapping |
|---|---|
| web / svg | 1u = 1 px (logical) |
| gpu | 1u = 1 pt × device scale |
| tui | 1 cell = 8u wide × 16u tall; geometry snaps to the cell grid at paint |
| png | 1u = 1 px × `--scale`; direct rasterization |

`%` is relative to the parent's content box on that axis. There is no `px`
unit in the language.

**Authoring for cells.** The solver measures with the real font tables on
every client (§11.1); cell media quantize at paint: each grapheme cluster
occupies one cell except East Asian Width `W`/`F`, emoji-presentation, and
regional-indicator-pair clusters, which occupy two. VS15 forces text
presentation to one cell and VS16 forces emoji presentation to two. Every
line occupies one row regardless of `size`/`leading`, runs re-quantize to
whole columns, and geometry snaps to the grid (`round(x / cell)`) — so sizes
that are not cell multiples (8u × 16u) drift by up to half a cell where they
accumulate. Layout measurement, line breaks, and ellipsis cuts still follow
the vector metrics; wide-cell quantization only preserves alignment within a
painted run. For designed terminal screens, keep pads, gaps, and fixed sizes
cell-multiples. Border strokes paint INSIDE the box as box-drawing cells —
ink shares cells with content, so boxes with borders need `pad=16,8` or more,
or text overwrites the border.

## 4. Nodes

### 4.1 Containers

| node | places children |
|---|---|
| `row`, `col` | along an axis, in document order, with `gap`. Sugar for `box axis=row\|col`; conditionals may patch `axis` (§10) |
| `wrap` | like `row`, but starts a new line when out of room |
| `grid` | column tracks that agree across rows (`cols=…`, §6.5) |
| `stack` | children on top of each other; later = above. **Overlap opt-in** |
| `canvas` | children where you say (`at=x,y`); SVG-ish leaves allowed. **Overlap opt-in** |
| `para` | inline text flow: strings and `span`s wrap as one paragraph |
| `group` | flow island: a plain `box` (default `axis=col`, hug sizing) usable with `at`/`anchor` inside `canvas`; accepts all box attributes |

### 4.2 Leaves

| node | meaning |
|---|---|
| `text "…"` | single-style text. Wraps by default |
| `span "…"` | styled run inside `para` |
| `rect` | empty styled box (a `box` with no children) |
| `img` | image; `src` first arg or attr accepts a string, Text param, or Text item prop; `fit=cover\|contain\|stretch` (§13.7) |
| `path "M…"` | vector path, SVG `d` syntax; `d=` also accepts a Text param or Text item prop. Canvas only. Interior paint is `bg` (uniform with boxes), outline `stroke`/`stroke-w`: `path "M0 0 L40 20 Z" bg=color.accent` |
| `spacer` | sugar for `rect w=fill` (in a row) / `h=fill` (in a col); any explicit attr overrides, e.g. `spacer h=fill:3` |
| `hole NAME` | host-filled viewport (§13.2). Either axis may hug the host-reported natural content size; takes sizing attrs and `scroll`/`clip` flags; no children |
| `divider` | childless split-pane handle with an authored main-axis extent. Must be a non-first, non-last direct child of `row`/`col`; controls its previous sibling (§6.11) |
| `icon NAME` | named vector icon; `NAME` may be a literal, Text param, or Text item prop. `size=` is square and defaults to inherited text size; `color=` supplies `current` paint (§4.3) |

### 4.3 Runtime vector paths and icons

Path data uses one canonical SVG path normalizer for both authored literals and
runtime Text values. It accepts absolute and relative `M L H V C S Q T A Z`
commands and lowers them to absolute `M L C Q Z` geometry. A dynamic `d=`
value is parsed once per distinct string in an instance and reused by every
node with that value. Malformed runtime data emits one `attr` diagnostic per
unique string and paints nothing. A path's intrinsic size is the bounding box
of its normalized points and control points.

`path` remains a `canvas`-only leaf. Data-driven graph rows therefore author a
canvas inside the row template; each item can provide its own route and paint:

```slab
def Segment(route="", tone=#64748B) export {
  canvas w=96 h=24 {
    path route bg=none stroke=tone stroke-w=3
  }
}
col { each param.segments }
```

A reusable icon is declared at top level as a positive square design box
(default `24`) containing one or more static paths:

```slab
icon check viewbox=24 {
  path "M4 12 L9 17 L20 6"
}
row color=#2563EB {
  icon check size=16
  icon param.status_icon size=20 color=#DC2626
}
```

Inside an icon declaration, `current` is legal for `bg` and `stroke`;
an omitted path `bg` defaults to `current`. At each usage, `current` resolves
to the icon node's `color`, which itself inherits like text color. Icon
declarations compile to detached static subtrees and do not otherwise enter
layout, paint, hit testing, or traversal. An icon usage lays out as a
`size × size` square, scales its design box uniformly, and paints its paths in
declaration order. A dynamic name is re-resolved on every solve. An unknown
name keeps the icon's layout box, paints nothing, and emits one `icon-missing`
diagnostic per unique name.

Declarations with an empty/non-path/dynamic body report `icon-body`; duplicate
names report `icon-dup`.

### 4.4 Document root


A document has one root node; multiple top-level nodes are implicitly wrapped
in a `col`. The root receives constraints from the renderer invocation
(e.g. `--width`, default 800u wide, unbounded height).

## 5. Sizing

Per axis, `w=` / `h=` take exactly one of:

| value | meaning |
|---|---|
| `240` | fixed request, in u. Clamped by constraints — a request, not a law |
| `hug` | size to content |
| `fill` / `fill:2` | share of parent's leftover space, weighted |
| `40%` | percent of parent content box (unbounded parent → treated as `hug`, warning `pct-unbounded`) |

Plus clamps: `min-w`, `max-w`, `min-h`, `max-h`.

**Defaults.** Main axis: `hug`. Cross axis: containers — and `rect`, which
is a `box` with no children — default to **stretch** (= `fill`); other
leaves default to `hug`. This gives block-like familiarity (a `col` child
fills its parent's width), full-width `rect h=1` dividers, and equal-height cards
in rows for free. Root default: stretch to the invocation width.
**Exception:** inside `stack` and `canvas` everything defaults to `hug` on
both axes — layers and manual islands want natural sizes. `fill`/`%` on a
stack or canvas child resolves against that container's bounds.

## 6. Layout algorithm (normative)

`measure(node, cons) -> size` where `cons = (min_w, max_w, min_h, max_h)`,
maxima may be ∞. A child is **never** given more space than actually remains.

### 6.1 The containment invariant

> In flow containers (`box/row/col`, `wrap`, `grid`, `para`): every child
> rect lies inside the parent's content box and sibling rects are pairwise
> disjoint. Overlap is expressible only via `stack`, `canvas`, or `offset`
> — and is quarantined inside that container by the boundary rule (§8).

This is a theorem about the solver (children are measured against remaining
space, which floors at 0), verified by property tests in the reference
implementation.

### 6.2 Row/col

Main axis = the container's axis; cross = the other.

1. `remaining = max_main − padding − Σgaps`.
2. **Pass 1 — non-fill children, in document order.** Each is measured with
   `max_main = remaining` (fixed → `min(request, remaining)`, hug → content,
   % → of content box); then `remaining −= size`. Space flows in reading
   order: earlier children win; `remaining` never goes below 0.
3. **Pass 2 — fill children** share `max(0, remaining)` by weight; each is
   measured with min = max = its share.
4. **Container main size:** hug → Σchildren + gaps + padding; otherwise the
   resolved size.
5. **Cross axis:** children first measure naturally against
   `max_cross`. Container cross = hug → max natural child cross (including
   baseline shifts). Stretch children whose natural cross ≠ final container
   cross are re-measured once with min = max = container cross.
   Baseline-aligned items keep their natural cross — stretch and baseline
   are contradictory, baseline wins (same resolution as CSS flexbox).
6. **Placement:** main axis per `pack` = `start|center|end|between`
   (default `start`); cross axis per container `align` =
   `start|center|end|baseline` (default `start`), overridable per child with
   `self=`. `baseline` aligns first text baselines (children without a
   baseline center).

### 6.3 The deflation ladder

When demand exceeds supply, losers are chosen in this order — the result is
always total, never overlapping:

1. `fill` children take only leftover (floor 0).
2. `hug` children re-measure tighter: text wraps → hard-breaks long words →
   `ellipsis` if flagged.
3. `fixed` children clamp to what remains → **diagnostic `squeeze`** naming
   the node and the deficit.
4. Content that still exceeds a fixed container is clipped, never painted
   outside → **diagnostic `clipped`** (see §8 for `bleed`).

Mistakes become design-time diagnostics, not silent overlap.

### 6.4 Wrap

Greedy line-filling: children measure as in a row; a child that doesn't fit
`remaining` starts a new line. Lines stack on the cross axis with `gap`.
`fill` children take the remainder of *their* line.

### 6.5 Grid

`cols=120,fill,hug` — a tuple of track sizes using the same vocabulary.
Children fill cells in row-major order; `span=N` occupies the next N columns.

1. Hug tracks: width = max natural width over that column's non-spanning
   cells. 2. Fixed tracks clamp; fills share leftover. 3. Row height = max
   cell height at final track widths. Spans clamp to their tracks' total and
   don't inflate track sizing (`squeeze` diagnostic if short). A cell with
   `self=start|center|end` is justify-self: measured at natural size and
   placed within its track instead of being force-filled to it.

### 6.6 Stack

Every child measures against the stack's constraints. Stack hug size = max
child extent. Children place by the 9-position vocabulary
(`top-start top top-end start center end bottom-start bottom bottom-end`,
default `top-start`) plus `offset=x,y`. **Positioning a child is `self=` ON
THE CHILD** (or `align=` on the STACK to set the default for all children) —
`align=` on the child itself would set the child's own children's alignment
and warns when the child is childless.

### 6.7 Canvas

Children take `at=x,y` (default `0,0`; negative allowed). By default `at`
addresses the child's **top-left**; `anchor=<9-position>` changes which
point of the child lands on the coordinate (`anchor=center` centers a
station dot on a line joint, `anchor=end` right-anchors a label —
`start`/`end` are middle-left/middle-right). A `group at=…` re-enters flow
layout inside a canvas. An un-anchored child is measured against
`canvas size − at` when the canvas is sized (anchored children measure
against the full canvas), else unbounded. Canvas hug size = bounding box of
children (paths use their `d` bounding box).

### 6.8 Para and text

Text wraps at word boundaries to `max_w`; a word longer than the line
hard-breaks. `nowrap` disables wrapping; a nowrap line that still does not
fit truncates with a `clipped` diagnostic (flag `ellipsis` to make the
truncation intentional and silent). `ellipsis` truncates the last line
with `…` when out of width or height. `para` flows its strings/`span`s as one
wrapped paragraph with per-run styling. Text measurement uses the SLIR
`FONT` metric tables (§11.1; normative formulas in spec/FRAME.md):
per-codepoint advances under the vendored Inter / JetBrains Mono faces —
the same tables every driver rasterizes from.

`each` may be a direct child of `para` to supply data-driven rich-text runs.
Its list schema def MUST contain exactly one top-level `span`; any other body,
including two spans, is `err[each-span]`. The span may read Text, Color, Num,
and family item props for its content, `color`, `size`, `weight`, `family`,
and `tracking`. Every item remains one independently styled run, while line
breaking, baselines, paragraph spacing, and gradient bounds are computed once
for the combined paragraph. This is the syntax-highlighting primitive; it does
not create a box per token.

### 6.9 Scroll

Bare `scroll` on a box activates its main axis; `scroll=cross` activates only
the cross axis, and `scroll=both` activates both. Children receive an unbounded
constraint on every active axis and the box clips to its viewport. These are the
only sanctioned "content bigger than parent" modes — still not overlap.
(Static renderers just clip.)

`sticky` is valid only on a direct child of a main-axis scroll container;
otherwise compilation fails with `sticky-ctx`. It pins to the main-start edge,
and the next sticky sibling pushes it away. Cross-axis and end-edge sticky
placement are deliberately unsupported in v1.

### 6.10 Anchored overlays

An `attach=` overlay is a direct child of `stack` or `canvas`; using
`attach`, `gravity`, or `collide` elsewhere is an `attach-ctx` error. The
attachment value is an exact full node key (§15.1), supplied by a quoted
string, a `Text` param, or a `Text` item prop. This permits a host to feed a
signal's `meta.key` back into a dynamic overlay:

```slab
stack #surface {
  rect #button w=96 h=32
  rect w=160 h=80 attach="#surface/#button"
       gravity=below-start collide=auto offset=0,6
}
```

Attached overlays do not contribute to stack/canvas hug size, scroll content
extents, or automatic overflow clipping. Explicit `clip` and scroll-container
clipping still apply through their ordinary ancestor boundaries.

The overlay first lays out normally to determine its size. After layout and
before frame lowering, its top-left is derived from the anchor's current
document-space scene rect. `gravity` has twelve values:
`below-start|below-center|below-end`, `above-start|above-center|above-end`,
`left-start|left-center|left-end`, and
`right-start|right-center|right-end`; the default is `below-start`.
`collide=auto` (the default) flips to the opposite side when the preferred
main direction overflows the root viewport, then slides the overlay along
its alignment axis into that viewport. `collide=none` preserves the
preferred placement. Authored `offset=x,y` is applied after collision
handling.

Placement is recomputed on every solve from painted geometry, so anchors
inside scrolling containers track their current position. If the key is not
present in the current scene, the overlay subtree is omitted from both the
frame and hit testing. Dismissal boundaries, outside-click policy, focus
trapping, and focus restoration remain host responsibilities, using the
exported scene rects and focus API.

### 6.11 Dividers

`divider` is a styleable box and focusable control which must be a non-first,
non-last direct child of `row` or `col`; every other placement is
`error[divider-ctx]`. Its handle footprint uses ordinary fixed, percentage,
hug, or fill sizing, and it controls the main-axis extent of its **previous**
sibling. For example:

```slab
row w=640 h=400 {
  col #sidebar w=fill min-w=160 max-w=360 { }
  divider #split w=6 resize=sidebar_resized dblclick=sidebar_reset
  col w=fill min-w=240 { }
}
```

A keyed persistent overlay replaces the previous pane's authored main-axis
size with a fixed extent. With no overlay, authored sizing is unchanged.
`inst_set_divider(i, key, extent)` restores an overlay and
`inst_get_divider(i, key)` returns it (`-1` for unknown or unset). Every write
is clamped by the previous pane's authored min/max. A pointer gesture also
snapshots the next pane's solved extent and minimum at pointer-down, so growth
cannot consume that minimum; layout rechecks the same invariant when applying
a host-restored value.

Primary pointer-down captures the divider. Pointer moves update the clamped
overlay and re-solve continuously; pointer-up emits its optional `resize=`
signal with `sig_text=fmt3(final_extent)` and ordinary pointer metadata.
When focused, the container-axis arrow keys adjust by 8u (1u with Shift) and
emit `resize=` for every keypress. Row dividers use the `col-resize` cursor and
Left/Right; column dividers use `row-resize` and Up/Down.

A double-click always clears the overlay, restoring authored sizing, and emits
the divider's optional `dblclick=` signal. Slab deliberately has no collapse
threshold or content-aware initial-allocation policy: hosts express collapse
with params/`when` and provide initial extents through the keyed API.

## 7. Styling

Small closed set; everything else is composition.

| attr | applies to | values |
|---|---|---|
| `bg` | any box | color **or gradient paint** (**not** `fill` — that's a size keyword. SVG delta, deliberate) |
| `stroke`, `stroke-w` | any box, `path` | color **or gradient paint**; number (default 1) |
| `stroke-align` | box | `inside\|center\|outside` (default center) |
| `stroke-sides` | box | subset of `t,r,b,l` — border only those sides (radius ignored; the tab-underline/list-divider attr) |
| `stroke-dash` | any box, `path` | dash pattern in u: `stroke-dash=16,14` (single value = even dashes). Dashed strokes render with butt caps |
| `radius` | box | number (`999` ≈ pill) |
| `smooth` | box, `img` | 0–1 corner smoothing (squircle; iOS ≈ 0.6). No-op unless `radius>0`; ink and clip only — geometry untouched |
| `shadow` | box | one shadow: preset `sm\|md\|lg` or `[inset,]x,y,blur,color` — or a **layered list** of presets/token refs: `shadow=shadow.crisp,shadow.soft`. `inset` paints above the fill |
| `blur` | any node | self blur in u: the node and its children render to a layer, blur, composite (frosted content, glows) |
| `backdrop` | box | **glass**: `backdrop=blur[,saturation[,brightness]]` blurs (then saturates and brightens, both default 1) what is already painted beneath the node's rounded rect, then the node paints over it |
| `backdrop-mask` | box with `backdrop` | any paint — scales the backdrop effect by the paint's **alpha** over the node box (**progressive blur**). Approximated as fixed blur bands: 6 on web/gpu/png, 3 on svg |
| `grain` | box | `amount[,size]` — deterministic monochrome speckle painted over the node's own fill area (works over `bg=none` — the overlay idiom). `amount` 0–1 alpha; `size` = speckle cell in u (default 1) |
| `mask` | any node | any paint — the node and its children render as a layer multiplied by the paint's **alpha** mapped over the node's border box; ink outside the box vanishes (the fade-out contract). Rides the opacity group |
| `animate` | any node | `animate=NAME,dur[,loop\|once\|alternate][,easing][,delay]` — run keyframes (§14) |
| `transition` | any node | `transition=dur[,easing][,delay]` — ease this node's `when`-state patches (§14) |
| `opacity` | any | 0–1. Composites as a **group**: children blend first, then fade as one layer |
| `color` | text | text color **or gradient paint** — gradient text maps the paint over the text node's content box (all lines share one box). **Inherits** |
| `family` | text | authored family name. **Inherits**; runtime registration may provide its actual face (§11.1) |
| `size`, `weight`, `leading`, `tracking` | text | font metrics. **Inherit** (leading = line-height multiplier, default 1.4; tracking = letter-spacing in u, after every glyph) |
| `style` | any node | token group applied as an attribute bundle, e.g. `style=text.title`, `style=card.raised`. Explicit attrs win |
| `align-text` | text | `start\|center\|end` within the text box |
| `rotate` | any node | rotation in degrees about the node's center. **Quarter turns (±90/270) are layout-aware**: the node measures against swapped constraints and occupies its rotated bounding box — a spine caption authors in place inside its strip. **Arbitrary angles are ink-only** (§6.1's third overlap opt-in): geometry untouched, paint tilts. TUI skips rotated subtrees (`cap-transform`) — redesign with `when tui` |
| `scale` | any node | `1.05` or `sx,sy` — ink-only zoom about the node's center. **Never layout**; hit-testing keeps the layout rect (no hover oscillation). TUI skips (`cap-transform`) |
| `tilt` | any node | `rx[,ry[,depth]]` — ink-only 3D perspective about the node's center: CSS `perspective(depth)·rotateX(rx)·rotateY(ry)` (degrees; `depth` in u, default 800; single number = rx). The subtree flattens into one plane; hit-testing keeps the layout rect. SVG degrades to an affine three-corner fit; TUI skips (`cap-transform`) |
| `fit` | img | `cover\|contain\|stretch` |
| `scroll` | box | bare flag = main axis; `scroll=cross` = cross axis; `scroll=both` = both (§6.9, §15.5) |
| `item-extent`, `overscan` | `each virtual` | required positive uniform main extent; optional nonnegative retained-item margin (default 4), §13.6 |
| `virtual` flag | root-param `each` in a main-scroll row/col | materialize a bounded uniform window (§13.6) |
| flags | box | `clip`, `bleed`, `sticky` (direct main-scroll child only), `nowrap`, `ellipsis`, `inert` (subtree ignored by hit testing and focus, §15.2), `focusable` (participates in tab order, §15.3) |
| `pad` | any box | `pad=16` (all) · `pad=v,h` (vertical, horizontal) · `pad=t,r,b,l` |
| `gap` | containers | `gap=8` or `gap=main,cross`: second value = grid row gap / wrap line gap (`gap=16,0` gives table gutters with tight rows) |
| `attach`, `gravity`, `collide` | direct child of `stack`/`canvas` | keyed anchored-overlay placement and viewport collision policy (§6.10) |

`family` preserves the authored string in SLIR. The compiler supplies Inter or
JetBrains Mono fallback metrics when no runtime face is registered: an
ASCII-case-insensitive family name containing `mono` selects the mono class,
all others select sans. A runtime registers a face by name and metrics; it
matches family ASCII-case-insensitively and nearest weight, so the same
authored document can use a page, native, or export-provided face without
changing source.

Colors are CSS color strings: `#0e1116`, `#fff8`, `rgb(…)`, `oklch(…)`.
Renderers with limited gamuts quantize (§11).

**Gradient paints** go anywhere `bg` takes a color:
`bg=linear(135, #241A4E 0%, #E8865E 100%)` (angle in degrees, 0 = up,
90 = right — CSS's angle convention),
`bg=radial(#FFE0B0 0%, #FFB37C00 100%)`, and
`bg=conic(180, #44CFFF 0%, #B48CFF 100%)` — a centered sweep, clockwise
from the `from` angle (REQUIRED first argument, 0 = up). Stops are
`color pct`; missing offsets distribute evenly; 8-digit hex alpha
participates in the ramp. Radial is deliberately non-configurable:
centered, radius covers the box — off-center glows are an oversized rect
in a `stack`/`canvas` with `offset`.
Stop ramps interpolate in sRGB (matching SVG); TUI samples every gradient
per cell. Gradients apply to box fills, box strokes, `path` fills and
strokes, and text `color` — web alone falls back to the first stop for
dashed or per-side gradient box strokes.

**Shadow tokens & layering.** A shadow token is a tuple entry —
`shadow { crisp 0,2,6,#00000040; lift inset,0,1,0,#FFFFFF40 }` — and
layered shadows are ALWAYS a list of presets/token refs
(`shadow=shadow.crisp,shadow.lift`): comma tuples are flat, so a multi-shadow
cannot be written inline. One inline `[inset,]x,y,blur,color` is the single-
shadow form.

**Glassmorphism is a recipe, not a primitive** — bundle it as a token style:

```slab
tokens { fx { glass { backdrop 22,1.35; bg #FFFFFF12; stroke #FFFFFF3D; stroke-w 1; radius 20 } } }
col style=fx.glass shadow=shadow.soft,shadow.edge { … }
```

**Inheritance whitelist:** `color family size weight leading` flow to
descendants. Nothing else inherits, ever.

## 8. The boundary rule

Uniform across every container, including `stack` and `canvas`:

- Child ink inside parent bounds → fine.
- Child exceeds bounds, no flag → **clip + diagnostic `clipped`**.
- Exceed caused **solely by the child's own `offset`** is the declared
  overlap opt-in (§6.1): it passes as ink — no clip, no diagnostic.
- `bleed` flag on the container → paint outside knowingly (ink only — never
  affects sibling layout).
- `clip` flag → clip silently (you asked).

Silent = contained. Loud = annotated. Overlap never leaks out of the
container that declared it. Ink effects (shadows, outer stroke halves) are
exempt: they are ink, not geometry.

## 9. Components

```slab
def Chip(label, tone=color.muted) {
  row pad=4,10 gap=6 radius=999 stroke=tone align=center {
    rect w=6 h=6 radius=3 bg=tone
    text label size=12 color=tone
  }
}

Chip label="FLAC"
Chip label="1977" tone=color.accent
```

- `def Name(params) { body }` — names MUST be Capitalized; builtins are
  lowercase. Expansion is lexical macro substitution; props are values.
- Call-site children splice at the `slot` node in the body (at most one).
  Slotted children keep the CALLER's style context (scope, font, color, key
  path) but take geometry defaults — cross-axis stretch — from the slot's
  parent, the container they actually land in (CSS `::slotted` semantics).
- A def body may have MULTIPLE top-level nodes: all splice as siblings at
  the call site. This is the grid-row idiom — `def Dep(…)` emitting six
  cells is one visual table row.
- Props are referenced by bare ident in value or arg position
  (`text label`). Truthiness: absent/`false`/`0`/`""` are false.
- `export` after the parameter list marks the def **exported**: it also
  compiles as a standalone embeddable document, its props promoted to typed
  params (§13.4).
- Recursion depth is capped (32); there are no general loops — `each` is the
  sole typed runtime repetition (§13.6). Slab is not a programming language.

## 10. Tokens and conditionals

```slab
tokens {
  color { bg #0e1116; ink #e6edf3; accent oklch(72% 0.16 250) }
  space { sm 8; md 16; lg 24 }
  text  { title { family "Inter"; size 18; weight 650 } }
}
```

Referenced by dotted path anywhere a value fits. `style=text.title` merges a
group as text attrs.

**`when` patches** — one mechanism for variants, states, media, and
responsiveness. Lexically attached to the node they modify; last patch wins;
no action at a distance.

```slab
row pad=8 radius=8 {
  text "Delete" color=color.danger
  when hover { bg=color.surface }     // state (kernel dispatch drives per-node states)
  when tui   { pad=1 radius=0 }       // renderer class: web | gpu | tui | svg | png
  when w<600 { axis=col }             // against the INCOMING max-w constraint
  when playing { stroke=color.accent }// component prop truthiness
}

when tui { tokens { space { md 1 } } }  // top-level: token overrides per renderer
```

**Themes** are named, compiler-checked token override sets. A declaration is
sugar for a deferred token override:

```slab
theme dusk {
  color { bg #16131f; ink #f2ecff }
}
```

`theme NAME { group* }` desugars to
`when theme(NAME) { tokens { group* } }`. Duplicate declarations of the same
name merge; later groups win under the same last-wins rule as other token
overrides. Theme declarations and nodes are top-level and order-independent,
so a declaration may follow every node that uses its tokens.

The host selects one declared theme by name. `inst_set_theme` rejects an
unknown name without mutation; the empty name is always valid and restores
the authored base. A `theme(NAME)` condition is active exactly when `NAME` is
selected, and may also be used directly in a node's `when` patch.

Theme token overrides use rule-10 site expansion, not a runtime token table.
Consequently a deferred token value resolves token references against the
authored base, and compound theme×client token overrides are not
representable. Use explicit per-node `when theme(NAME)` patches when
theme-specific token site expansion is insufficient.

Width conditions read the incoming **constraint**, never the resolved size —
responsive patches cannot create layout feedback loops. A `when` block may
contain attrs and/or extra children (appended in place).

**State scoping (§15).** A state ident matches in this order: component-prop
scope (folds at compile time) → a **bool param** of the same name (§13.1) →
the node's OWN states (keyed by the node's identity path, §15.1) → the
document-global set (CLI `--state`, `inst_set_state`). Kernel dispatch
drives the per-node interaction states, so two `Button`s never hover
together; hosts drive app states (`disabled`, `selected`, …) per node via
`inst_set_node_state`; the global set stays for previews and document-wide
conditions. Canonical per-node state names (documented, not enum-enforced —
unknown idents behave as inactive states): `hover pressed focus
focus-visible disabled selected composing dragging drop`.

## 11. SLIR and the Frame contract

Pipeline: parse → resolve (tokens, defs, prop folds) → **SLIR** → **kernel**
(when/anim/param eval, layout, flatten) → **Frame** (draw ops + scene) →
driver. The split is normative. Compile time folds everything foldable:
token refs, `style=` bundles, ordinary def expansion/prop truthiness, shadow
presets, path normalization, and font subsetting. Each-template props remain
symbolic as PropRef/Prop conditions. Everything env- or item-dependent ships
as **data** for the kernel: `when` patches, animations, scalar/list
params, list templates, holes, and signals. A driver does zero layout and
zero policy — it paints ops and forwards events.

### 11.1 SLIR

SLIR is the compiled form of a `.slab` document; clients receive SLIR
(embedded or fetched), never `.slab` text. It is a protobuf `slir.Doc`
message compressed as a raw Snappy block behind an 8-byte `SLIR` envelope
(major 2, minor 0). **spec/SLIR.md** and `spec/slir.proto` define the wire
contract; hosts use generated protobuf bindings to decode it, then construct
the public kernel `Doc`. The kernel does not parse SLIR bytes.

- `NODE`/`ATTR`: the resolved tree (DFS pre-order) carrying **authored
  attributes only** — the §5 sizing defaults and §7 inheritance are the
  kernel's job. `when`-patch extra children ride `detached` at the end of
  the parent's child chain and are spliced by the kernel while their
  condition holds.
- `WHEN`: conditions (`State`, `Env(portrait|landscape|dark|coarse)`,
  `Client(web|gpu|tui|svg|png)`, `Prop`, `Theme`, `W/HCmp` — width/height
  compare against the INCOMING constraint, as in 0.5) plus per-node patch
  runs; last patch wins. Top-level token overrides compile to ordinary
  per-site patches placed before the node's explicit ones (site expansion,
  rule 10, §18).
- `THEM`: declared theme names, deduplicated in declaration order.
- `PARM`/`LIST`/`HOLE`/`SIGN`: the typed host surface (§13), including list
  schemas, normalized defaults, detached Each templates, and every ordinary
  attribute site a scalar param feeds.
- `FONT`: subset cmap + per-glyph advances and fallback metrics per authored
  family/weight. Font bytes are a runtime concern: registered faces override
  matching fallback tables; otherwise the client uses its bundled or platform
  fallback.
- `IMGS`: image metadata plus parallel `img_data` payloads. Empty payloads
  represent omitted or unavailable assets.
- `ANIM`: keyframe stops, binds, and transitions (§14) as data.

### 11.2 The kernel Instance API

The kernel has one hand-maintained Rust implementation (`slab-kernel`).
Native clients link it directly; web clients call it through the
`slab-kernel-wasm` representation bridge. Both expose the same semantics.
The full as-built contract is **spec/FRAME.md**; the shape:

```
host decode SLIR -> Doc                         // generated protobuf binding; host-owned
inst_shell() -> Instance                        // assign decoded Doc to instance.doc
inst_init(i)                                    // initialize persistent parameter state
inst_font_register(i, family, metrics...) -> i32 // append runtime face metrics; later equal match wins
inst_set_env(i, vw, vh, client, dark, coarse)  // client: 0 web | 1 gpu | 2 tui | 3 svg | 4 png
inst_set_state(i, name, on)                    // document-global states (§10)
inst_set_theme(i, name) -> bool                 // unknown rejected; empty restores authored base
inst_theme(i) -> str                            // current name; empty means authored base
inst_set_node_state(i, key, name, on)          // host app states on one node
inst_set_scroll(i, key, axis, offset) -> bool   // axis 0 main | 1 cross; retained-geometry clamp
inst_get_scroll(i, key, axis) -> float         // 0 for an unknown key or axis
inst_reveal(i, key, margin) -> bool             // minimally reveal through every scroll ancestor
inst_set_param(i, param, value) -> bool        // typed; false on any mismatch
inst_list_len(i, param, path) -> i32           // path "" is root; -1 when unresolved
inst_set_list_len(i, param, path, n) -> bool   // recursive defaults / recursive truncation
inst_set_list_field(i, param, path, index, field, v) -> bool // scalar field in selected list
inst_set_list_key(i, param, path, index, key) -> bool // innermost stable identity
inst_reveal_item(i, each_key, index, align) -> bool // virtual: start|center|end|nearest (0..3)
inst_each_window(i, each_key) -> (i32, i32)     // virtual materialized half-open range
inst_set_hole_size(i, hole, w, h)              // persistent natural size; dirties only on change
inst_set_divider(i, key, extent) -> bool        // persistent split extent; clamps to adjacent panes
inst_get_divider(i, key) -> float               // -1 for unknown or authored fallback
inst_lift_animations(i) -> [Lift]              // driver takes over CSS-translatable bindings (§14.1)
inst_frame(i, t_ms) -> Frame                   // solves iff dirty or animating
inst_holes(i) -> [HoleRect]                    // hole rects of the last solve
inst_hit(i, x, y) -> [node]                    // path root -> target (§15.2)
inst_dispatch(i, ev) -> Effects                // events in, Effects out (§15)
text_glyphs(i, frame, op) -> [GlyphPos]        // per-codepoint gid/x walk (GPU)
```

`inst_frame` re-solves only when inputs changed (env/param/state/scroll/
edit/focus) or the clock moved while animations or transitions are live —
"interpolate inputs, re-solve" (§14): an animating instance stays live, an
idle one solves once. Layout diagnostics (§12) accumulate per solve on the
instance.

### 11.3 Frame: ops and scene

`Frame { width, height, ops, scene, strings, paths_rt }` — the solved root
size, draw ops in paint order, the retained scene for hit testing, the
per-frame text pool, and normalized runtime paths referenced by this frame.
Ops carry absolute document coordinates.

| op | payload (abridged; exact fields in spec/FRAME.md) |
|---|---|
| `Rect` | x y w h radius smooth; bg/stroke paint; stroke w/align/sides/dash; shadow run; opacity; grain amount/size |
| `Text` | x, y_baseline, string ref, measured width, font table index, size/weight/tracking, color (solid rgba8 or gradient handle + gradient box), opacity — one op per line |
| `Image` | x y w h, image handle, fit, radius, opacity, smooth |
| `PathDraw` | dx dy, path handle (`>=0` document PATH, `<0` complemented `paths_rt` index), bg/stroke paint, stroke-w, dash, opacity |
| `ClipPush {x y w h radius smooth}` / `ClipPop` | rounded/squircle-rect clip chain |
| `GroupPush {opacity blur mask mask_box}` / `GroupPop` | layer compositing: fade/blur/alpha-mask as one |
| `RotatePush {cx cy deg}` / `RotatePop` | paint-time rotation |
| `ScalePush {cx cy sx sy}` / `ScalePop` | paint-time scale about a center (icon design boxes, `scale=`) |
| `TiltPush {cx cy rx ry depth}` / `TiltPop` | paint-time 3D perspective; the subtree flattens into one plane |
| `Backdrop {x y w h radius smooth blur saturate brightness mask}` | reads the canvas: blur/saturate/brighten beneath (optionally alpha-masked — progressive blur), then continue |

- **Paint** is a `(kind, handle)` pair: none · solid rgba8 · gradient
  (GRAD table index). Gradient stop ramps interpolate in sRGB.
- Every paint op carries its **node id** — the retained-DOM diffing key
  for the web driver; GPU/TUI drivers ignore it.
- 0.5's `ANIM_PUSH/POP` ops are **gone**: motion interpolates attribute
  inputs before the solve (§14), so ops arrive already sampled at `t`.
- `SceneNode` (one per node, same DFS): rect, radius, rotation, effective
  flags (clip, inert, …), `content_main` (scroll clamping), source line.
  Ops and scene are emitted by one flatten pass, so paint order and hit
  order agree by construction.

### 11.4 frame.json and conformance

The kernel canonicalizes a frame as one line of JSON —
`{"width","height","ops","scene","strings","diags"}`, fields in struct
order, numbers through the shared `fmt3` formatter (round-half-even to 3
decimals, trailing zeros trimmed, `-0 → 0`); exact shapes in spec/FRAME.md.
This is the conformance currency: native and WASM execution must emit
**byte-identical** output for the same SLIR + env. `slab conformance` and
`bun run tools/conformance-wasm.ts` byte-compare frame dumps, TUI cell grids,
scripted interaction traces, and capability reports against the shared
goldens in `conformance/expected/`. Native compiler conformance separately
checks SLIR dumps against their checked-in goldens.

### 11.5 Degradation is chart-spec'd, not ad hoc

What each client does with each feature is normative in the support chart
(§Platform support), generated from machine-readable `spec/support.toml`
into driver lookup tables — a driver reports the chart's `cap-*` code once
per document (§12) instead of improvising. The classic cell-media rules
still hold: box `stroke` degrades to box-drawing borders, a box thinner
than a cell to a `─`/`│` hairline run (`╌`/`╎` when dashed — `rect h=1
bg=…` stays the medium-independent rule primitive), gradients sample per
cell, colors more than half transparent drop entirely, `BACKDROP`
paints flat. Cross-media text divergence is the design: intentional
per-medium redesign belongs in `when tui { … }` patches, not in renderer
heuristics.

## 12. Diagnostics

`file:line: level[code]: message`, machine-readable; codes that carry a
remedy print it as indented follow-up lines. Compile-time codes surface
from `slab check`/`slab build` (stderr; exit 1 on errors); layout-time
codes accumulate per solve in the kernel instance and ride in frame.json
`diags`; capability notes (`cap-*`) are reported once per document by the
driver from the generated support tables (§11.5).

Compile time (`slab-syntax` + `slab-compile`):

| code | level | meaning |
|---|---|---|
| `parse` | error | syntax error |
| `ref` | error | unknown token/param/prop/component reference; token cycle; malformed value |
| `param-type` | error | a param default does not fit its declared type, or a non-bool param used as a `when` condition (§13.1) |
| `dup-hole` | error | one hole name declared twice |
| `attr` | warning | unknown/ignored attribute for this node (also emitted at layout time) |
| `dup-param` | warning | duplicate param declaration; the first wins |
| `dup-signal` | warning | one signal name is bound to both a non-text trigger and a text-payload trigger; same-shape fan-in is legal (Change, Submit, and Resize all carry text; §13.3) |
| `list-def` | error | `list(Def)` does not name an exported def schema (§13.4) |
| `each-target` | error | `each` does not reference a root List param or an enclosing List-typed item prop |
| `each-nest` | error | an `each` template contains a `hole` |
| `each-span` | error | an `each` directly under `para` references a def whose body is not exactly one `span` (§6.8) |
| `virtual-ctx` | error | `virtual` is nested or not a direct child of a main-axis scroll row/col |
| `virtual-extent` | error | `virtual` has no positive numeric `item-extent` |
| `shadow` | warning | a def param shadows an attribute/flag name or the `fill`/`hug` keywords (rule 8, §18) |
| `dup-token` | warning | token path redefined; last definition wins |
| `dup-def` | warning | component redefined; last definition wins |
| `dup-id` | warning | one `#id` resolves more than once (second site reported) |
| `dup-key` | warning | sibling key collision; both nodes kept (§15.1) |
| `attach-ctx` | error | `attach`, `gravity`, or `collide` used outside a direct child of `stack`/`canvas` (§6.10) |
| `icon-body` | error | an icon has no paths, a non-path child, a dynamic value, or a nonpositive viewbox (§4.3) |
| `icon-dup` | error | a top-level icon name is declared more than once |

Layout time (kernel, per solve):

| code | level | meaning |
|---|---|---|
| `squeeze` | warning | fixed size clamped; names node and deficit (§6.3) |
| `clipped` | warning | content exceeded bounds and was clipped (§8) |
| `pct-unbounded` | warning | `%` against an indeterminate axis; degrades to hug (§13.5) |
| `fill-unbounded` | warning | `fill` against an unbounded axis (e.g. inside `wrap`) |
| `img-missing` | warning | a resolved image name matches neither an active runtime registration nor a compiled image; one warning per unique name |
| `icon-missing` | warning | a resolved icon name has no declaration; one warning per unique name |

Capability notes (`cap-*`, note level, once per document). The per-client
trip conditions are the support chart's (§Platform support); the 1.0 set:

| codes | reported by |
|---|---|
| `cap-shadow` `cap-blur` `cap-backdrop` `cap-path` `cap-transform` `cap-grain` `cap-hole` | tui: no cell representation |
| `cap-transition` `cap-input` `cap-scroll` `cap-signal` `cap-edit` | static exporters (svg/png): no runtime |
| `cap-ime` | tui and static exporters |
| `cap-gradient-stops` `cap-clip-rotated` | gpu driver approximations (see chart notes) |
| `cap-image` | image payload missing or undecodable → placeholder |
| `cap-font` | selected FONT face unusable by the driver → its glyphs are skipped |

`slab check` exits non-zero on errors; warnings are the "you were about to
overlap in CSS" signal. 0.5's `key-missing` is retired: it audited
host-injected siblings, and runtime injection no longer exists (§13.5).

## 13. Host embedding: params, lists, holes, runtime images, signals, exported defs

0.5's template/selector/injection surface (`tpl.frame()`,
`f["Gauge#cpu"].set(…)`, `.children(…)`) is **removed** (rule 9, §18). A
document declares its host surface in the language, typed and compiler-checked:
**params and lists** (inputs), **holes** (host-filled
viewports), **signals** (outputs), and **exported defs** (standalone
components and list schemas). Hosts never parse `.slab`, never mutate trees,
and cannot inject an ill-typed value.
The AST-splicing machinery survives only *inside* the compiler, as the
mechanism of `when` patches (attr overlays + detached children, §11.1); no
host API reaches it.

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
text#field param.draft field=draft w=300 h=32     // kernel-edited (§15.6)
col { each param.rows }
col#panel clip { hole rows w=fill h=336 scroll }  // host-filled (§13.2)
```

### 13.1 Params

- Seven types: `text num pct color bool enum(a, b, …) list(Def)`. Every
  default is **required** and must fit the declared type (`err[param-type]`).
  List types and defaults are specified in §13.6. Duplicate declarations warn
  `dup-param`; the first wins.
- Reference a **scalar** `param.NAME` at any whole-value site — attribute
  values and text content/args. `num`/`pct` param refs are also legal in
  tuple member positions (`offset=param.x,param.y`, `at=`, `pad=`, `gap=`,
  `stroke-dash=`, `backdrop=`); any other param type in a tuple member, or
  a param used where its type cannot fit, is `err[ref]` at the use site.
  Dynamic tuples cannot size grid tracks. A List
  param is referenced only by `each param.NAME` (§13.6).
- **Bool params are `when` conditions** (`when compact { gap=8 }`); a
  non-bool param in a condition is `err[param-type]`. Resolution order:
  §10.
- Setting a param marks the instance dirty on change; the next frame re-solves.
  `inst_set_param` handles scalar types and is total: `false` (never a throw)
  on an unknown name, List param, type mismatch, or unknown enum member.

Per client: **web** — every param is an observed attribute (name
kebab-cased) and a typed property on the generated element (§13.4); bool
attributes use presence semantics (absent or `"false"` = false), other
attributes coerce from the string, property writes set the param directly.
**native** — `slab gen rust` emits `PARAM_*` ids and one typed setter per
param on the generated `Doc` wrapper (`set_title(&str)`,
`set_compact(bool)`, …). **CLI** — `slab render --set param=value` coerces
to the declared type and fails on unknown names or bad enum members.
Static renders otherwise show the declared defaults.

### 13.2 Holes

`hole NAME` reserves a rectangle that the HOST fills: the web driver slots
real DOM into it, the native driver mounts a child kernel instance, static
exporters leave the box empty, the TUI reports `cap-hole`.

- Either axis may use the ordinary fixed, `fill`, `%`, param, or `hug`
  sizing rules. On a `hug` axis, the kernel uses that hole's persistent
  host-reported natural content dimension; it is `0` before the first report.
  The resulting size then passes through the ordinary `min-w`/`max-w` or
  `min-h`/`max-h` clamps. A reported dimension has no effect on a non-hug
  axis. Holes take the container cross-axis fill default (§5).
- `inst_set_hole_size(i, hole, w, h)` stores both reported dimensions. It
  ignores an invalid hole index and marks the instance dirty only when either
  stored float changes; re-reporting equal values is a no-op.
- Placement is a per-frame kernel output: `inst_holes` →
  `HoleRect { hole, x, y, w, h, clip }`. The sanctioned host loop is: solve,
  read this viewport, measure the host content's natural size, report it with
  `inst_set_hole_size`, then re-solve once. The report is a persistent layout
  input rather than a measurement of Slab content, and an equal subsequent
  report does not dirty the instance, so a stable natural size converges
  without a demand-frame loop.
- Web: each hole mounts a named `<slot name="NAME">` positioned over the
  hole rect; `scroll` holes scroll natively in the host DOM, and slotted
  content reports its natural size.
- Native: the `HoleContent` trait (natural size / resize / frame / dispatch);
  the shipped `InstanceHole` mounts a child `Instance` composited into the
  hole rect under a forced clip and reports the child's preferred size.
- TUI and static SVG/PNG rendering have no mounted host content and make no
  size report, so a hug axis starts at `0` before its ordinary clamps.

### 13.3 Signals

Signals are the document's only outputs — named, declared on the node that
fires them. Bindings are node-static: placing any signal binding inside a
deferred `when` patch warns `attr` and ignores it.

- `act=NAME` — Activate (trigger 0), with empty text.
- `field=NAME` — Change (1); text nodes only, with the full committed text
  after every mutation. Caret-only moves repaint without a signal (§15.6).
- `submit=NAME` — Submit (2); legal only on a `field=` text node, with the
  full committed text according to the Enter matrix (§15.6).
- `press=NAME` — Press (3), on primary pointer-down before capture.
- `context=NAME` — Context (4), on secondary pointer-down.
- `dblclick=NAME` — Dblclick (5), on a host-counted double pointer-down.
- `drag=NAME` — DragStart (6), once an armed pointer moves beyond the drag
  threshold.
- `drop=NAME` — Drop (7), on the deepest eligible target at drag release.
- `resize=NAME` — Resize (8), with the divider's final extent as text (§6.11).
- `pointer-move=NAME` — PointerMove (9), on every pointer move routed to the
  deepest enabled binding in the captured path, or current hit path without capture.
- `pointer-up=NAME` — PointerUp (10), on every pointer-button release routed
  through that same captured/current path.
- `drag-update=NAME` — DragUpdate (11), on every active-drag move, including
  the threshold-crossing move immediately after DragStart.
- `drag-end=NAME` — DragEnd (12), exactly once when an active drag ends.

`act=`, `field=`, `press=`, and `drag=` imply `focusable`; the other bindings
do not. One name may fan in from sites with the same payload shape. Change,
Submit, and Resize are text-bearing; the other triggers are non-text. Reusing
one name across those shapes warns `dup-signal`.

Every emitted signal carries an innermost list item key (or `""` outside an
`each`) and a `SigMeta` in the parallel
`Effects.sig_name/sig_text/sig_item/sig_meta` arrays. `SigMeta` is
`{x,y,dx,dy,drag_dx,drag_dy,mods,button,clicks,key,src_key,src_item,cancelled,dropped}`:
`key` is the emitter's full node-key path; keyboard-originated `x/y` are
`-1/-1`; `dx/dy` are the originating event deltas; `drag_dx/drag_dy` are the
current pointer displacement from the armed pointer-down origin when a drag
is active (otherwise zero); and `src_key/src_item` are populated only for Drop.
`cancelled` distinguishes abnormal DragEnd termination. `dropped` is true on
a delivered Drop and on its corresponding DragEnd, and false otherwise.
Web events are bubbling and composed, with an
always-present `{item, meta}` detail plus `text` for Change, Submit, and
Resize. `slab gen rust` emits one shared `SignalMeta` type and includes
`meta: SignalMeta` on every `Signal` variant; `slab gen wc` emits the same
shared metadata shape for every named `CustomEvent` detail.

### 13.4 Exported defs

`def Row(label, tone) export { … }` compiles the def as its own standalone
document for `slab gen wc`, and also makes it eligible as a `list(Row)`
element schema (§13.6). Its props are promoted to typed fields by the existing
use-site inference: text content / `act=` / `field=` / `src=` → text;
`bg`/`stroke`/`color` → color; numeric slots including tuple members → num;
`when` truthiness → bool; conflicting or missing votes → text. The same
inferred schema governs generated component properties and every list default
or host update. A list schema must be exported (`err[list-def]`). A field
declared with `children=list(Tree)` is itself List-typed; the referenced def
must also be exported. Schemas may be self-recursive or mutually recursive.
The compiler allocates canonical schema rows before resolving their fields, so
cycles are represented by row references rather than by expanding a type
forever.

Host-owned exported-def instances remain appropriate for arbitrary host
content and variable-height virtualization. Runtime `each` also has a
kernel-owned uniform-extent virtual mode (§13.6); neither mechanism is a
compatibility shim for the other.

### 13.5 % needs a determinate axis

A latent rule 0.2 made normative, unchanged in 1.0: `%` resolves against
the parent's content box **only when that axis is determinate** (fixed, %,
or fill-given). Against a hug axis the child would drive the very size it
resolves against — it degrades to hug with a `pct-unbounded` warning.
Progress bars therefore live inside `fill`/fixed tracks.

### 13.6 Lists and `each`

`list(Def)` is a persistent, typed parameter whose element schema is the
exported def's inferred props (§13.4). Its required default is `[]` or a list
of calls to that exact def. List-typed fields accept nested literals, which are
normalized recursively into the same typed default runs as the root:

```slab
def Tree(label="", children=list(Tree)) export {
  col {
    text label
    each children
  }
}
params {
  roots list(Tree) = [
    Tree(label="src", children=[
      Tree(label="ui", children=[Tree(label="tree.rs")])
    ])
  ]
}
col { each param.roots key=tree }
```

Entries contain field assignments only: no positional args, blocks, or
children. Unknown fields, wrong call names, and type mismatches are errors.
Omitted scalar fields take the def-prop default, or the type-zero default when
none was authored: `""`, `0`, `false`, the first enum member, or
`#00000000`. An omitted List field is empty. Defaults and host extension create
every descendant list state recursively; truncation removes descendant values,
keys, synthetic ids, and keyed state recursively.

`each param.rows` instances a root list in order. Inside its schema template,
`each children` instances the enclosing item's List-typed `children` field.
Symbolic List props may be forwarded through ordinary defs before reaching the
nested `each`. Templates stay detached and symbolic: recursive data controls
runtime depth, and the compiler never unrolls a recursive schema. `hole`
remains forbidden anywhere inside an `each` template (`err[each-nest]`).
Direct `para` children use the exactly-one-span rule in §6.8.

Each item has a stable, nonempty, unique string identity. Before a host
assigns one, its key is its decimal index. A synthetic descendant's public key
is `<each-key>~<item-key>/<template-relative-key>`; every nested level appends
another `~item/relative` segment. `inst_set_node_state`, scroll, focus, edits,
animation state, hit testing, and signals all use that full synthetic identity,
while document attributes come from the shared template. `sig_item` is always
the **innermost** item key; `SigMeta.key` carries the full path needed to
disambiguate nested items. Reordering values under stable keys preserves
per-item state.

The low-level typed API addresses a concrete list with `param` plus `path`.
`""` selects the root; every nonempty path alternates `<index>.<field>` pairs,
for example `"3.segments"` or `"3.segments.0.points"`:

- `inst_list_len(i, param, path)` returns its length, or `-1` when any pair is
  malformed, out of range, scalar, or unknown.
- `inst_set_list_len(i, param, path, n)` rejects invalid paths and negative
  lengths. Extension recursively seeds schema defaults; truncation recursively
  removes dropped data and state.
- `inst_set_list_field(i, param, path, index, field, value)` accepts scalar
  fields only and rejects an out-of-range item, wrong `ParamValue.kind`, or
  unknown enum member.
- `inst_set_list_key(i, param, path, index, key)` rejects an empty key. It
  deliberately permits a transient duplicate so a host can reorder with
  sequential writes; complete all key writes before solving.

Every valid equal write is a successful no-op and does not dirty. Invalid
requests return `false` without mutation.

**Uniform virtualization.** `virtual` is valid only on a non-nested root-param
`each` directly inside a row/col whose main axis scrolls; otherwise
`err[virtual-ctx]`. It requires a positive constant
`item-extent=N` (`err[virtual-extent]`) and accepts `overscan=N` (default 4).
Variable-height items deliberately stay unvirtualized in v1.

For list length `len`, retained main-axis offset `off`, viewport `vp`, extent
`e`, and overscan `o`, the materialized half-open window is
`[floor(off/e)-o, ceil((off+vp)/e)+o]`, clamped to `[0,len)`. Before viewport
geometry exists, the conservative first window is `[0,min(len,2o))`; the fresh
scene geometry dirties one settling frame. Layout places retained items in
their logical slots and accounts for omitted leading/trailing slots directly,
so scroll content extent is exactly `len*e` and frame/scene size is
`O(window)`, not `O(len)`. De-windowed synthetic ids and keyed state remain
registered; only truncation prunes them. Focus and tab traversal see only
materialized scene nodes.

`inst_each_window(i, each_key)` returns the last materialized half-open range,
or `(-1,-1)` for an unknown/non-virtual each. `inst_reveal_item` accepts
alignment `0 start | 1 center | 2 end | 3 nearest`, clamps against
`len*extent-vp`, updates the owning scroll offset, and materializes the target
on the next solve.

Bulk public inputs prevalidate the complete recursive replacement, including
final nonempty key uniqueness, before mutating it. Web components accept
nested arrays of plain objects, expose recursive TypeScript interfaces, and
write every descendant through the path API. `slab gen rust` emits recursive
`Vec<SubItem>` fields plus a `set_<param>` that validates the whole tree before
walking the same paths; an omitted Rust key resets to its positional key.
CLI, WASM options, and `slab-tui --set` accept equivalent nested JSON. Invalid
JSON, entries, keys, fields, types, colors, or enum members reject the whole
assignment. All five clients render nested lists, para runs, and retained
virtual windows from the shared kernel frame.

### 13.7 Runtime images

`img src=…` resolves an exact image name from a string literal, Text parameter,
or Text item property. Active host registrations win before compiled `IMGS`
entries of the same name; compiled entries retain their authored last-wins
order. A name that matches neither keeps the image's solved box and scene node,
suppresses its `Image` frame op, and reports `warn[img-missing]` once for that
unique name during the solve.

The runtime registry is instance-owned:

- `inst_img_register(i, name, w, h, format, data)` requires nonzero dimensions
  and accepts PNG (`format=0`, fully decoded dimensions must equal `w,h`) or
  straight-alpha sRGB RGBA8 (`format=1`, exactly `w*h*4` bytes). Invalid input
  is rejected atomically. A first registration appends one stable index after
  compiled images; replacing the
  same name keeps that index and advances its generation only when dimensions,
  format, bytes, or active state actually change; an equal registration is a
  no-op and does not dirty the instance.
- `inst_img_unregister(i, name)` deactivates the registration, preserves its
  reserved index for later reuse, and dirties only when an active entry changed.
  Resolution then falls through to a compiled image of the same name.
- `inst_img_info` exposes `(w,h,format,generation)` for active unified indices;
  `inst_img_bytes` borrows the single kernel-owned payload. Generated Rust
  `Doc` wrappers and web components expose register/unregister pass-throughs.

GPU and web clients cache decoded/uploaded images by unified index and
generation, replacing stale resources and releasing unregistered ones.
Standalone SVG embeds PNG bytes directly and converts RGBA8 registrations to
embedded PNG; the PNG renderer composites either format. TUI intentionally
uses the same labeled placeholder degradation as compiled images.

## 14. Motion

The load-bearing rule: **animation interpolates inputs, then re-solves.**
A document is a pure function `(states, t) → frame`; keyframes are
time-indexed attribute patches (the same payloads as `when`); every
intermediate frame is a normally solved document, so the containment
invariant (§6.1) holds at every instant for free. Animating `w` genuinely
reflows.

### 14.1 Keyframes: `anim` + `animate`

```slab
anim pulse {
  0%   { opacity=1;    bg=color.green }
  100% { opacity=0.25; bg=color.mint }
}
rect w=9 h=9 radius=999 animate=pulse,1100,alternate,ease-in-out
```

- `anim NAME { <pct> { attrs } … }` at top level; stops sort by position.
  An attribute animates between the stops where it appears; outside its
  first/last stop it clamps (declare 0%/100% for full-cycle control).
- `animate=NAME,dur[,loop|once|alternate][,easing][,delay]` — durations and
  delays are **plain numbers in milliseconds** (Slab has no unit suffixes).
  `once` holds its final frame; easing applies to the whole cycle.
- Time comes from outside: `--t MS` renders one instant; `--dur S --fps N`
  renders an APNG sequence, one solve per frame; interactive drivers pass
  `t_ms` to every `inst_frame`. Omitting `--t` samples 0ms.
- `content="…"` is a discrete keyframe attribute on `text` nodes. The
  sampled string participates in text measurement and layout, so web, GPU,
  TUI, PNG, and APNG output all re-solve for each displayed value. Binding
  such an animation to another node kind warns and ignores its content
  stops. Standalone SVG cannot express text replacement in CSS: it freezes
  the authored content and reports `cap-anim-content`.
- **Lifting.** A driver may call `inst_lift_animations` to take over every
  binding whose native replay is indistinguishable from the kernel overlay:
  static keyframes over ink-only attributes — `offset`/`opacity` on paint
  leaves or render-only containers, `rotate`/`scale` (including two-axis
  scale) on `rect`/`image`/`path` leaves, solid `bg` on plain rects and paths,
  and `color` on text — outside `each`. Paint-only tracks may coexist with
  signals, actions, and patches that do not switch their paint channel;
  geometry tracks require an interaction-free subtree with no scroll,
  detached paint, holes, or conditional materialization. The web driver gives
  every lifted node one stable, node-sized compositing group: `offset` and
  `opacity` animate that group so every paint op and child moves/fades
  together, while leaf-local transforms and colors animate their native paint
  element. Transform deltas require static bases; a base quarter-turn remains
  kernel-owned, and an animated `rotate` track may cross 90°/270° only for a
  statically square leaf, whose swapped layout is identical. The lift is
  normalized for native replay: whole-cycle Slab easing is remapped into
  time-domain stop positions carrying each segment's exact quadratic-
  restriction Bézier, and OKLab color tracks are subdivided until a native
  sRGB lerp stays within one 8-bit quantization step. Lifted bindings stop
  driving kernel motion — a fully lifted document solves once and goes idle —
  and the web driver emits split group/paint `@keyframes` with per-segment
  `animation-timing-function`s. Everything else stays "interpolate inputs,
  re-solve".

### 14.2 Interpolation

Numbers and percents lerp; **colors lerp in OKLab** (perceptually even —
gradient STOP ramps stay sRGB to match SVG); tuples lerp elementwise.
Strings, enums, flags, mismatched kinds, and other discrete values hold the
earlier stop until the next stop (step-start). Easing curves are Slab-defined
(exact formulas, not CSS beziers): `linear`, `ease-in` t², `ease-out`
1−(1−t)², `ease-in-out`/`ease` piecewise quadratic.

### 14.3 Transitions: easing state changes

```slab
row #card transition=200,ease-out {
  when hover { bg=color.surface; offset=0,-2 }
}
```

When a `when` condition flips between frames, a node with `transition`
applies the patch **interpolated** from the attribute's base value. The
kernel tracks the clocks itself: a State-condition patch's activity flip
is stamped with the observing solve's `t_ms`, and while
`age = t − flip − delay < dur` the patch's attrs apply as
`lerp(base, target, ease(p))` entering — and the mirror (`1 − p`) leaving.
Only State-condition flips tween; env/client/width flips re-solve without
tweening. The document stays a pure function of its inputs and `t` — the
flip stamps are the only retained clock, and they live in the kernel
instance, not the host.

Attributes without an explicit base value step at the midpoint, except
colors and solid paints, which fade through the target color at alpha 0
(CSS `transparent` semantics) on both the entering and leaving leg; flags
and extra `when` children never tween. Transition clocks start at the
first frame that observes a state flip, so sparse frame sampling shifts
the window with it.

### 14.4 What motion refuses

No physics, no scroll-linking (host territory — drive a param per frame), no
animating tree structure (enter/exit is opacity/offset keyed on `#id`
presence), no per-segment easing. Slab is still not a programming language.

## 15. Interaction

Interaction is **kernel-owned**: a driver translates platform input into
kernel `Event`s, calls `inst_dispatch`, and acts on the returned `Effects`
— events in, Effects out; the driver holds no interaction policy. There
are no capture/bubble phases and no handler registration: the kernel
routes hover, capture, focus, scroll, and editing internally, and the app
observes signals (§13.3). Exact struct shapes: spec/FRAME.md.

```
Event   { etype, x, y, dx, dy, button, clicks, key, text, mods }
Effects { repaint, signals (name/text/item/meta), scroll changes,
          caret rect, IME rect,
          cursor (default|pointer|text|col-resize|row-resize), focus }
```

`key` is the named key as a string (`"Tab"`, `"Enter"`, `"ArrowLeft"`,
`"a"`, …); `mods` is a bitset (1 shift | 2 alt | 4 ctrl | 8 meta).
`clicks` is the host-computed pointer-down click count (`0`/`1` is single;
exactly `2` is double). `Effects.repaint` doubles as the dirty mark: the next
`inst_frame` re-solves.

### 15.1 Node keys (identity)

Every node gets a stable key path **at compile time** (SLIR `NODE.key`).
Per-child segment precedence: explicit `key=v` → `v`; else `#id` →
`#<id>`; else `<kind>@<n>` where `n` is the ordinal among *unkeyed
same-kind* siblings. Full key = parent key + `"/"` + segment (root: bare
segment). Component calls contribute their own segment; expanded body
roots and slot children continue under the call's key. Diagnostics:
`dup-id`, `dup-key` (§12). All interaction state — node states, scroll
offsets, focus, edits — is keyed or node-addressed inside the kernel and
survives every re-solve by construction.
Synthetic descendants of `each` add the stable `~<item-key>/` segment
specified in §13.6; public state APIs accept those full keys.

### 15.2 Scene and hit testing

`flatten` emits the draw ops AND the retained scene in one DFS, so paint
order and absolute coordinates agree by construction. `inst_hit(x, y)`
returns the chain root → target: candidates are tried in **reverse paint
order**; a candidate hits when the point — transformed into each rotated
ancestor's local space (rotate −deg about `(cx, cy)`, outermost first;
sin/cos are the kernel's deterministic quadrant-reduced polynomials) —
lies inside its rounded rect AND inside every clipping ancestor's rounded
rect (radius clamp `min(r, w/2, h/2)`, identical to the painters). `inert`
subtrees never hit and never focus.

Any node may export platform accessibility semantics. `role=` is an open
identifier (`role=button`) or string (`role="application-specific"`), not a
closed Slab enum. `label=` and `desc=` accept string literals, Text
parameters, and Text item props. The remaining authored surface is:

| attributes | accepted value | exported meaning |
|---|---|---|
| `checked` | `false\|true\|mixed`, Bool, compatible enum, or Text resolving to one of those states | optional tri-state checked value |
| `expanded`, `selected`, `modal`, `live-atomic` | Bool | optional disclosure, selection, modality, and live-region atomicity |
| `active-descendant`, `controls` | Text containing one exact full Slab key | optional selection/disclosure relationship |
| `value-now`, `value-min`, `value-max` | Num | optional range value and bounds |
| `value-text` | Text | optional human-readable range value |
| `live` | `off\|polite\|assertive`, compatible enum, or Text resolving to one of those priorities | optional live-region priority |
| `level`, `pos-in-set`, `set-size` | Num | optional hierarchy level and collection position/cardinality |

Every value may be a compatible scalar list prop. Absence is preserved:
optional booleans do not collapse to false, optional numbers do not collapse
to zero, and `checked=mixed` remains distinct from true. `level` and
`pos-in-set` are positive integers; `set-size` is a positive integer or `-1`
for unknown; `pos-in-set` cannot exceed a known set size. Static range values
must satisfy `value-min <= value-now <= value-max` for every statically known
bound. Invalid literals are compile-time `a11y-range` errors; invalid dynamic
numeric combinations are omitted from that solved scene entry.

Relationship Text is one full key in the same format as `SigMeta.key` and
`sceneSnapshot().key`, never an id segment or whitespace-separated list.
Static literals must name a node in the compiled scene (`a11y-key` otherwise).
Adapters resolve dynamic values against the current scene and omit a
relationship whose target is currently absent.

The resolved `SceneNode` includes these values, its existing parent index and
bounds, plus derived `disabled` and `focused` booleans. Native string fields
are references into the instance scene STRS pool: reference 0 means absent;
the pool starts with the empty string, deduplicates static and runtime values,
and remains append-only for the instance lifetime. The WASM snapshot resolves
those references to strings and exports optional booleans/numbers as JSON
`null` when absent.

Accessibility adapters are framework-owned. The web client maintains a
key-retained, pointer-transparent shadow-DOM semantic hierarchy. DOM ids derive
from complete stable scene keys rather than synthetic node numbers, so
relationships survive list rematerialization. Scene rotations are applied to
the equivalent nested DOM geometry. Exactly one enabled focusable has a
sequential tab stop: the kernel-focused node, or the first focusable when focus
is initially empty; subsequent Tab movement remains kernel-owned. The adapter
maps metadata to real `role`/`aria-*` attributes and routes semantic focus and
default click actions into ordinary kernel focus/activation.

The native GPU client publishes the same scene through AccessKit and routes
only actions with existing kernel semantics (focus, default activation,
divider increment/decrement, reveal, and scrolling); unsupported actions such
as general SetValue are not advertised. TUI, SVG, and PNG retain metadata in
the kernel scene for programmatic consumers but do not claim a platform
accessibility-tree adapter. A custom driver must provide equivalent platform
tree plumbing; applications only author semantics and handle their ordinary
signals and policy.

### 15.3 Focus

`focusable` nodes participate in tab order; **document order IS tab
order**. `Tab`/`Shift-Tab` walk the ring, and the arrow keys walk it too
whenever the focused node is neither an edit field nor a scrollable on the
arrow's main axis (`Right`/`Down` forward, `Left`/`Up` back). Keyboard-driven
focus sets `focus-visible`; pointer focus sets only `focus` (ring-free).
Restoration rule: when the focused
key vanishes after a re-solve, focus moves to the nearest following entry
of the previous focusables list (then nearest preceding), else clears.
Focusing a `field=` node — keyboard focus included — binds its
`EditState` on first focus, seeded from the node's content (§15.6).
Hosts move focus for dialogs and wizards through `inst_set_focus`
(FRAME.md): the target must be focusable in the current scene, and the
same `focus-visible` rule applies — `visible` selects the keyboard-grade
ring, a cleared or pointer-grade focus shows none.

### 15.4 Events and dispatch (deliberately simpler than the DOM)

17 event types (kernel codes 0–16): `pointer-move pointer-down pointer-up
wheel key-down text paste copy cut composition-start composition-update
composition-end blur resize close inspect` plus `activate`, which is
**synthesized internally only** — pointer-up over the still-pressed
focusable, or Enter/Space key-down on the focused non-edit node;
`disabled` suppresses it.

`keys=Escape,F2` declares additional activation keys and implies `focusable`.
On key-down, dispatch walks from the focused scene node through its parents;
the first enabled node whose `keys` list contains the event key receives the
synthesized activate event. A disabled match is skipped so an enabled ancestor
may handle it. Routing precedence is field-edit commands, focused divider
adjustment, focused scrolling, `keys=`, default Enter/Space activation, then
Tab/arrow focus navigation.
While an edit field is focused, single printable keys stay in the text-input
path; unconsumed named keys may bubble.

The portable named-key vocabulary is `Enter Space Escape Tab Backspace Delete
Insert Home End PageUp PageDown ArrowLeft ArrowRight ArrowUp ArrowDown` and
`F1`–`F24`; any single printable character other than comma is also valid.
Authored `Space` is canonicalized to the event key `" "`. Unknown names produce
`warn[attr]`, because a platform-specific driver may never emit them.

- Primary pointer-down (`button=0`) first fires the deepest enabled `press=`
  binding in the hit path, then captures the nearest focusable node (or the
  raw target), sets `pressed`, and applies pointer-grade focus. Hover
  enter/leave still follows the whole uncaptured hit path.
- Secondary pointer-down (`button=2`) fires the deepest enabled `context=`
  binding and has no pressed or focus side effects. Auxiliary buttons likewise
  do not press, focus, or activate.
- A primary down with `clicks == 2` fires the deepest enabled `dblclick=`
  binding. When one is found, that gesture's later Activate is suppressed;
  ordinary single-click activation remains pointer-up over the captured node.
- Each pointer move emits PointerMove on the deepest enabled `pointer-move=`
  binding in the captured owner path, or in the current hit path when there is
  no capture. Each pointer-button release likewise emits PointerUp through
  `pointer-up=` before any primary-only activation or drag completion. An
  outside release still reaches the captured binding even though it cannot
  Activate the pressed node.
- A primary down on or below `drag=` arms the deepest such ancestor. Captured
  movement starts Drag once Euclidean document-space distance is strictly
  greater than 4 units, emits DragStart on the source, sets its `dragging`
  state, and suppresses Activate. That threshold-crossing move and every later
  active move emit DragUpdate with per-event `dx/dy` and cumulative
  `drag_dx/drag_dy`. While active, the deepest enabled `drop=` node under the
  pointer is marked `drop`, excluding the source and every descendant in its
  subtree. Enter/leave updates that state.
- Primary pointer-up over an eligible target emits Drop on the target. Its
  ordinary item identity is the target's; `SigMeta.src_key/src_item` identify
  the source. DragEnd then fires once on the source with
  `cancelled=false,dropped=true`. Release without a target still fires DragEnd
  once with both booleans false. Every release clears `pressed`, `dragging`,
  and `drop`.
- `blur`, close, a vanished or disabled source, and host pointer cancellation
  clear the same gesture state without Drop and emit exactly one DragEnd with
  `cancelled=true,dropped=false`, using the last cached pointer metadata.
  Cancellation discovered during a solve is queued; live hosts must call
  `inst_take_signals` immediately after the settled frame and deliver the
  returned signals once.
- Signal order is stable: a threshold-crossing move is PointerMove,
  DragStart, DragUpdate; later moves are PointerMove, DragUpdate; successful
  release is PointerUp, Drop, DragEnd.
- With `drag-ghost`, flatten appends a `0.72`-opacity duplicate of the source
  subtree above normal content. It follows the pointer while preserving the
  pointer-down grab offset, contributes draw ops only, and never adds a
  `SceneNode`, hit target, accessibility node, DragEnd, or other signal.
  Authors still use `when dragging` and `when drop` for source/target styling.
- Modifier clicks need no separate trigger: every signal's metadata includes
  event modifiers, button, click count, coordinates/deltas, emitter key,
  drag displacement, and termination outcome.
- Printable input reaches edit fields as `text` events; `key-down` carries
  named keys. `wheel` routes through scroll containers (§15.5). `resize`
  (`dx/dy > 0`) updates env. `copy` and `inspect` carry no kernel semantics.
- Activate, Press, Context, Dblclick, DragStart, DragUpdate, DragEnd,
  PointerMove, PointerUp, and Drop use empty text; Change, Submit, and Resize
  carry text. A real (non-list) node uses an empty item key (§13.3).
- Drivers coalesce pointer-moves through their demand-driven frame loop
  (immediate mode: dispatch, then repaint iff something changed).

### 15.5 Scroll

Scroll offsets are **kernel-owned, key-addressed, and independent per axis**.
Axis `0` is the container's main axis; axis `1` is its cross axis. Flatten
shifts children by each active offset inside the same clipped viewport and
retains `content_main` and `content_cross`, each including trailing padding.
Every offset clamps independently to
`[0, max(0, content_axis − viewport_axis)]`; a fresh solve re-clamps fresh
geometry (dirty for the next frame, never a mid-frame mutation).

Wheel dispatch treats `dy` as the main-axis delta and `dx` as the cross-axis
delta; Shift swaps them. Each delta routes to the deepest hit-path node with
that axis active, so nested owners may consume different deltas and a
`scroll=both` node may consume both in one dispatch. Every actual change emits
one axis-qualified `Effects.scrolls` entry; a clamped no-op emits none. Keyboard
scrolling remains main-axis only: focused columns consume `Up`/`Down`, focused
rows consume `Left`/`Right`, and the off-axis pair may walk the focus ring.
Arrow steps are 40u (200u with Shift), `PageUp`/`PageDown` step by viewport
minus 40u, and `Home`/`End` select zero/maximum.

`scrollbar=never|auto|always` controls geometry for every active axis (default
`never`; `auto` requires overflow). `scrollbar-w` defaults to 4u and
`scrollbar-fg`/`scrollbar-bg` default to `#80808080`/`#33808080`. A main-axis
track occupies the cross-end edge; a cross-axis track occupies the main-end
edge, both with a 2u inset. Thumb length is
`min(viewport, max(viewport²/content, 16u))`, positioned proportionally to that
axis's retained offset. Tracks and thumbs are ordinary `Rect` ops.

A direct `sticky` child of a main-scroll container paints at
`min(max(slot − offset, 0), next_sticky_slot − offset − extent)`. Normal
siblings paint first, then sticky children, then scrollbars. The retained scene
uses those painted rectangles and the same order, so clipping and reverse-order
hit testing match the frame. Cross-axis and end-edge sticky are not part of v1;
unsupported placement is a compile-time `sticky-ctx` error.

Hosts call `inst_set_scroll(i, key, axis, offset)` and
`inst_get_scroll(i, key, axis)`. `inst_reveal(i, key, margin)` minimally moves
both active axes of every scroll ancestor, applying each inner displacement to
the target rectangle before considering its parent. Rust bindings and web
components expose the same axis argument and reveal operation.

Web, GPU, and TUI clients support wheel, keyboard, reveal, sticky placement,
and host-API scrolling. SVG and PNG have no interaction and report
`cap-scroll`; with `scrollbar=always`, they still paint both active tracks and
thumbs at offset zero.

### 15.6 Editing

Editing is kernel-owned on `field=` text nodes (§13.3). Fields are single-line
unless they carry the `multiline` flag.

- `EditState` keeps caret/anchor/selection on **grapheme cluster** boundaries
  using the UAX #29 subset implemented by the Rust kernel. It also owns
  horizontal viewport offset, vertical goal-x, and bounded undo/redo history.
- `text` and paste insert committed text. Dispatch maps CR/LF codepoints to
  spaces before a single-line insertion; multiline fields preserve them.
  Composition events manage an inline IME run plus the `composing` node state.
  Every committed mutation, including undo/redo, fires Change with the full
  restored text; caret-only moves only repaint.
- Enter routing is exact: multiline without `submit=` inserts a newline for
  plain, Shift-, or Alt-Enter; multiline with `submit=` submits on unmodified
  Enter while Shift- or Alt-Enter inserts a newline; single-line with
  `submit=` submits on Enter; single-line without it is inert. Submit carries
  the full committed text and does not also emit Change.
- Backspace/Delete delete one cluster. Ctrl- or Alt-Backspace deletes the
  preceding space-delimited word; the same modifiers with Delete remove the
  following word. Ctrl- or Meta-W deletes the preceding word. Ctrl-K deletes
  caret to visual-line end and Ctrl-U deletes visual-line start to caret.
  Paste, cut, word deletion, and kills are single undo groups.
- ArrowLeft/Right move by cluster, by space-delimited word with Alt, or to
  document start/end with Ctrl/Meta; Shift extends selection. In multiline
  fields ArrowUp/Down move by **visual line** using the last solved wrapped
  `TextLayout` and preserve a goal-x across consecutive vertical moves;
  horizontal movement or an edit resets it. At the first/last line they move
  to document start/end. Home/End move to
  visual-line start/end; Ctrl- or Meta-Home/End move to document start/end.
  Single-line Home/End retain document behavior. Shift extends any movement;
  Ctrl/Meta-A selects all.
- Ctrl/Meta-Z undoes and Ctrl/Meta-Shift-Z redoes. History is capped at 100
  snapshots. Consecutive same-kind inserts/deletes then coalesce; a caret or
  selection command, mutation-kind transition, or insertion ending in
  whitespace starts another group. Paste, cut, word deletion, and kills
  always start a group. Any new edit clears redo.
- Caret and IME geometry is line-aware and describes the LAST solve:
  x is the visual-line prefix width minus the field's horizontal edit scroll;
  y is the line origin; h is that line's height; w is 1. Hosts refresh it
  after the next frame.
  Caret-anchored popover recipe: the host reads `Effects.caret_*`, writes
  the values into two `num` params consumed by an overlay child
  (`at=param.pop-x,param.pop-y`, §13.1), and toggles a bool param — no
  kernel positioning API is needed.
- The focused field paints its non-empty selection as a half-alpha band of
  its resolved text color (alpha `0x80`): one solid rect per visual line,
  emitted before the glyphs inside the field clip. Selection therefore
  exists only on interactive clients — a static render never has focus, so
  no selection op is ever emitted there (svg/png stay `none (cap-edit)` in
  the support chart).
- A single-line field clips its display text and horizontally scrolls it to
  keep the caret inside an 8u content-box inset, clamped at zero.
  Multiline fields do not horizontally scroll; after a mutation or caret
  command the kernel scrolls the nearest `scroll` ancestor enough to reveal
  the caret, clamped by the ordinary retained-scene scroll limits.
- The embedding owns IME plumbing and the clipboard. Web uses a hidden
  `<textarea>` for field focus and prevents its native Enter behavior after
  forwarding one key event, so the browser cannot duplicate a kernel newline
  or Submit; native forwards winit IME and uses the line-aware candidate rect;
  TUI forwards named Enter and editing keys. Kernel `cut`/`copy` touch no
  system clipboard, and the GPU/native clipboard degradation remains as
  charted. Shaping stays
  per-codepoint advances; complex scripts (Arabic, RTL) are out of scope.

### 15.7 Driver duties

The surface lifecycle is owned by each driver, documented rather than
abstracted: the browser element (ResizeObserver + matchMedia for
dark/coarse), the winit window, and the terminal (size → cell grid) send
logical sizes and env flags into `inst_set_env` / `resize` events. Ops
stay in logical units end to end; **only the driver multiplies by device
scale** (web: CSS px; gpu: the projection; png: `--scale`).
`Effects.cursor` maps to the platform cursor. Terminal drivers enable
bracketed paste (pastes arrive as one `paste` event, §15.6) and, where
supported, the kitty keyboard protocol so Shift+Enter is distinguishable
from Enter; in legacy terminals Alt+Enter remains the newline fallback
for multiline fields. Frame loops are
demand-driven: schedule a frame only when the instance is dirty or the
kernel reports live motion. Hot reload and inspectors are host features
outside this contract — kernel state is keyed (§15.1) and survives
instance rebuilds by construction.

## 16. Kernel implementation and adding a platform

### 16.1 One Rust kernel and deterministic execution

The kernel — evaluation of decoded SLIR, when/anim, layout, flatten, hit,
focus, edit, dispatch, motion, and cells — is maintained once in the Rust crate
`crates/slab-kernel`. Native clients link that crate directly. Browser clients
use `crates/slab-kernel-wasm`, a representation bridge that exposes the same
Instance, Frame, dispatch, scene, and conformance behavior to JavaScript
without reimplementing kernel policy.

Every conforming build MUST preserve these determinism rules:

- All model math is IEEE-754 `f64`; `+`, `-`, `*`, `/`, and `sqrt` use their
  correctly rounded IEEE operations.
- All integer arithmetic retains wrapping 32-bit two's-complement semantics.
- Iteration order from a hash map MUST NOT reach observable output; ordered
  output is produced from explicitly stable sequences.
- Results from `sin`, `cos`, `pow`, and `cbrt` pass through the existing
  domain-level tolerance or integer quantization before they can affect frame
  output. Conformance never depends on intermediate transcendental bits.
- Model code performs no float-to-string formatting. Canonical frame output
  uses only the `fmt3` boundary format specified in §11.4.

`just gen` rebuilds only live generated assets, including capability tables,
the support chart, and the browser WASM artifacts; `just freshness` fails CI
on drift. Correctness is proved against one executable contract:
`cargo test -p slab-kernel` covers the Rust implementation, while
`slab conformance` and `bun run tools/conformance-wasm.ts` byte-compare native
and WASM frames, cell grids, traces, and capability reports against the same
goldens (§11.4).

### 16.2 Adding a driver

A new SURFACE (a compositor, a canvas, a printer, an e-ink panel) is a
driver over the Frame contract — no kernel changes:

1. A native driver decodes the SLIR envelope with the protobuf binding,
   constructs a kernel `Doc`, then calls `inst_shell`, assigns the document,
   and calls `inst_init`. A representation bridge may instead own this
   decoding step and accept SLIR bytes, as `slab-kernel-wasm` does. Re-send
   the document or update the instance on resize as appropriate for the driver.
2. Each frame (only when dirty or animating): `inst_frame(t_ms)` → paint
   `Frame.ops` in order (§11.3). Text uses metric tables for layout and a
   registered runtime face, bundled fallback, or platform fallback for paint;
   atlas renderers use `text_glyphs`. Honor clip/group/rotate as a stack.
3. Translate platform input to `Event`s → `inst_dispatch` → obey
   `Effects` (§15). Mount holes (§13.2); deliver signals (§13.3).
4. Declare capabilities in `spec/support.toml`, then
   `cargo run -p xtask -- gen-caps` + `support-md`: every degradation is
   chart-spec'd and reported by code (§11.5, §12) — never silent, never
   ad hoc.

The reference drivers are small by design: the web element, the wgpu
renderer, and the interactive TUI all follow exactly this shape.

### 16.3 Adding a language binding

A new host language does not receive a second kernel implementation. Rust
hosts link `slab-kernel` directly; other hosts invoke it through a narrow FFI
or WASM representation bridge. `slab-kernel-wasm` is the reference browser
bridge. The host then implements a driver over the Frame contract (§16.2).
Any new bridge MUST preserve the Instance and Frame semantics and pass the
shared conformance goldens byte for byte in the environment it serves.

## 17. Reference CLI

```
slab check FILE                          # compile, print diagnostics (exit 1 on errors)
slab build FILE -o OUT.slir [--no-embed-assets]
slab dump  FILE.slir                     # canonical slir-dump text (spec/SLIR.md)
slab fmt   FILE... [--check]             # canonical formatter ('-' filters stdin)
slab render FILE -o OUT.{svg,png,apng,txt}
     [--client web|gpu|tui|svg|png] [--width N --height N] [--scale N]
     [--t MS] [--dur S --fps N] [--state a,b] [--env portrait,dark,coarse]
     [--set param=value]... [--plain]
slab conformance [--update] [--emit-slir DIR]
slab gen wc   FILE -o DIR [--tag NAME] [--separate-ir]   # web-component module
slab gen rust FILE -o OUT.rs                             # typed Rust module
slab lsp                                 # stdio LSP server (diagnostics, completion, hover, formatting, slab/preview)
```

- `check` is compile-only and env-independent; the `--width`, `--height`,
  `--state`, `--env`, and `--client` flags are accepted for compatibility
  and ignored (env-sensitive checking is the kernel's job, via
  `render`/`conformance`).
- `fmt` rewrites files in place (`--check` only reports, exit 1 on drift).
  It is line-preserving — statements are never merged or split — and
  normalizes indentation, spacing, and entry-name alignment in
  `tokens`/`theme`/`params`/`anim` blocks; comments and strings survive
  verbatim. The same formatter backs the LSP `textDocument/formatting`.
- `render` defaults: 800u × unbounded height, client from the extension
  (svg / png / tui); `--t` renders one instant; `--dur` (seconds) +
  `--fps` encode an APNG, one solve per frame; `--set` coerces to the
  declared param type and fails on unknown names or members; `--plain` is
  the ANSI-free golden cell format.
- `gen wc` emits `<stem>.ts` + `.d.ts` + a bundled `.js` (one element for
  the document, one per exported def); `gen rust` emits the typed module
  of §13.1/§13.3.

Toolchain and driver binaries beside `slab`:

```
slab-tui FILE.slab [--script 'TOKENS'] [--dump-after PATH] [--debug]
slab-native --demo settings [--headless-frame OUT.png]
```

`slab-tui` is the interactive terminal driver (terminal size drives env;
headless `--script` replays and `--dump-after` matches
`slab render --client tui --plain` byte for byte); `slab-native` is the
winit/wgpu driver demo built on `slab gen rust` output.

## Platform support

One row per feature, one column per client (the `when` renderer
classes, §2; the column order is the kernel client code, §11.2). **full**
= renders as specified. **degraded** = renders with the documented
approximation; the note is the whole contract. **none** = the
renderer skips the feature and reports the `cap-*` code once per
document (§12 notes; TUI notes accumulate on the cell grid, static
exporters print to stderr). Machine-readable source:
`spec/support.toml`; the shared driver lookup table is generated into
`crates/slab-kernel/src/caps.rs` by
`cargo run -p xtask -- gen-caps`.

<!-- support-chart:begin -->
<!-- GENERATED from spec/support.toml by `cargo run -p xtask -- support-md`; edit the toml, not this table. -->

| feature | web | gpu | tui | svg | png |
| --- | --- | --- | --- | --- | --- |
| radius | full | full | degraded — corner radius collapses to single arc glyphs (radius >= 4u draws ╭ ╮ ╰ ╯, smaller radii draw square corners) | full | full |
| shadow | full | degraded — outset shadows use blurred-SDF approximations | none (`cap-shadow`) | degraded — the shadow spread field is ignored by the SVG filters (research parity; the raster honors it) | full |
| blur | full | full | none (`cap-blur`) | full | full |
| backdrop | full | full | none (`cap-backdrop`) | degraded — approximated by re-emitting the ops beneath the panel, clipped and blurred, behind it | full |
| gradient | degraded — dashed or per-side gradient strokes fall back to their first stop | degraded — gradients sample at most 8 stops (cap-gradient-stops) | degraded — sampled per cell (linear angle, radial farthest-corner, conic sweep), alpha-composited | full | full |
| gradient-conic | full | degraded — gradients sample at most 8 stops (cap-gradient-stops) | degraded — sampled per cell over the paint box | degraded — approximated by a 90-wedge fan, each wedge solid at its center sample | full |
| gradient-text | full | degraded — gradients sample at most 8 stops (cap-gradient-stops) | degraded — sampled per cell over the text node's content box | full | full |
| path | full | full | degraded — strokes as slope-charred cell runs, fills as even-odd scanline at cell centers | full | full |
| path-runtime | full | full | degraded — uses the same cell-center path approximation as compiled path data | full | full |
| icon | full | full | degraded — scale transforms are ignored and icon paths render in design-box coordinates | full | full |
| image | full | full | degraded — drawn as a shaded placeholder block labeled with the source basename | full | full |
| img-runtime | full | full | degraded — drawn as the same shaded placeholder block used for compiled images | full | full |
| rotation | full | full | none (`cap-transform`) | full | full |
| scale | full | full | none (`cap-transform`) | full | full |
| tilt | full | full | none (`cap-transform`) | degraded — affine three-corner fit (no foreshortening) | full |
| smooth | degraded — inset shadows and shadow spread keep circular corners | degraded — shadows and clips keep circular corners | degraded — same arc glyphs as radius | full | full |
| grain | full | full | none (`cap-grain`) | degraded — feTurbulence approximation (different noise realization) | full |
| mask | full | full | degraded — mask alpha sampled per cell over the box | full | full |
| backdrop-fade | degraded — 6-band blur approximation | degraded — 6-band blur approximation | none (`cap-backdrop`) | degraded — 3-band blur approximation | degraded — 6-band blur approximation |
| animation | full | full | full | degraded — animates paint attributes; text content and layout-affecting keyframes freeze at authored base (cap-anim-content) | degraded — re-solved per frame and encoded as APNG at the requested --dur/--fps |
| text-keyframes | full | full | full | degraded — content keyframes freeze at the authored base (cap-anim-content) | full |
| transition | full | full | full | none (`cap-transition`) | none (`cap-transition`) |
| themes | full | full | full | full | full |
| input | full | full | degraded — pointer positions are quantized to character-cell centers | none (`cap-input`) | none (`cap-input`) |
| ime | full | degraded — an Esc-cancelled composition stays open until the next commit and dead-key commits arrive as plain text events | none (`cap-ime`) | none (`cap-ime`) | none (`cap-ime`) |
| scroll | full | full | full | none (`cap-scroll`) | none (`cap-scroll`) |
| scroll-cross | full | full | full | none (`cap-scroll`) | none (`cap-scroll`) |
| scroll-reveal | full | full | full | none (`cap-scroll`) | none (`cap-scroll`) |
| sticky | full | full | full | none (`cap-scroll`) | none (`cap-scroll`) |
| divider | full | full | full | degraded — renders authored or host-preset divider geometry; interactive resizing has no static dispatch loop | degraded — renders authored or host-preset divider geometry; interactive resizing has no static dispatch loop |
| holes | full | full | none (`cap-hole`) | degraded — the hole box renders empty; a hug axis has no host report, so it uses zero before ordinary min/max clamps | degraded — the hole box renders empty; a hug axis has no host report, so it uses zero before ordinary min/max clamps |
| lists | full | full | full | full | full |
| lists-nested | full | full | full | full | full |
| para-runs | full | full | full | full | full |
| lists-virtual | full | full | full | full | full |
| popover | full | full | full | full | full |
| signals | full | full | full | none (`cap-signal`) | none (`cap-signal`) |
| a11y | full | full | none (`cap-a11y`) | none (`cap-a11y`) | none (`cap-a11y`) |
| text-edit | full | degraded — cut/copy/paste do not reach the system clipboard (winit has no clipboard API) | full | none (`cap-edit`) | none (`cap-edit`) |
| text-raster | degraded — browser-rasterized glyphs; kernel line breaks and advances are authoritative | full | degraded — wide clusters (EAW W/F, emoji presentation) occupy two cells; runs re-quantize to columns under the real font metrics | degraded — viewer-rasterized glyphs; textLength force-fits each run to the kernel-measured width | full |

<!-- support-chart:end -->

## 18. Changelog

- **1.1.x** — wave 2 (component-parity fixes). The focused field paints
  its selection kernel-side (§15.6); hosts move focus with
  `inst_set_focus` (§15.3); `num`/`pct` params are legal in tuple member
  positions (new SLIR value tag `TupleDyn`, §13.1) — the caret-anchored
  popover recipe writes caret geometry into two params; Shift+Arrow
  scrolls 5× (§15.5); slab-tui enables bracketed paste and the kitty
  keyboard protocol and paints real images over the cell placeholder
  where the terminal supports kitty graphics; `slab fmt` and LSP
  `textDocument/formatting` reformat sources canonically (§17).

- **1.1.0** — the dynamic-content release. Typed `list(Def)` params and
  `each` add stable keyed rows; multiline fields add wrapped navigation,
  submit signals, kill bindings, and coalesced undo/redo; holes may hug
  host-reported content. Named themes, configurable scrollbars and typed
  scroll APIs, custom activation keys, discrete text keyframes, and
  Unicode-aware wide terminal cells complete the five-client surface.
  `inst_lift_animations` hands CSS-translatable `animate=` bindings to the
  driver (§14.1): the web element replays them as `@keyframes` and a fully
  lifted document solves once and idles instead of re-solving per frame.
  Breaking changes:
  - **rule 11** — discrete keyframes hold the earlier stop until the next
    stop; the former midpoint switch is removed.
  - **rule 12** — `hug` is legal on holes and resolves from the host's
    reported content size before ordinary min/max clamps; the former
    `err[hole-hug]` is removed.
  - **rule 13** — navigation keys scroll a focused scrollable before they
    walk the focus ring.
  - Signal effects gain `sig_item`; Rust signal bindings emitted by
    `slab gen rust` and web event details carry a list item's stable key.
  - **SLIR 2.0** replaces the sectioned 1.x wire format with a
    Snappy-compressed protobuf envelope. Recompile documents and regenerate
    embedded Rust/web artifacts; there is no compatibility shim.
- **1.0.0** — the kernel release. One pipeline replaces 0.5's parallel
  SDKs: ONE Rust compiler (`slab-compile`) lowers to binary **SLIR**
  (spec/SLIR.md); ONE semantic kernel owns layout, styling, motion, hit
  testing, focus, editing, and dispatch (spec/FRAME.md). The kernel is now
  maintained once in Rust (`slab-kernel`), linked directly by native clients,
  and executed on the web through `slab-kernel-wasm`; thin drivers — web
  custom element (`slab gen wc`), native wgpu (`slab-native`), interactive
  TUI (`slab-tui`), static SVG/PNG/APNG exporters — paint frames and forward
  events. New language surface: `params` (six typed host inputs), `hole`
  (host-filled viewports), `act=`/`field=` signals with kernel-owned
  single-line editing, `export` defs (standalone embeddable components).
  Native and WASM conformance is byte-identical across frames, cells, traces,
  and capability reports. Breaking changes:
  - **rule 6** — renderer class `gui` is renamed **`gpu`** (`when gpu`);
    `gui` is now an ordinary (inactive) state ident.
  - **rule 7** — family fallback metrics route any authored `family`
    containing `mono` (ASCII-case-insensitive) to JetBrains Mono and every
    other name to Inter; weights snap to 400/500/600/700 (ties up). The
    authored name remains in SLIR, and a runtime-registered matching face
    overrides those fallback metrics and paint data. Every client still
    solves from real font tables, including cell media.
  - **rule 8** — new `warn[shadow]`: a def param shadowing an
    attribute/flag name or the `fill`/`hug` keywords is reported instead
    of silently winning.
  - **rule 9** — the host injection/selector API is REMOVED; params,
    holes, signals, and exported defs are the host surface (§13).
    `key-missing` is retired with it.
  - **rule 10** — top-level `when <cond> { tokens … }` overrides compile
    to per-site patches (token **site expansion**); token refs inside
    deferred patch values resolve against base tokens (compound
    conditions are not representable).
  Also visible: text baselines sit at CSS half-leading over the real font
  box (0.5 centered a fictional 0.76-em box; `text.y` shifts ≈ 1u at size
  14), and the 0.5 JSON wire-op serialization (old §11.1) is replaced by
  SLIR + canonical frame.json.
- **0.5.0** — the SDK release: independent from-scratch implementations
  (Python reference `sdks/py`; Rust `sdks/rust` with wire/TUI/SVG and a
  `gpu` runtime feature; Go `sdks/go` with wire/TUI; C11 `sdks/c` with a
  C++ wrapper) sharing one uniform application surface — events stage,
  `frame()` solves once, injections persist last-write-wins, polling
  queries (`clicked/hovered/focused/set_state/scroll`) answer against the
  last built frame; batch converters (`parse/build/to_wire/render_*`)
  everywhere. `conformance/` becomes the executable contract (18 graded
  cases, reference-metrics wire ops ±0.05, byte-exact plain TUI).
  The `slab-wgpu` native host folded into the Rust SDK (gpu-feature bin);
  the standalone `wgpu-host/` crate is gone. Parser fix:
  nested color functions re-serialize without spurious spaces. Editor
  tooling: stdlib LSP (`slab lsp`), `slab preview`, tree-sitter grammar,
  VS Code + Zed extensions.
- **0.4.0** — the application-runtime release: stable node identity
  (`key=`, derived key paths, `dup-id`/`dup-key`/`key-missing`), per-node
  state scoping (§10), retained `Scene` with hit testing/focus/scroll
  clamping built in the same DFS as the draw list, renderer registry +
  `Caps`, the normative wire format (§11.1) with its golden fixture, the
  reference host runtime `slab/app.py` (dispatch without capture/bubble,
  one synthesized `activate`, pointer capture, tab order = document order,
  focus restoration), host-owned key-addressed scroll offsets, host-side
  virtualization (`window()`), the stdlib grapheme/caret text kernel
  (`cap-shaping` boundary), hot reload with last-good-frame semantics, the
  pixel→source inspector, `el()` escape-proof injection + `Selection.one()`
  + the `measure_text` memo, and TWO op consumers: the browser host
  (`slab/serve.py`, SSE + mini DOM runtime + served solver fonts) and the
  native wgpu/Metal host (`wgpu-host/` + `slab/native.py`, NDJSON stdio
  protocol, headless CI probes).
- **0.3.0** — effects & motion: gradient paints (`linear`/`radial`, stop
  ramps in sRGB); layered + `inset` shadows; `stroke-align`/`stroke-sides`;
  self `blur` (layer filter); `backdrop` glass primitive (the one op that
  READS the canvas — png blurs in place, SVG re-emits prior ops clipped +
  blurred, TUI paints flat); `anim` keyframes + `animate` + `transition`
  with OKLab color interpolation and Slab-defined easing curves; time as an
  input (`--t`, `--dur/--fps` → APNG encoder, host `Timeline`); SVG exports
  paint-level keyframes as CSS. Glassmorphism ships as a token recipe, not
  a primitive. Containment holds at every animation instant by construction
  (interpolate inputs, re-solve).
- **0.2.0** — the immediate-mode release: direct `png` renderer (pure-stdlib
  rasterizer: analytic-coverage rounded rects, scanline nonzero paths and
  TrueType glyphs, dashes, box-blur shadows, opacity/rotation layers) whose
  `PngMetrics` measures with the same font that paints — layout and pixels
  agree with no textLength crutch; images degrade to placeholders
  (`cap-image`), missing fonts diagnose (`cap-font`). Host embedding (§13):
  templates, per-frame instances, `kind#id` descendant selectors,
  AST-level injection (text/props/children/remove) with token-aware value
  parsing; render calls raise on error diagnostics. Normative fix: `%`
  requires a determinate parent axis — against hug it degrades to hug +
  `pct-unbounded` (the progress-bar-in-hug trap).

- **0.1.6** — iteration 6 (cassette J-card, subagent-authored): quarter-turn
  `rotate` participates in layout (swapped constraints, rotated bbox) while
  arbitrary angles stay ink-only; specified nowrap overflow (truncate +
  `clipped`, silent with `ellipsis`); truncation diagnostic reworded with a
  remedy hint.
- **0.1.5** — iteration 5 (terminal departure board, subagent-authored):
  `gap=main,cross` two-value form (grid row gap / wrap line gap);
  documented TUI quantized metrics, paint-time grid snapping and
  cell-multiple authoring guidance, stroke→box-drawing degradation, the
  `rect h=1` rule idiom, multi-node def splicing, and cross-media
  truncation divergence. `pad` on `text` reserves gutters that survive
  ellipsis truncation.
- **0.1.4** — iteration 4 (transit-map poster, subagent-authored):
  `stroke-dash` on paths/boxes (replaces hand-chopped subpaths); `anchor=`
  9-position on canvas children (`at` addresses the anchor point — kills the
  pre-subtracted-coordinate arithmetic); `tracking` text attr (inherits);
  documented `group` as a first-class node, `at` anchor semantics, and
  renderer cap/join behavior (solid = round/round, dashed = butt).
  Deliberately rejected: value arithmetic (`at=x-14`) — anchors remove the
  main motivation and Slab is not a calculator.
- **0.1.3** — iteration 3 (authored by a fresh agent from spec alone):
  `\_` escape = non-breaking space (widow/orphan control; NBSP glues words);
  opacity is group-composited via `GROUP_PUSH/POP` IR ops; documented pad
  orders, stack/canvas hug defaults, filled paths, spacer overrides; added
  `slab check --tree` geometry dump. Known limitation (unchanged): baseline
  alignment does not propagate across equal-height siblings — align card
  titles with consistent padding instead.
- **0.1.2** — iteration 2: baseline-aligned items keep natural cross size;
  hug-cross accounts for baseline shifts; offset-caused exceed is ink (no
  double opt-in); grid cells honor `self=` as justify-self.
- **0.1.1** — iteration 1: `\` line continuation; `style=` bundles on any
  node (explicit mixins, still no cascade); `TEXT` IR op carries measured
  run width so vector renderers force-fit (`textLength`) — solver and pixels
  agree exactly; reserved-`fill` misuse gets an explanatory diagnostic.
- **0.1.0** — initial spec: box protocol, sequential measure + deflation
  ladder, containment invariant, six containers, tokens/defs/when, draw-list
  IR, SVG + TUI reference renderers.

