# SLIR — slab intermediate representation (protobuf + snappy)

SLIR is the compiled form of a `.slab` document. It carries the resolved node
structure, authored attributes, conditions, animations, host surface, font
metrics, and image payloads that the Rust kernel needs to produce frames.
Native clients pass SLIR bytes to `slab-slir`, whose Rust decoder constructs
the kernel's flat `slir::Doc` and initializes a `frame::Instance`. Browser and
Node clients pass the same bytes to the wasm-bindgen `KInst` constructor; the
WebAssembly bridge invokes that same Rust decoder and owns the resulting Rust
instance. No JavaScript-side protobuf decoding or parallel document model is
part of the runtime contract.

`spec/slir.proto` is the normative document schema. Its field numbers and
field types are wire compatibility commitments. The Rust protobuf binding is
committed in `crates/slab-slir/src/pb.rs`; consumers must use the canonical
`slab-slir` decoding entry points rather than write another protobuf codec.

## Envelope

Every file is exactly:

```text
0..4    ASCII "SLIR"
4..6    major u16 little-endian (2)
6..8    minor u16 little-endian (0)
8..     a raw Snappy block containing one protobuf `slir.Doc` message
```

The payload uses Snappy's **raw block** format, not the Snappy framed stream
format. Readers reject an incorrect magic or major version and reject a minor
version newer than the one they implement. Protobuf's normal unknown-field
behavior applies within an accepted envelope.

There is no section directory, alignment padding, fixed record layout, or
wire-level `BLOB` section in SLIR 2.0. Protobuf and Snappy own the byte layout;
only the envelope and `slir.proto` are normative.

## Document model

`Doc` is a flat structure-of-arrays model. Arrays describing the same entity
have equal lengths and share an index; `*_off`/`*_len` pairs are element
fenceposts into the named parallel pool, never byte offsets. The Rust decoder
validates cardinality, integer narrowing, pool ranges, and string references.
Compiler and diagnostic paths may retain a compiler-side `Slir` value; runtime
paths translate the decoded protobuf directly into the public
`slab_kernel::slir::Doc`.

| Schema fields | Meaning |
|---|---|
| `strs` | UTF-8 string pool. String 0 is the empty string and the optional-string sentinel. |
| `node_*` | DFS-preorder node arrays: kind, flags, links, key, id, and source line. |
| `aval_*`, `f64s`, `tup_dyn_*` | Typed attribute-value pool, tuple-number pool, and dynamic-tuple member pool. |
| `grad_*`, `shdw_*`, `path_*` | Gradient, shadow, and normalized path pools. |
| `attr_*` | Base per-node attribute fenceposts and `(id, value)` pairs. |
| `font_*` | Metric-only font tables and their cmap/advance pools. |
| `cond_*`, `patch_*`, `wattr_*`, `patch_children` | `when` conditions and patch payloads. |
| `anim_*`, `aattr_*`, `bind_*`, `trans_*` | Keyframes, animation bindings, and transitions. |
| `parm_*`, `list_*` | Scalar parameters, recursive list schemas, and normalized list defaults. `list_field_sub` is zero for scalar fields and one plus a nested schema row otherwise. |
| `theme_*`, `hole_*`, `sign_*` | Themes, host holes, and signals. |
| `icon_*` | Icon names, detached subtree roots, and square design-box sizes. |
| `img_*`, `img_data` | Image metadata and payloads. |

The compiler deduplicates values and pools where useful. Consumers must not
infer semantics from a particular ordering beyond the ordering explicitly
stated below.

### Links

Kernel node links use `0xFFFF_FFFF` for none. Protobuf cannot represent that
sentinel compactly without making a valid node link ambiguous, so
`node_parent`, `node_first`, and `node_next` use this encoding:

```text
protobuf 0       -> kernel NONE
protobuf n + 1   -> kernel node n
```

All other index references retain their ordinary zero-based meaning except
`list_field_sub`: zero means a scalar field and `n + 1` refers to nested list
schema row `n`. Node 0 is the root; a document with multiple authored roots has
a compiler-synthesized root `Col`.

### Attribute values

`aval_tag`, `aval_lo`, `aval_hi`, and `aval_num` are parallel arrays. For
numeric tags (`Num`, `Pct`, `SizeFixed`, `SizeFill`, and `SizePct`),
`aval_num[i]` is the f64 value. For every other tag, `aval_lo[i]` and
`aval_hi[i]` reconstruct the legacy 64-bit payload as:

```text
payload = aval_lo | (u64(aval_hi) << 32)
```

`aval_num` is zero for nonnumeric values. This split preserves exact numeric
semantics without relying on a protobuf integer representation for f64 bits.
The value tags and payload meanings are:

