# Slab language reference

Contents: [Syntax](#syntax) · [Nodes](#nodes) ·
[Runtime paths & icons](#runtime-paths--icons) · [Sizing](#sizing) ·
[Layout algorithm](#layout-algorithm) · [Scroll, overlays & dividers](#scroll-overlays--dividers) ·
[Boundary rule](#boundary-rule) · [Text & data runs](#text--data-runs) ·
[Components](#components) · [Tokens, themes & when](#tokens-themes--when) ·
[Identity & keys](#identity--keys) · [Accessibility semantics](#accessibility-semantics) ·
[Diagnostics](#diagnostics)

## Syntax

KDL-flavored. A node is `name [#id] (arg|attr=value|flag)* [{ children }]`.
Newlines or `;` separate siblings. `//` line and `/* */` block comments.
Node headers end at a newline. Put `\` at each continued line end:

```slab
text "Save" w=fill \
  family="sans" \
  weight=700
```

Indentation does not continue a header. A missing `\` produces one recovery
diagnostic for the continued attribute run.

```ebnf
document := stmt*
stmt     := tokens | theme | params | def | icon | anim | topwhen | node
params   := "params" "{" (IDENT ptype "=" pdefault)* "}"
ptype    := "text"|"num"|"pct"|"color"|"bool"|"enum"(a,b,…)|"list"(UIDENT)
def      := "def" UIDENT "(" dparams ")" ["export"] block
dparam   := IDENT ["=" (scalar | "list" "(" UIDENT ")")]
icon     := "icon" IDENT ["viewbox" "=" NUMBER] "{" path+ "}"
node     := NAME ["#" IDENT] (arg | IDENT "=" value | flag)* [block]
each     := "each" (REF | IDENT) ["#" IDENT] (IDENT "=" value | flag)*
value    := keymap | scalar ("," scalar)*      // 2+ scalars form a tuple
keymap   := (IDENT|STRING) ":" (IDENT|STRING) ("," …)*
scalar   := NUMBER | PCT | STRING | #HEX | REF | IDENT | IDENT ":" NUM | fn(...)
when     := "when" cond block
cond     := IDENT | "!"IDENT | ("w"|"h") OP NUM | "theme" "(" IDENT ")"
```

This is an authoring sketch, not a replacement grammar. Use `spec/SPEC.md`
for the complete grammar, especially recursive list defaults and token forms.

- lowercase `NAME` = builtin; Capitalized = component call.
- `REF` is a dotted path (`color.bg`) — always a token, EXCEPT the reserved
  head `param.`: `param.title` reads a declared param. Bare idents in value
  position are keywords or component props, never tokens.
- `#word` after a name = id; `#hex` in value position = color.
- Bare string children are text runs (meaningful inside `para`; on
  `text`/`span` they set the content).
- `\_` in strings = non-breaking space (widow control).
- Reserved attrs on any node: `key=` (identity, never style). Signal binders:
  `act=`, `field=`, `submit=`, `press=`, `context=`, `dblclick=`,
  `pointer-move=`, `pointer-up=`, `drag=`, `drag-update=`, `drag-end=`,
  `drop=`, `resize=`. `act`/`field`/`press`/`drag` imply `focusable`;
  placement and payload rules are in hosts.md. Use
  `keys=Escape,F2 act=cancel` when several keys share one action, or the typed
  `keys=Escape:close,F2:rename` map for distinct actions. A mapped `keys=`
  owns activation routing and cannot be combined with `act=`. Both imply
  `focusable`; the fired key is in `SignalMeta.key`.
  `field-sync=host` is the compiler-only opt-out when a differently named
  field signal is intentionally reconciled by the host.
- Accessibility attrs start with `role= label= desc=` and include checked,
  disclosure/selection, exact-key relations, value/range, modal/live, and
  level/set metadata; the complete typed list is below.
- `attach=`, `gravity=`, and `collide=` place an anchored child of
  `stack`/`canvas`.
- `multiline` is legal only on a `field=` text node. `escape-blur` is an
  explicit editable-node opt-in: Escape clears focus while retaining the edit
  buffer. `drag-update=`, `drag-end=`, and `drag-ghost` require `drag=` on that
  same node. `virtual`, `item-extent`, `overscan`, and `sticky` are
  context-restricted below.
- A `cond` ident resolves in order: client classes `web gpu tui svg png` →
  env idents `portrait landscape dark coarse` → component props → bool
  params → state idents (per-node, then global).

## Nodes

Containers:

| node | places children |
|---|---|
| `row`, `col` | along an axis in document order with `gap` (sugar for `box axis=…`; `when` may patch `axis`) |
| `wrap` | like row, new line when out of room; `fill` takes its line's remainder |
| `grid` | column tracks agreeing across rows: `cols=120,fill,hug`; `span=N`; `self=` is justify-self |
| `stack` | layers, later = above. Children position via `self=<9-pos>` + `offset=x,y`. **Overlap opt-in** |
| `canvas` | children at `at=x,y`; `anchor=<9-pos|center>` picks which point of the child lands there. **Overlap opt-in** |
| `para` | strings and `span`s wrap as one paragraph with per-run styling |
| `group` | plain box usable with `at=` inside canvas (flow island) |

9-pos vocabulary: `top-start top top-end start center end bottom-start
bottom bottom-end`.

Leaves: `text "…"` (wraps by default), `span`, `rect`,
`img src=… fit=cover|contain|stretch`, `path "M…"` / `path d=…` (canvas
only), `spacer`, `divider`, `icon NAME`, and `hole NAME`. `img src` and
`path d` accept a literal, Text param, or Text item prop. `spacer` is
`rect w=fill`/`h=fill` by parent axis; attrs override. A `hole` is a
host-filled childless viewport with sizing plus `scroll`/`clip` (hosts.md).

Root: one node; multiple top-level nodes wrap in an implicit `col`. Root
receives invocation constraints (default 800u wide, unbounded height).

## Runtime paths & icons

Keep dynamic paths inside a `canvas`; bind the entire `d` value rather than
constructing it in Slab:

```slab
params { route text = "M4 20 L28 4 L52 20" }
canvas w=56 h=24 {
  path d=param.route bg=none stroke=#38BDF8 stroke-w=3
}
```

The kernel normalizes and caches each distinct runtime string; normalized
geometry bounds determine intrinsic size. Invalid data paints nothing and
reports `attr` once for that string.

Declare each reusable icon once at top level with one or more static `path`
children. `viewbox` is a positive square design size and defaults to 24.
Use `current` only for the declaration paths' `bg`/`stroke`; omitted `bg`
means `current`.

```slab
icon chevron viewbox=24 {
  path "M6 9 L12 15 L18 9" bg=none stroke=current stroke-w=2
}
params { leading_icon text = "chevron" }
row color=#2563EB {
  icon chevron size=16
  icon param.leading_icon size=20 color=#DC2626
}
```

An icon is a `size × size` box (`size` defaults to inherited text size).
`color` supplies `current`. Dynamic names re-resolve on solve; an unknown
name preserves the box, paints nothing, and reports `icon-missing` once per
unique name. Do not put dynamic paths or non-path nodes in a declaration.

## Sizing

Per axis `w=`/`h=`: fixed number (a request, clamped by constraints) |
`hug` | `fill[:weight]` | `%` (of parent content box). Clamps:
`min-w max-w min-h max-h`.

Defaults: main axis hug; cross axis stretch (= fill) for containers AND
`rect`, hug for other leaves (block-like familiarity: a col child fills the
width; `rect h=1` is a full-width divider; cards in a row get equal height).
Inside `stack`/`canvas` everything hugs both axes; `fill`/`%` there resolves
against the container's bounds. `hole` takes the container cross-fill
default; a `hug` hole axis uses the host-reported natural size (0 before
the first report).

`%` requires a determinate parent axis (fixed/%/fill-given); against hug it
degrades to hug + `pct-unbounded` warning.

A leaf used as an `each` item root cannot consume the list's bounded item
space directly. Explicit `w=fill` or `h=fill` there resolves as hug and emits
`fill-unbounded`. Wrap the leaf in a fill-sized `row` or `col`.

## Layout algorithm

Padding follows CSS direction order without CSS shorthands: one value applies
to all sides; two values are `pad=vertical,horizontal`; four values are
`pad=top,right,bottom,left`. Three values are not accepted.

`measure(node, cons)` — constraints down, sizes up, parent places.
A child is never given more space than actually remains.

Row/col: (1) non-fill children measure in document order against
`remaining` (earlier children win; remaining floors at 0); (2) fill
children share `max(0, remaining)` by weight; (3) cross: natural measure,
then stretch children re-measure once at container cross; baseline-aligned
items keep natural cross (baseline beats stretch). Placement: `pack=start|
center|end|between` (main), `align=start|center|end|baseline` (cross),
per-child override `self=`.

**Containment invariant**: in flow containers every child rect lies inside
the parent's content box and siblings are pairwise disjoint. Overlap is
only expressible via `stack`/`canvas`/`offset` and never leaks out of the
declaring container.

**Deflation ladder** when demand exceeds supply (total, never overlap):
fill takes leftover → hug re-measures tighter (wrap → hard-break →
ellipsis if flagged) → fixed clamps (**`squeeze`** diagnostic) → residue
clips (**`clipped`** diagnostic; `bleed` to paint outside knowingly).

Grid: hug tracks = max natural width of non-spanning cells; fixed clamp;
fills share leftover; row height = max cell height. Spans clamp to their
tracks' total (`squeeze` if short).

Bare `scroll` activates the container's main axis; `scroll=cross` activates
only its cross axis; `scroll=both` activates both. Each active axis measures
children unbounded, clips to the viewport, and owns an independent keyed
offset. Scrollbars apply to every active axis:
`scrollbar=never|auto|always`, `scrollbar-w`, `scrollbar-fg`, `scrollbar-bg`.
See hosts.md for axis numbers, notifications, and reveal.

## Scroll, overlays & dividers

Pin `sticky` only on a direct child of a main-axis scroll container. It pins
to main-start and the next sticky child pushes it away; cross-axis and
end-edge sticky are unsupported.

```slab
col#feed h=320 scroll=both scrollbar=auto {
  row sticky h=32 bg=#111827 { text "Files" }
  col w=640 { /* overflowing content */ }
}
```

Anchor a popover by putting it directly in `stack` or `canvas`. Pass an exact
full key (usually a Text param populated from signal `meta.key`):

```slab
params { anchor text = ""; menu_open bool = false }
stack#surface {
  row#more act=open_menu { text "More" }
  when menu_open {
    col#menu w=180 attach=param.anchor gravity=below-end collide=auto
      offset=0,6 { text "Rename" }
  }
}
```

Choose `below|above|left|right` with `-start|-center|-end`; gravity defaults
to `below-start`.
`collide=auto` (default) flips on main-edge overflow, then slides into the
root viewport; `none` preserves placement. `offset` applies last. A missing
anchor omits the overlay subtree from paint and hit testing. Keep dismissal,
outside-click, focus trapping, and restoration in the host.

Place the focusable `divider` between two nonempty sibling positions in a
`row`/`col`; it controls the previous sibling's main extent:

```slab
row w=640 h=360 {
  col#sidebar w=240 min-w=160 max-w=360 { }
  divider#split w=6 resize=sidebar_resized dblclick=sidebar_reset
  col w=fill min-w=240 { }
}
```

The kernel owns capture, continuous clamped resizing, and double-click reset.
A row divider uses Left/Right plus a `col-resize` cursor; a column divider
uses Up/Down plus `row-resize`. Steps are 8u (Shift = 1u), and both panes'
minima are respected. There is no collapse threshold or content-aware
allocation; drive those policies with params/`when` and the host extent API.

## Boundary rule

Uniform across all containers: ink inside bounds = fine; exceed without a
flag = clip + `clipped`; exceed caused solely by the child's own `offset`
= declared overlap, passes as ink; `bleed` on the container = paint
outside knowingly; `clip` = clip silently. Ink effects (shadows, outer
stroke halves) are exempt — ink, not geometry.

## Text & data runs

Wraps at word boundaries; over-long words hard-break. `nowrap` disables
wrapping (truncates + `clipped` unless `ellipsis`). `ellipsis` truncates
the last line with `…`. A `para nowrap ellipsis` is one composite line across
all spans; the ellipsis inherits the last retained span's style.
`align-text=start|center|end`. Inheritance
whitelist (the ONLY inheritance in Slab): `color family size weight
leading tracking strike`. `leading` = line-height multiplier (default 1.4);
`tracking` = letter-spacing in u after every glyph. `strike` is a boolean,
defaults to false, and bare `strike` means true; it changes paint only, never
measurement, wrapping, or line height.

For host-supplied rich text, put `each` directly inside `para` and make its
exported schema body exactly one `span`:

```slab
def Run(content="", tone=#E5E7EB, emphasis=400) export {
  span content color=tone weight=emphasis
}
params {
  runs list(Run) = [
    Run(content="git ", tone=#94A3B8),
    Run(content="status", tone=#38BDF8, emphasis=650)
  ]
}
para { each param.runs }
```

Any other run-template body is `each-span`. Runs participate in one paragraph
layout; do not render them as separate `text` nodes.
Bind run props directly to span content, `color`, `size`, `weight`, `family`,
`tracking`, and `strike`; the whole run list reflows as one paragraph.

Conditional display strings are host-computed: project state into a text param
or list field (`"✓"`, `"due in 3m"`, timer captions) rather than looking for a
ternary/content expression. For 1 Hz displays, rebuild the typed visible-row
projection once per second and call the generated `set_rows`; equal-key,
equal-field diffing preserves retained item state and avoids needless work.

Fonts: the compiler embeds complete metric/coverage tables (SLIR `FONT`) for
the vendored faces — an authored `family` containing `mono`
(ASCII-case-insensitive) maps to JetBrains Mono, everything else to Inter;
weights snap to 400/500/600/700 (ties up). The authored name is preserved;
a runtime-registered face of the same name overrides metrics and paint.
Every client solves from the same real font tables. Shaping is
per-codepoint advances; complex scripts (Arabic, RTL) are out of scope.

The selected SLIR font cmap is authoritative coverage on every client.
Codepoints absent from it keep their deterministic fallback advance but paint no
platform fallback or tofu glyph. Static literals and known parameter/list-field
defaults receive `glyph-missing` compiler warnings. Host-provided runtime text
emits the same code as a frame diagnostic once per family and codepoint.

## Components

```slab
tokens { color { accent #4FC7E0; amber #F2B24C } }
def Gauge(label, pct, tone=color.accent) {
  col w=fill pad=8 {
    text label size=11
    row h=8 bg=#222 clip { rect w=pct h=fill bg=tone }
  }
}
Gauge#cpu label="CPU" pct=63% tone=color.amber
```

- Lexical macro expansion; props are values referenced by bare ident.
- Call-site children splice at the single `slot` node in the body. Slotted
  children keep the CALLER's style context but take geometry defaults from
  the slot's parent (CSS `::slotted` semantics).
- A body may have multiple top-level nodes — all splice as siblings (the
  grid-row idiom: one def emits six cells = one visual table row).
- Call-site `#id` becomes a key path SEGMENT above the component's root; the
  root keeps its own segment (see Identity & keys).
- Truthiness for `when prop`: absent/`false`/`0`/`""` are false.
- A def param shadowing an attr/flag name or `fill`/`hug` warns `shadow`.
- `export` after the parameter list makes the def a standalone embeddable
  document and a `list(Def)` schema. A def prop may declare a child-list type
  with `children=list(ChildDef)`, including `children=list(Self)` (hosts.md).
- Component macro recursion is capped at 32; recursive list data is
  runtime/data-bounded. There are no loops or arithmetic.

## Tokens, themes & when

```slab
tokens { color { bg #0e1116 }  space { md 16 }  text { title { size 18; weight 650 } } }
when tui { tokens { space { md 1 } } }   // top-level: per-client token overrides
theme dusk { color { bg #16131f } }      // sugar for: when theme(dusk) { tokens { … } }
```

`style=text.title` merges a token group as attrs; explicit attrs win.
Shadow tokens are tuple entries: `shadow { crisp 0,2,6,#00000040 }`.

**Themes** are named, compiler-checked token override sets. The host selects
one by name (`inst_set_theme`, web `theme` attribute, CLI `--theme`);
unknown names are rejected; the empty name restores the authored base.
Every scalar token reference retains its dotted path and resolves through the
active theme at evaluation time: direct attrs, values in deferred `when`
patches/animations, and values passed through def defaults or explicit args all
behave identically. Missing theme leaves fall back to authored base.

Use the simple state form:

```slab
when hover { bg=color.bg }
```

Do not duplicate it under `when theme(NAME)`. That old workaround is removed.
Nested `when` blocks report `error[when-compose]`; direct
`when theme(dusk) { … }` remains for genuinely theme-specific structure or
behavior, not palette selection.
Rust hosts can query the same resolved table without allocation via
`inst_get_token(&instance, "color.bg") -> Option<TokenValue<'_>>`; colors are
the usual packed RGBA word, numbers are `f64`, and other scalar forms are
borrowed canonical text.

`when` is the ONE mechanism for variants/states/media/responsiveness,
lexically attached, last patch wins:

```slab
row transition=200,ease-out {
  when hover    { bg=#26314A }        // per-node state (kernel dispatch)
  when tui      { pad=1 radius=0 }    // client class: web | gpu | tui | svg | png
  when dark     { bg=#05070B }        // env: portrait landscape dark coarse
  when w<600    { axis=col }          // reads the INCOMING constraint (no feedback)
  when playing  { stroke=#4FC7E0 }    // component prop truthiness / bool param
}
```

State ident match order: component-prop scope → bool param → the node's own
states (keyed by identity path) → the global set. Canonical per-node names
(not enum-enforced; unknown idents are inactive): `hover pressed focus
focus-visible disabled selected composing dragging drop`. Dispatch owns
hover/pressed/focus/composing and the drag states. Hosts drive app states
(`disabled`, `selected`, …); the global set is for previews (`--state`) and
document-wide conditions. Author drag feedback with `when dragging` on the
source and `when drop` on a target.
The CLI `--state a,b` flag populates only that global preview set; it cannot
target one node. Use host-driven node state for per-node previews.

Binders and `animate=` may appear inside `when` on the node the block patches.
The compiler registers the union of every branch's signal names statically,
then the active patch cascade gates them at runtime. A false binder condition
suppresses dispatch, pointer behavior, editing, and tab focus. If deactivation
invalidates current focus, focus is restored or cleared in that solve; the
node's edit buffer, selection, and undo history remain retained for a later
reactivation. Overlapping active branches use source order: the last binding
for a trigger channel wins. A false animation condition stops its motion clock,
so idle repaint reaches zero.

```slab
params { editing bool = false; draft text = "Rename me" }
text#title param.draft color=color.ink {
  when editing {
    field=draft
    submit=commit
    bg=color.inset
    pad=6,10
    radius=6
  }
}
```

This is the preferred conditional-edit pattern: author one stable text node,
then conditionally activate its field and submit binders. Do not collapse a
permanently bound field to `h=0`; inactive conditional binders are absent from
hit testing and focus traversal while retained editing state survives.

## Identity & keys

Every node gets a stable key path at compile time. Segment precedence:
explicit `key=v` (escaped) → `#id` → `<kind>@<n>` (ordinal among unkeyed
same-kind siblings). Full key = `parent/segment`. Component calls contribute
their own segment; body roots and slot children continue under the call's key.
A call-site `#id` is therefore a segment ABOVE the component root's own
segment: `Filter#factive …` whose body root is a `row` yields
`…/#factive/row@0`, not `…/#factive`.

An `each` descendant inserts
`<each-full-key>~<item-key>/<template-relative-key>`; nested eaches repeat the
marker at every level. Literal `%`, `/`, and `~` in explicit `key=` or stable
item-key values are escaped as uppercase `%25`, `%2F`, and `%7E` in full keys.
`sig_item` remains the raw innermost item key; signal `meta.key` carries the
escaped unambiguous full path.

Node APIs accept an exact full key, a unique bare `#id`/`id`, or a unique
authored suffix rooted at an id (`#list/rows`). Component call ids resolve to
the actual first body root. Ambiguous shorthand fails with candidates. Prefer
generated constants or copy `sceneSnapshot().key`/`scene::key_of` rather than
hand-building anonymous segments. Node state, scroll, focus, edits, and
animation survive re-solve by canonical identity.

## Accessibility semantics

Author semantics on the painted node; all attrs are optional:

```slab
stack#app {
  col#issues role=listbox label="Issues" expanded=true focusable \
      active-descendant="#app/#issues/#issue-b" controls="#app/#detail" {
    row#issue-a role=option selected=false pos-in-set=1 set-size=42 act=select_issue { text "Login" }
    row#issue-b role=option selected=true pos-in-set=2 set-size=42 act=select_issue { text "Sync" }
  }
  col#detail role=region label="Issue detail" desc="Selected issue details" { }
  row#triage role=checkbox label="Partially selected" checked=mixed act=toggle_triage { }
  row#progress role=progressbar label="Upload" \
      value-min=0 value-max=100 value-now=65 value-text="65 percent" { }
  text#heading "Activity" role=heading level=2
  col#dialog role=dialog label="Conflict" modal=true { }
  text#status "Saved" role=status live=polite live-atomic=true
}
```

`role` accepts an open identifier/string. `label`, `desc`, `active-descendant`,
`controls`, and `value-text` accept Text literals/params/item props.
`checked` is `false|true|mixed` (dynamic Bool or compatible Enum);
`expanded`, `selected`, `modal`, and `live-atomic` are Bool. `live` is
`off|polite|assertive` and accepts a compatible Enum. `value-now`,
`value-min`, `value-max`, `level`, `pos-in-set`, and `set-size` are Num.
Known ranges require `value-min <= value-now <= value-max`. `level` and
`pos-in-set` are positive integers; `set-size` is positive or `-1` (unknown),
and a known size cannot be smaller than its position. Invalid literals are
`a11y-range`; invalid dynamic combinations are omitted for that solved node.
Absence stays distinct from false or zero.
Typed metadata attrs accept matching scalar params and item props.

Each relation names one exact full Slab key—the same string as
`SignalMeta.key`/`sceneSnapshot().key`, never an id fragment or key list.
Static relations are compile-validated. The web adapter omits a dynamic
relationship while its exact target is absent from the retained scene.

Live attrs publish semantic state; the kernel does not schedule or deduplicate
announcements.

No role is inferred. Semantics do not imply focusability, actions, visuals,
or hit behavior.
In particular, `selected=` does not activate `when selected`; drive both from
the same param/state when visual selection is required. Use `focusable`,
`inert`, and signal binders explicitly. The shipped web semantic tree and
native AccessKit bridge consume the retained scene; custom drivers must wire
an equivalent adapter (hosts.md). `spec/SPEC.md` is normative.

## Diagnostics

`file:line: level[code]: message`; codes with a remedy print it as indented
follow-ups. `slab check` exits non-zero on errors. It also compiles every
`export` definition as a standalone document. Export diagnostics keep the
source filename and add `in export NAME`. The list below highlights authoring
failures; `spec/SPEC.md` is the complete normative registry.
Layout diagnostics ride in frame output, and drivers report `cap-*` once per
document from `spec/support.toml` (rendering.md).

Compile time:

| code | level | meaning / remedy |
|---|---|---|
| `parse` | error | syntax error; an attribute run after a newline suggests the missing node-header `\` once |
| `ref` | error | unknown token/param/prop/component; token cycle; malformed value — check dotted paths and Capitalization |
| `param-type` | error | param default doesn't fit its type, or non-bool param used as `when` condition |
| `dup-hole` | error | one hole name declared twice |
| `list-def` | error | a `list(Def)` or nested `list(Def)` target is not an exported schema |
| `each-target` | error | root `each` is not a List param, or nested `each` is not a List item prop |
| `each-nest` | error | an `each` template contains a `hole` (nested `each` itself is legal) |
| `each-span` | error | an `each` directly in `para` does not expand to exactly one `span` |
| `virtual-extent` | error | virtual each lacks a positive numeric `item-extent` |
| `virtual-ctx` | error | virtual each is nested or not directly under a main-scroll row/col |
| `sticky-ctx` | error | sticky is not a direct child of a main-scroll container |
| `divider-ctx` | error | divider is first/last or not a direct row/col child |
| `attach-ctx` | error | attach/gravity/collide is not on a stack/canvas child |
| `icon-body` / `icon-dup` | error | icon body is empty/non-static-path, or a name repeats |
| `attr` | warning | unknown/ignored attr or misplaced reserved attr |
| `shadow` | warning | def param shadows an attr/flag name or `fill`/`hug` |
| `dup-param` | warning | duplicate param declaration; first wins |
| `dup-signal` | warning | one name bound to both Activate and a text-payload trigger (same-shape fan-in is legal) |
| `dup-token` / `dup-def` | warning | token path / component redefined; last wins |
| `dup-id` | warning | one `#id` resolved twice |
| `dup-key` | warning | sibling key collision; both kept |
| `fill-unbounded` | warning | explicit fill on a leaf `each` item root resolves as hug — wrap it in a fill-sized row/col |
| `glyph-missing` | warning | static text contains a character absent from its resolved embedded family; dynamic text is not compile-checked |

Layout time (kernel, per solve):

| code | level | meaning / remedy |
|---|---|---|
| `squeeze` | warning | fixed size clamped; names node and deficit — shrink the request or free space |
| `clipped` | warning | content truncated — wrap, resize, or flag `ellipsis` |
| `pct-unbounded` | warning | `%` against indeterminate axis — give the parent a determinate size or use fill |
| `fill-unbounded` | warning | fill against an unbounded axis (e.g. inside `wrap`) behaves as hug |
| `attr` | warning | one distinct runtime path string is invalid; that path paints nothing |
| `img-missing` / `icon-missing` | warning | dynamic name is unresolved; layout box remains but paint is suppressed |

Capability notes (`cap-*`) come from the generated support chart. Important
new boundaries: TUI approximates paths, ignores icon scale, and uses image
placeholders; SVG/PNG do not dispatch signals or scroll; static divider
output has no resize loop. Read rendering.md for practical degradations and
`spec/support.toml` for the exact normative matrix.
