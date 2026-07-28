# Slab styling & motion reference

Contents: [Style attributes](#style-attributes) · [Colors & gradients](#colors--gradients) ·
[Icon current paint](#icon-current-paint) · [Shadows](#shadows) ·
[Glass & blur](#glass--blur) · [Grain & masks](#grain--masks) ·
[Squircle corners](#squircle-corners) · [Strokes](#strokes) ·
[Rotation, scale & tilt](#rotation-scale--tilt) ·
[Interaction state & drag ghosts](#interaction-state--drag-ghosts) · [Motion](#motion)

## Style attributes

Closed set; everything else is composition.

| attr | applies to | values |
|---|---|---|
| `bg` | any box, path | color or gradient paint (NOT `fill` — that's a size keyword); `current` only in icon declarations |
| `stroke`, `stroke-w` | box, path | color or gradient paint; width (default 1); `current` only in icon declarations |
| `stroke-align` | box | `inside\|center\|outside` (default center) |
| `stroke-sides` | box | subset of `t,r,b,l` — tab underlines, list dividers (radius ignored) |
| `stroke-dash` | box, path | u pattern: `stroke-dash=16,14`; single value = even; butt caps |
| `radius` | box | number; `999` ≈ pill; clamped to `min(r, w/2, h/2)` |
| `smooth` | box, img | 0–1 corner smoothing (squircle; iOS ≈ 0.6); no-op unless `radius>0`; ink+clip only |
| `shadow` | box | preset `sm\|md\|lg`, inline `[inset,]x,y,blur,color`, or a LIST of presets/token refs |
| `blur` | any node | self blur in u — node+children render to a layer, blur, composite |
| `backdrop` | box | glass: `backdrop=blur[,saturation[,brightness]]` blurs what is already painted beneath |
| `backdrop-mask` | box with `backdrop` | any paint — backdrop strength scaled by paint alpha (progressive blur) |
| `grain` | box | `amount[,size]` — deterministic speckle over the node's fill area; amount 0–1, size in u (default 1) |
| `mask` | any node | any paint — subtree alpha-faded over the node's border box; ink outside vanishes |
| `opacity` | any | 0–1; composites as a GROUP (children blend first, fade as one) |
| `color` | text, icon usage | text: color/gradient over content box; icon: solid `current` tint. Inherits |
| `family size weight leading tracking` | text | inherit; leading = multiplier (1.4); tracking = u per glyph |
| `style` | any | token group as attr bundle (`style=fx.card`); explicit attrs win |
| `align-text` | text | `start\|center\|end` |
| `fit` | img | `cover\|contain\|stretch` |
| `scale` | any node | `1.05` or `sx,sy` — ink-only zoom about center; never layout; hit-testing keeps the layout rect |
| `tilt` | any node | `rx[,ry[,depth]]` — ink-only 3D perspective (degrees; depth in u, default 800) |
| `pad` | box | `16` / `v,h` / `t,r,b,l` |
| `gap` | containers | `8` or `main,cross` (grid row gap / wrap line gap; `gap=16,0` = table gutters) |
| `animate` | any node | `animate=NAME,dur[,loop\|once\|alternate][,easing][,delay]` |
| `transition` | any node | `transition=dur[,easing][,delay]` — ease this node's `when`-state patches |
| `scrollbar` etc. | scroll box | `scrollbar=never\|auto\|always`, `scrollbar-w`, `scrollbar-fg`, `scrollbar-bg` |

## Colors & gradients

CSS color strings: `#0e1116`, `#fff8` (4/8-digit alpha), `rgb(…)`,
`oklch(72% 0.16 250)`. Limited-gamut clients quantize.

Gradients go anywhere `bg` takes a color:

```slab
// angle: 0 = up, 90 = right (CSS convention)
rect w=240 h=80 bg=linear(135, #241A4E 0%, #E8865E 100%)
// radial: centered, radius covers the box
rect w=240 h=80 bg=radial(#FFE0B0 0%, #FFB37C00 100%)
// conic: centered sweep, clockwise from the REQUIRED from-angle (0 = up)
rect w=240 h=240 radius=999 bg=conic(0, #44CFFF 0%, #B48CFF 50%, #44CFFF 100%)
```

Stops are `color pct`; missing offsets distribute evenly; 8-digit alpha
participates in the ramp. Radial is non-configurable by design —
off-center glows are an oversized radial rect in a `stack` with `offset`.
Stop ramps interpolate in sRGB. Gradients apply to box fills, box strokes,
`path` fills/strokes, and text `color`; the one exception: web falls back
to the first stop for dashed or per-side gradient box strokes. TUI samples
gradients per cell.

**Gradient text**: `color=` takes any paint; the gradient maps over the
text node's content box, so a wrapped headline sweeps continuously across
all its lines. Inherits exactly like a solid `color`.

```slab
text "Neon nights" size=32 weight=700 color=linear(90, #44CFFF 0%, #FF7AC6 100%)
```

## Icon current paint

Use `current` only on static paths inside a top-level `icon` declaration.
It resolves from each icon usage's `color`, so one asset can inherit or take
an explicit tint:

```slab
icon alert viewbox=24 {
  path "M12 3 L22 21 L2 21 Z" bg=none stroke=current stroke-w=2
  path "M12 8 L12 14" bg=none stroke=current stroke-w=2
}

row color=#F59E0B {
  icon alert size=18                  // inherits #F59E0B
  icon alert size=18 color=#EF4444    // explicit tint
}
```

An icon path with no `bg=` defaults to `bg=current`; write `bg=none` for
stroke-only art. Do not use `current` on ordinary boxes, standalone paths,
or text. Icon `size` scales ink and the square layout box together; it is
not the ink-only `scale=` transform.

**Recipe — conic progress ring** (72% donut): a conic bg under a
knocked-out inner circle.

```slab
stack w=96 h=96 {
  rect w=96 h=96 radius=999 bg=conic(0, #3DDC84 0%, #3DDC84 72%, #232A36 72%, #232A36 100%)
  rect self=center w=76 h=76 radius=999 bg=#0e1116
}
```

**Recipe — animated shimmer border**: an oversized conic rect spinning via
`rotate` keyframes inside a clipped ring; the card body knocks out the
middle, leaving a 2u animated gradient border.

```slab
anim spin { 0% { rotate=0 } 100% { rotate=360 } }
stack w=240 h=140 radius=16 clip {
  rect self=center w=340 h=340 animate=spin,2400,loop,linear
    bg=conic(0, #44CFFF10 0%, #44CFFF 25%, #44CFFF10 50%, #44CFFF 75%, #44CFFF10 100%)
  col self=center w=236 h=136 radius=14 bg=#11161F pad=16 { text "shimmer" }
}
```

## Shadows

Single inline: `shadow=0,2,6,#00000040` or `shadow=inset,0,1,0,#FFFFFF40`
(inset paints ABOVE the fill — the rim-light trick). Presets `sm md lg`.

Layered shadows are ALWAYS a list of presets/token refs (comma tuples are
flat, so multi-shadow cannot be written inline):

```slab
tokens { shadow { ambient 0,28,64,#00000073; lift inset,0,1,0,#FFFFFF26 } }
col pad=16 bg=#171C26 radius=12 shadow=shadow.ambient,shadow.lift { text "card" }
```

GPU approximates outset shadows (blurred SDF) and skips inset
(`cap-shadow-inset`); TUI has none (`cap-shadow`).

## Glass & blur

Glassmorphism is a recipe, not a primitive — bundle as a token style:

```slab
tokens { fx { glass { backdrop 22,1.35,1.1; bg #FFFFFF12; stroke #FFFFFF3D; stroke-w 1; radius 20 } } }
col style=fx.glass pad=16 { text "frosted" color=#E8EEF6 }
```

`backdrop` is the ONE op that reads the canvas:
`backdrop=blur[,saturation[,brightness]]` blurs, then saturates and
brightens (both default 1), what is already painted beneath the node's
rounded rect; the node then paints over it. Web/GPU/PNG are full; SVG
approximates by re-emitting prior ops clipped + blurred; TUI paints flat
(`cap-backdrop`). `blur` (self blur) affects the node's own layer instead.

**Progressive blur**: `backdrop-mask=<paint>` scales the backdrop strength
by the paint's alpha over the node box — the footer-strip blur ramp:

```slab
rect w=fill h=120 backdrop=24 backdrop-mask=linear(180, #fff0 0%, #fff 100%)
```

Every client approximates with fixed blur bands (6 on web/gpu/png, 3 on
svg); TUI stays flat (`cap-backdrop`).

## Grain & masks

`grain=amount[,size]` paints a deterministic monochrome speckle over the
node's own fill area — texture for hero gradients, or a standalone overlay
chip over `bg=none`. `amount` is 0–1 alpha; `size` is the speckle cell in
u (default 1). The hash is fixed-seed and node-local, so grain stays
static under motion (no per-frame shimmer). `amount` tweens. SVG
substitutes an feTurbulence approximation (different noise realization);
TUI drops it (`cap-grain`).

```slab
rect w=360 h=180 radius=16 bg=linear(135, #241A4E 0%, #E8865E 100%) grain=0.12
```

`mask=<paint>` renders the node and its children as one layer, then
multiplies it by the paint's ALPHA mapped over the node's border box.
Coverage outside the box is 0 — ink outside the box vanishes; that is the
fade-out contract. The list edge-fade:

```slab
col scroll h=320 mask=linear(180, #fff 70%, #fff0 100%) { … }
```

Masks ride the same group as `opacity`/`blur`. Under animation the mask
paint is discrete (like all gradients). TUI samples the mask alpha per
cell.

## Squircle corners

`smooth=0..1` bends a rounded corner toward a squircle (Figma corner
smoothing; the iOS feel ≈ 0.6). No-op unless `radius>0`; ink and clip
only — geometry, layout, and hit testing keep the plain rect. Per the
support chart, inset shadows and shadow spread keep circular corners on
web, and shadows/clips keep circular corners on gpu.

```slab
col w=320 pad=20 radius=24 smooth=0.6 bg=#171C26 { text "squircle card" }
```

## Strokes

Stroke halves are ink, not geometry (outer halves may poke out of parents
legally). `stroke-align=inside` + all sides maps to a clean CSS border in
web media; side subsets render as per-side segments. Dashed strokes use
butt caps; solid paths round/round. `rect h=1 bg=…` is the
medium-independent hairline-rule idiom (renders as box-drawing in TUI).
TUI paints box strokes INSIDE as box-drawing cells sharing space with
content — bordered boxes need `pad=16,8`+.

## Rotation, scale & tilt

`rotate=deg` about the node center. Quarter turns (±90/270) are
LAYOUT-AWARE: the node measures against swapped constraints and occupies
its rotated bounding box (spine captions author in place). Any other angle
is ink-only — geometry untouched, paint tilts (the third overlap opt-in).
TUI skips rotated subtrees (`cap-transform`) — redesign with `when tui`.

`scale=1.05` (or `scale=sx,sy`) is an ink-only zoom about the node
center — the hover-pop primitive. It NEVER affects layout, and hit testing
keeps the layout rect, so a hovered card cannot oscillate. Numeric, so it
tweens under `transition`/`anim`. TUI skips (`cap-transform`).

```slab
row#card transition=150,ease-out pad=16 radius=12 bg=#1A2230 {
  when hover { scale=1.02 }
}
```

`tilt=rx[,ry[,depth]]` is ink-only 3D perspective about the node center:
CSS `perspective(depth) rotateX(rx) rotateY(ry)` (degrees; `depth` in u,
default 800; a single number is rx). The subtree flattens into one plane;
layout and hit testing keep the flat rect. Numeric, so it tweens. SVG
degrades to an affine three-corner fit (no foreshortening); TUI skips
(`cap-transform`).

CAUTION: `backdrop` inside a tilted subtree is browser-fragile (Chromium
flattening quirks) — prefer tilt on non-glass content.

## Interaction state & drag ghosts

Style the real source and current target with kernel states:

```slab
row drag=started drag-update=moved drag-end=finished drag-ghost {
  when dragging { opacity=0.55 }
}
col drop=accepted {
  when drop { stroke=#4ADE80 stroke-w=2 }
}
```

`drag-ghost` is a behavior flag, not a paint attr. During an active drag the
kernel duplicates the resolved source subtree above ordinary content, keeps
the original grab offset, and applies fixed opacity 0.72. It is not separately
styleable and has no scene/hit/a11y node. Do not draw a second host ghost.

## Motion

Load-bearing rule: **animation interpolates inputs, then re-solves.** A
document is a pure function `(states, t) → frame`; every intermediate
frame is a normally solved document, so containment holds at every
instant. Animating `w` genuinely reflows. Ops arrive already sampled at
`t` — there are no animation ops in the frame.

### Keyframes

```slab
tokens { color { green #3DDC84; mint #A8F0C6 } }
anim pulse {
  0%   { opacity=1;    bg=color.green }
  100% { opacity=0.25; bg=color.mint }
}
rect w=9 h=9 radius=999 animate=pulse,1100,alternate,ease-in-out
```

`animate=NAME,dur[,loop|once|alternate][,easing][,delay]` — durations are
plain numbers in **milliseconds**. An attribute animates between the stops
where it appears and clamps outside them (declare 0%/100% for full-cycle
control). `once` holds the final frame; easing applies to the whole cycle.

`animate=` may be state-gated:

```slab
rect w=9 h=9 radius=999 {
  when running { animate=pulse,1100,alternate,ease-in-out }
}
```

The binding is registered statically. Its clock and overlay run only while the
condition wins the animation channel. When false, it contributes no active
motion and does not keep an otherwise idle instance repainting. Conditional
bindings stay kernel-driven instead of using native animation lifting.

Time comes from outside: `--t MS` renders one instant (omitted → 0ms);
`--dur S --fps N` renders an APNG (one solve per frame); interactive
drivers pass `t_ms` to every `inst_frame`.

**Content keyframes**: `content="…"` inside a stop is a discrete keyframe
on `text` nodes — the sampled string participates in measurement, so
layout re-solves per displayed value (countdown/ticker idiom). Standalone
SVG can't express it and freezes the authored content (`cap-anim-content`).

**Lifting**: drivers may take over CSS-translatable bindings (static
`offset`/`opacity`/`rotate`/`scale`, solid `bg`, and text `color` keyframes
on non-interactive, unpatched leaves outside `each`; any easing) via
`inst_lift_animations` — the web element replays them as `@keyframes` with
exact per-segment curves and OKLab-faithful color stops, and a fully lifted
document solves once and idles. Automatic; nothing to author.

### Transitions

```slab
row#card transition=200,ease-out pad=12 bg=#1A2230 {
  when hover { bg=#26314A; offset=0,-2 }
}
```

When a state-condition flips between frames, the patch applies
interpolated from the base value; the kernel tracks flip clocks itself
(hosts hold no motion state). Entering eases `p`, leaving eases `1−p`
within `dur` after `delay`. Only State flips tween; env/client/width flips
re-solve without tweening. Static svg/png report `cap-transition`.

### Interpolation

Numbers/percents lerp; **colors lerp in OKLab** (stop ramps stay sRGB);
tuples elementwise. Strings, enums, flags, and mismatched kinds are
discrete: they hold the earlier stop until the next stop (step-start).
Transition attrs without a base step at the midpoint, EXCEPT colors/solid
paints which fade through the target at alpha 0 (CSS `transparent`
semantics); flags and extra `when` children never tween. Easings
(Slab-defined formulas, not CSS beziers): `linear`, `ease-in` t²,
`ease-out` 1−(1−t)², `ease-in-out`/`ease` piecewise quadratic.

### What motion refuses

No physics, no scroll-linking (host territory — drive a param per frame),
no animating tree structure (enter/exit = opacity/offset keyed on `#id`
presence), no per-segment easing. Slab is not a programming language.