| tag | name | payload |
|---:|---|---|
| 0 | `Num` | f64 |
| 1 | `Pct` | f64, 0–100 |
| 2 | `Str` | STRS reference |
| 3 | `Color` | rgba8 in low u32 |
| 4 | `Tuple` | `lo` = `f64s` offset, `hi` = count |
| 5 | `SizeFixed` | f64 |
| 6 | `SizeHug` | none |
| 7 | `SizeFill` | f64 weight |
| 8 | `SizePct` | f64, 0–100 |
| 9 | `PaintSolid` | rgba8 in low u32 |
| 10 | `PaintGradient` | gradient index |
| 11 | `PathRef` | path index |
| 12 | `ShadowList` | `lo` = shadow offset, `hi` = count |
| 13 | `ParamRef` | parameter index |
| 14 | `EnumSym` | STRS reference |
| 15 | `PaintNone` | none |
| 16 | `PropRef` | Each-schema field index |
| 17 | `ListDefault` | `lo` = list-item offset, `hi` = count |
| 18 | `TupleDyn` | `lo` = `tup_dyn_*` offset, `hi` = count |
| 19 | `PaintCurrent` | inherited text color |

The writer may deduplicate equal AVALs and tuple runs; readers must not depend
on that optimization.

`TupleDyn` is emitted only when at least one tuple member is a `num`/`pct`
parameter reference; all-literal tuples keep tag 4. The member pool is the
parallel arrays `tup_dyn_tag` (0 literal, 1 param), `tup_dyn_num` (the
literal value, else 0), and `tup_dyn_param` (the parameter index, else 0);
the kernel reads the current parameter value per solve.

### Nodes, conditions, and pools

Node kinds are `Row=0`, `Col=1`, `Wrap=2`, `Grid=3`, `Stack=4`, `Canvas=5`,
`Para=6`, `Group=7`, `Text=8`, `Span=9`, `Rect=10`, `Img=11`, `Path=12`,
`Spacer=13`, `Hole=14`, `Each=15`, `Divider=16`, and `Icon=17`. Node flags
retain the kernel bit assignments: `clip`, `bleed`, `scroll`, `nowrap`,
`ellipsis`, `inert`, `focusable`, `detached`, `multiline`, `scroll-cross`,
`virtual`, and `sticky`.

Base attribute runs are sorted by attribute id and contain authored values
only. Layout defaults and inherited text style remain the kernel's job.
`when` patch attribute runs are likewise sorted; patches for one node retain
document order and the last active patch wins. Patch children are detached
nodes spliced after the base child chain while their condition holds.

Condition kinds are `State`, `Env`, `Client`, `WCmp`, `HCmp`, `Prop`, and
`Theme`. Width and height comparisons use the incoming constraint, not the
resolved size. `Each` templates remain detached and carry symbolic `PropRef`
values and property conditions until the kernel materializes an item.

Path verbs are `M=0`, `L=1`, `C=2`, `Q=3`, and `Z=4`; path coordinates are
already absolute and normalized by the compiler. Gradient kinds are
`linear=0`, `radial=1`, and `conic=2`; `grad_angle` carries the linear
angle or the conic from-angle (0 = up, clockwise) and is unused for
radial. Gradient stops are in ramp order. Animation stops are sorted by
position, and durations and delays are milliseconds.

Parameter types are `Text=0`, `Num=1`, `Pct=2`, `Color=3`, `Bool=4`,
`Enum=5`, and `List=6`. A `list_field_sub` entry is zero for a scalar field
and one plus the referenced list-schema row for a `List` field. Sub-schema
rows use `list_param=0xFFFF_FFFF` because hosts address them through a root
list parameter and an item path. List defaults are normalized before reaching
the kernel.
Omitting the entire `list_field_sub` array defaults every existing field to
scalar for backward-compatible 2.0 decoding.

## Font tables and runtime faces

A `font_*` row supplies layout metrics only:

```text
family, class, weight, upem, ascent, descent, line_gap, default_advance,
cmap offset/length, sorted codepoint pool, glyph-id pool, advance pool
```

It never contains TTF or OTF bytes. `family` is the authored `family=` string
verbatim; string reference 0 means the generic default. `class` is the
fallback metric class (`0` sans, `1` mono), and `weight` is one of the snapped
fallback weights 400, 500, 600, or 700. Cmap and advance entries are parallel,
sorted by codepoint, and cover document-reachable text plus printable ASCII.

The compiler emits a table for each authored family and snapped weight used by
the document, including the implicit default family and weight 400. Its
fallback metrics come from Inter for sans and JetBrains Mono for mono; a family
whose ASCII-case-folded name contains `mono` selects mono fallback metrics.
Weights snap to the nearest of 400/500/600/700, with ties rounding up.

Runtimes may register a face by `(family name, metrics, font bytes)`. The
kernel appends its metric table, matches family names ASCII-case-insensitively,
and selects the nearest weight; later equal candidates win, so a registered
face overrides the equal compiled fallback. If no matching family exists, it
falls back to the generic family table. A registered face that is far from all
compiled weights still supplies the nearest match.

Hosts use the matching registered bytes to paint text. Without a matching
registered face, native and static exporters use the bundled class fallback;
web uses its registered `FontFace` or its platform fallback. Thus SLIR is
self-contained for layout metrics but deliberately not for font bytes.

## Icons

`icon_name`, `icon_node`, and `icon_viewbox` are parallel arrays. Each node is
the root of a detached icon subtree. A missing `icon_viewbox` array defaults
each declared icon to a square 24-unit design box.

## Images

`img_src`, `img_w`, `img_h`, `img_format`, and `img_data` are parallel image
arrays. `img_data[i]` is the bytes for image `i`; an empty value means the
asset was not embedded or was unavailable at compile time. Format 0 is PNG.
Image entries are deduplicated by source string. `--no-embed-assets` affects
these image payloads only; it has no font-embedding meaning.

The compiler-side `Slir` value may stage image data in a contiguous private
buffer for exporters. That is not a SLIR wire `BLOB` and must not be used by a
wire decoder.

## Attribute ids

The attribute id mapping is shared by `crates/slab-slir/src/attrs.rs` and
`slab_kernel::slir`:

```text
 0 w             1 h             2 min-w         3 max-w         4 min-h
 5 max-h         6 pad           7 gap           8 axis          9 pack
10 align        11 self         12 offset       13 at           14 anchor
15 bg           16 stroke       17 stroke-w     18 stroke-align 19 stroke-sides
20 stroke-dash  21 radius       22 shadow       23 blur         24 backdrop
25 opacity      26 color        27 family       28 size         29 weight
30 leading      31 tracking     32 rotate       33 align-text   34 fit
35 src          36 d            37 cols         38 span         39 content
40 flags        41 act          42 field        43 each         44 keys
45 scrollbar    46 scrollbar-w  47 scrollbar-fg 48 scrollbar-bg 49 submit
50 item-extent  51 overscan     52 attach       53 gravity      54 collide
55 press        56 context      57 dblclick     58 drag         59 drop
60 resize       61 role         62 label        63 desc
64 scale        65 smooth       66 grain        67 mask         68 backdrop-mask
69 tilt         70 pointer-move  71 pointer-up    72 drag-update  73 drag-end
74 checked      75 expanded      76 selected      77 active-descendant
78 controls     79 value-now     80 value-min     81 value-max     82 value-text
83 modal        84 live          85 live-atomic   86 level         87 pos-in-set
88 set-size      89 animate       90 strike
```

`style=`, `key=`, and `transition=` do not appear in attribute runs: styles
are folded at compile time, keys become `node_key`, and transitions use their
motion fields. Signal binders and `animate=` are registered in their static
signal or binding pools and also encode their selected name as a `Str` in the
corresponding base or `when` patch attribute channel. Attribute 89 is the
internal animation-binding channel. Attribute 90 is the authorable inherited
boolean `strike` text style. `each` is `Num(parameter index)` on an `Each` node.

`family` is `Str` carrying the authored family name. `src` is `Str` and has a
matching image row. The remaining AVAL forms follow the source-language rules:
size attributes use size values, text metrics use `Num`, colors and paints use
their corresponding color/paint tags, and symbolic Each fields use `PropRef`
in a type-compatible scalar position.

Accessibility attributes preserve authored scalar typing. `role` is
`EnumSym` for an identifier or `Str` for a quoted open role. `label`, `desc`,
`active-descendant`, `controls`, and `value-text` use `Str`, `ParamRef`, or
`PropRef`. `checked` and `live` use `EnumSym`/`Str` for named states and may
use compatible scalar refs; Boolean states use `Num(0|1)` or Bool refs.
Range, hierarchy, and set-position fields use `Num` or Num refs. Optional
absence is represented by no attribute entry, never a numeric zero sentinel.

## `slir-dump`

`slab dump FILE.slir` renders the decoded model into canonical, line-oriented
text for conformance goldens and debugging. Floats round half-even to three
decimals with trailing zeros trimmed, `-0` prints as `0`, and colors print as
`#rrggbb` or `#rrggbbaa`.

`FONT` lines print only family, fallback class, metric fields, and cmap triples
`codepoint:glyph:advance`; they never print font blob offsets. `IMGS` lines
continue to display `blob=@offset+length` because the decoded compiler model
stages image payloads in its private contiguous buffer. The final
`BLOB image-bytes=<n> fnv1a64=<hash>` line describes that in-memory image
staging buffer, not a wire section. Dump output is deterministic for a given
decoded document.
