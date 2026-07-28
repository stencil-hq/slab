# Slab Drive Protocol (SDP)

Status: normative for SDP version 1.

The Slab Drive Protocol is the deterministic automation and inspection protocol
for a live Slab kernel instance. It can run as a standalone `slab drive`
session or be mounted by a host with `slab_drive::RequestPump`. Both modes use
the same kernel and request semantics.

## 1. Transport and framing

SDP is UTF-8 newline-delimited JSON (NDJSON). Each non-blank input line is one
request object and produces exactly one response line, in input order. A server
MUST NOT emit unsolicited protocol lines. Blank input lines are ignored.

The standalone command uses stdin/stdout by default. With `--port N`, it listens
on `127.0.0.1:N`, serves one connection at a time, and retains the session
across sequential connections. While a connection is being served, any
additional connection receives exactly one `-32000` error line with a null id
(`session busy: another client holds this SDP session`) and is then closed; a
client MUST NOT wait for the slot. `protocol.quit` terminates either transport.
Binary payloads embedded in JSON use padded RFC 4648 base64. SVG and terminal
cell payloads remain UTF-8 strings.

A request has this form:

```json
{"id": 17, "method": "input.click", "params": {"key": "#toolbar/#save"}}
```

The `key` value above is an id-rooted locator; §4 defines the full addressing
grammar.

- `method` MUST be a string.
- `params`, when present, MUST be an object. Omission is equivalent to `{}`.
- `id` is OPTIONAL and MAY be any JSON value. A response echoes a present id.

Success and failure responses are mutually exclusive:

```json
{"id":17,"result":{"effects":{},"t":0}}
{"id":17,"error":{"code":-32000,"message":"unknown key '#saev'; nearest: '#save' (col@0/#toolbar/#save)"}}
```

Malformed JSON uses a null id because the request id cannot be trusted.

## 2. Errors

| Code | Meaning |
|---:|---|
| `-32700` | Invalid JSON. |
| `-32600` | Invalid request envelope. |
| `-32601` | Unknown method. |
| `-32602` | Invalid or missing method parameters. |
| `-32000` | Document, address, parameter, theme, render, or filesystem failure. |

An address that has multiple canonical matches MUST fail rather than choosing by
iteration order. Its message MUST list deterministic canonical full-key
candidates. An unknown address MUST include deterministic nearest or suffix
suggestions when any useful candidate exists.

## 3. Determinism and time

The virtual motion clock starts at `0` milliseconds and advances only through
`clock.advance` and `render.apng`. A successful document load resets it to `0`.
Document-dependent requests solve a fresh retained frame before acting, so
input dispatch and queries observe the same scene. Parameter writes retain the
kernel's deferred-solve semantics: a transition begins at the first solve that
observes the write. A deterministic transition script uses
`render` → `clock.advance` → `render`.

Results preserve authored/scene ordering. Hosts MUST NOT substitute their own
layout, hit testing, focus, motion, or list-window calculations. Native and WASM
clients consume the same kernel behavior.

## 4. Scene and `each` addressing

A canonical full scene key is a slash-separated hierarchy. Each segment is, in
precedence order:

1. an explicit `key=` value;
2. an authored `#id`;
3. `kind@index` for an otherwise unkeyed node among same-kind siblings.

A component call contributes its own segment and its expanded definition root
is the following segment. For example, the actionable root of `Button#save`
may have the full key `col@0/#save/row@0`. The id is a stable locator even
though it is not itself a retained node.

Explicit key values and stable list item keys escape reserved bytes using
uppercase `%25`, `%2F`, and `%7E` for `%`, `/`, and `~` respectively.
A concrete list descendant has this shape:

```text
<each-full-key>~<escaped-item-key>/<template-relative-key>
```

Nested lists repeat that construction.

Every SDP `key` parameter accepts exactly these locator forms:

- an exact canonical full key returned by `scene.tree`;
- a unique bare authored id, with or without `#`;
- a unique authored suffix rooted at an id, such as `#toolbar/#save`.

Bare ids and id-rooted suffixes MUST be unique. For a component-call id, a
unique match resolves to the first actual node under the call segment: the
expanded definition root. Exact full keys always take precedence.

Id-rooted suffixes tunnel through `~item` markers: the suffix is matched
against the full canonical key of every retained node, including concrete list
descendants, so `#rows~t2/#task/#toggle` uniquely addresses one item's toggle
without spelling the whole path from the root. Unmaterialized virtual items
have no retained node and cannot be addressed this way; the exception is
`scroll.reveal`, which resolves a trailing `each~key` locator through list
data (§5.2).

Every SDP `each` parameter accepts the same canonical full-key, unique bare-id,
and unique id-rooted-suffix forms, but the resolved node MUST be an `each` node
valid for the requested operation. Thus a document whose canonical each key is
`col@0/#feed/rows` can be addressed as either that full key or the unique
locator `#feed/rows`; `rows` is not a canonical alias unless it is an authored
id.

Automation SHOULD retain full keys returned by `scene.tree` or generated key
constants. Human-authored probes MAY use unique ids and id-rooted suffixes.

## 5. Method surface

`protocol.info` returns `{name:"sdp", version:1, doc, methods}`.
`protocol.quit` returns `{ok:true}` and then ends the server.

### 5.1 Document, environment, clock, and parameters

| Method | Parameters | Result / behavior |
|---|---|---|
| `doc.load` | `{file:string}` | `{ok,diags,reloaded?,theme_reset?}`; successful loads replace all kernel state. |
| `doc.open` | `{source:string,name:string?}` | Compiles inline source with `doc.load` semantics; never reads the filesystem. |
| `doc.open_slir` | `{slir:base64,name:string?}` | Installs precompiled SLIR with `doc.load` semantics; skips the compiler. |
| `doc.reload` | none | Reloads the current path with `doc.load` semantics. |
| `doc.info` | none | File, declarations, themes, holes, signals, environment, and clock. |
| `doc.diags` | none | Cumulative `{diags:[{code,line,msg}...]}` runtime diagnostics; deduplicated, ordered by first occurrence, cleared only by a successful load. |
| `env.get` | none | `{width,height,client,dark,coarse,theme}`. |
| `env.set` | any `env.get` fields | Atomically merges supplied fields; theme validation runs last. |
| `clock.get` | none | `{t}` in milliseconds. |
| `clock.advance` | `{ms:number}` | Requires finite `ms >= 0`; returns the new `{t}`. |
| `param.set` | `{name,value}` or `{sets:{...}}` | Validates the entire write atomically and returns `{ok:true}`. |
| `param.get` | `{name}` | Returns the live kernel `{value}`. |
| `field.set` | `{key,text}` | Returns `{ok:true,changed}`; a key that is not a field is a `-32000` error. |
| `field.get` | `{key}` | Returns committed edit text or resolved initial content; same non-field error. |
| `field.caret.get` | `{key}` | Returns `{caret,anchor,goal_x,composing}`; `goal_x` is null when no vertical goal is active. |
| `field.caret.set` | `{key,caret,anchor,goal_x?}` | Sets a directed selection; null or omitted `goal_x` resets the goal. Returns `{ok:true,changed:true}`. |
| `field.runs.get` | `{key}` | Returns canonical rich runs as `{rev,runs:[{style,start,end}]}`. |
| `field.runs.set` | `{key,rev,runs:[{style,start,end}]}` | Atomically replaces rich runs and returns `{ok:true,changed}`. |
| `field.style.toggle` | `{key,style}` | Toggles style `0..4` over the current selection and returns `{ok:true,changed}`. |
| `field.range.get` | none | Returns `{range:{anchor:{key,offset},head:{key,offset}}}` or `{range:null}`. |
| `field.range.clear` | none | Clears cross-field metadata and returns `{ok:true,changed}`. |
| `state.set` | `{name,on}` | Sets a global runtime state. |
| `state.node` | `{key,name,on}` | Sets state on one resolved node. |
| `focus.get` | none | `{focus,key,visible}`; `slir::NONE` represents no focus. |
| `focus.set` | `{key,visible?}` | Moves focus; `visible` defaults to true. |

Caret offsets and rich-run endpoints are signed codepoint offsets clamped by
the kernel to grapheme boundaries. Caret direction is preserved: `caret` is
the active endpoint and `anchor` is fixed. A finite nonnegative `goal_x`
selects and retains the shaped stop used by vertical movement. `composing` is
query-only and reports active IME composition.

The rich-run wire schema is exactly the Change signal `runs` schema:
`{"rev":u64,"runs":[{"style":u32,"start":i32,"end":i32}]}`. Styles are
`0 bold | 1 italic | 2 underline | 3 strike | 4 code`; ranges are half-open.
The supplied revision is informational and a changed write increments the
field's local revision. A malformed run, caret, or style payload rejects only
that request with `-32602`; decoding is atomic. Unknown/non-field locators and
unavailable caret targets use `-32000`.

Range endpoints use canonical `FieldLocator` objects
`{"key":string,"offset":i32}`. Keys include escaped stable list-item identity,
matching `inst_get_range` in FRAME.md.

A successful standalone load preserves the desired environment, registered
fonts, and valid named theme, but creates a fresh instance: params, lists,
states, focus, edits, scroll offsets, image registrations, and hole sizes reset.
A compile failure is returned as `{ok:false,diags}` and leaves the prior document
running. If a requested theme does not exist, the authored base theme is used
and `theme_reset:true` is returned.

A `pct` parameter accepts the number `60` or the string `"60%"` on write.
`param.get` always returns the bare number (`60`), so the numeric spelling is
the canonical round-trip form and the `%` string is write-side convenience
only. `pct` is the generic parent-relative percentage type and is deliberately
unclamped — `150%` is a legitimate sizing value. A host that projects a pct
param with progress-bar semantics MUST clamp in its own model; the kernel does
not.

`doc.open` is the load path for embedders without a filesystem, such as a
WebAssembly host that reads `.slab` text itself and passes it in. `name` only
labels diagnostics and `doc.info` and defaults to `<source>`; it is never
opened. Image `src` values resolve against an empty in-memory asset map instead
of host storage.

`doc.open_slir` is the load path for generated modules that embed lowered bytes
at build time. It decodes the SLIR document and installs it directly, so no
compilation runs and there are no compile diagnostics; malformed bytes return
`{ok:false,diags}` with a single `decode` diagnostic. `name` defaults to
`<slir>` and labels diagnostics and `doc.info` exactly as for `doc.open`.
Both methods return the same result shape as `doc.load` and appear in
`protocol.info.methods`.

### 5.2 Images, scrolling, lists, dividers, and holes

| Method | Parameters | Result / behavior |
|---|---|---|
| `img.register` | `{name,w,h,format:1,rgba:[u8...]}` or `{name,w,h,format:0,png_b64}` | Returns unified image index `{img}`. |
| `img.unregister` | `{name}` | Removes a runtime registration. |
| `img.info` | `{img}` | `{w,h,format,generation}`. |
| `img.data` | `{img}` | Base64 `{data,bytes}`. |
| `scroll.get` | `{key,axis:0|1}` | `{axis,off}`. |
| `scroll.set` | `{key,axis:0|1,off}` | Sets and returns the clamped offset. |
| `scroll.reveal` | `{key,margin}` | Minimally reveals a node through all scroll ancestors; a trailing `each~key` locator resolves through list data even when the item is unmaterialized. |
| `list.get_len` | `{param,path}` | `{len}` for a typed list path. |
| `list.set_len` | `{param,path,n}` | Resizes one list. |
| `list.set_field` | `{param,path,index,field,kind,value}` | Sets one typed item field. |
| `list.set_key` | `{param,path,index,key}` | Sets one stable item key. |
| `list.reveal_item` | `{each,index,align}` | Reveals a virtual item; align is 0 start, 1 center, 2 end, 3 nearest. |
| `list.window` | `{each}` | Returns materialized half-open `{start,end}`. |
| `divider.get` | `{key}` | `{extent}`; `-1` is the unset sentinel: the divider still sits at its authored position and no overlay extent has been recorded. |
| `divider.set` | `{key,extent}` | Sets the divider overlay. |
| `hole.list` | none | Visible hole geometry and clipping. |
| `hole.size` | `{name,w,h}` or `{hole,w,h}` | Records host-content size. |

List data paths such as `0.children` address values inside a typed list
parameter; they are not scene keys. `each` locators use the grammar in §4.

`list.reveal_item` alignment values are the kernel's `inst_reveal_item` enum
(`0 start | 1 center | 2 end | 3 nearest`), stated normatively in SPEC.md
§15.5; `skill/references/hosts.md` documents the same mapping for embedded
hosts. A keyed `scroll.reveal` on a virtual item uses nearest alignment.
Reveals are sticky-aware: a target parks below a pinned sticky header and
centering happens in the uncovered region (SPEC.md §15.5 is normative).

### 5.3 Scene, frame, input, and rendering

| Method | Parameters | Result / behavior |
|---|---|---|
| `scene.tree` | none | Flat pre-order retained entries with full keys, hierarchy, geometry, flags, scroll, and resolved accessibility semantics. |
| `scene.node` | `{key,states?}` | One entry plus hover, pressed, focus, disabled, and requested runtime states. |
| `scene.text` | `{key}` | Concatenated subtree text and positioned runs. |
| `scene.hit` | `{x,y}` | Root-to-target keys, nodes, and rectangles. |
| `scene.find` | `{text}` | Case-sensitive scene-ordered text matches. |
| `frame.dump` | none | Canonical conformance frame JSON. |
| `frame.summary` | none | Canonical focus, edit, and scroll summary. |
| `input.event` | `{type,...,text?,clauses?}` trace event | Dispatches a validated kernel event; composition clauses use optional `[[start,end],...]` codepoint pairs. |
| `input.pointer` | `{type:"move"|"down"|"up",x,y,button?,clicks?,mods?}` | Dispatches one pointer event. |
| `input.click` | `{key,...}` or `{x,y,...}` | Dispatches move, down, and up and merges their effects. |
| `input.wheel` | `{x,y,dx?,dy,mods?}` | Dispatches one wheel event. |
| `input.key` | `{key,mods?}` | Dispatches one key-down, subject to a mounted host callback. |
| `input.text` | `{text}` | Dispatches text input, subject to a mounted host callback. |
| `input.paste` | `{text}` | Dispatches paste input, subject to a mounted host callback. |
| `render.png` | `{scale?,path?}` | PNG bytes/data, dimensions, and notes. |
| `render.svg` | `{path?}` | UTF-8 SVG/data, byte count, and notes. |
| `render.cells` | `{plain?,caret?,path?}` | UTF-8 terminal cells, dimensions, and notes. |
| `render.apng` | `{dur?,fps?,scale?,path?}` | Deterministic APNG, frame count, and advanced clock. |

For `type:"composition-update"`, `text` is the preedit and optional `clauses`
is an ordered JSON array of two-element signed-integer codepoint ranges:
`[[start,end],...]`. Omission, an empty array, or any malformed clause payload
degrades to no metadata, which FRAME.md defines as one whole-preedit clause;
malformed clauses do not reject the SDP request.

Modifiers are `shift`, `alt`, `ctrl`, and `meta`. Input success returns
`{effects,t}`. Effects contain repaint, ordered signals and metadata, changed
scroll offsets, caret and IME rectangles, cursor, and focus. A rich-field
Change signal additionally contains `runs` in the exact `{rev,runs}` schema
defined above. A deferred cross-field edit additionally contains
`range_edit:{kind,anchor,head,text}` with the canonical endpoint objects
defined above; kind values are `0 text | 1 paste | 2 cut | 3 Backspace |
4 Delete | 5 composition | 6 copy`. Signal metadata
`key` is always the emitter node path; pointer-derived signals additionally
carry `hit_key` (the deepest hit-target canonical key) and keyboard-driven
activations carry `pressed_key` (the key name). Both fields are omitted when
absent; `spec/FRAME.md` is normative for their composition. A host-consumed
key/text result additionally contains `host_consumed:true` and is not
dispatched to the kernel.

`input.key` dispatches key-downs only: a printable key with focus inside an
editing field neither inserts text (text arrives exclusively through
`input.text`) nor falls through to `keys=` shortcut maps, because the focused
editor claims printable keys first. Automation that wants typing sends
`input.text`; automation that wants a shortcut ensures focus is not composing
in a field.

`render.cells` accepts `caret`, defaulting to false. When true, the returned
grid carries the kernel caret overlaid on the cell it occupies, which is what a
live terminal client paints.

### 5.4 Accessibility and diagnostics

A render `path` writes the payload and returns its path and byte count instead
of embedding data. A relative `path` resolves against the SDP server's working
directory, not the client's. `doc.load` diagnostics have ordered
`{level,code,msg,line,remedy?}` entries. Render `notes` and diagnostics embedded
in `frame.dump` are ordered deterministic runtime observations, including
layout degradation and missing-resource reports; each is reported once per
solve, so an intermediate solve consumes that solve's stream. `doc.diags`
(§5.1) is the cumulative alternative: it retains every distinct runtime
diagnostic since the current document loaded and can be queried at any time.
Automation MUST inspect diagnostics; hosts SHOULD expose them to developers
rather than silently dropping them.

`scene.tree` and `scene.node` expose the resolved accessibility contract:
role, label, description, disabled/focused state, checked, expanded, selected,
active-descendant and controls relationships, range/value text, modal/live
semantics, level, and set position/size. Browser semantic DOM and native
AccessKit bridges MUST project these fields and MUST NOT independently infer a
different semantic tree.

## 6. Host-mounted sessions

`RequestPump::new` keeps ownership of the live `Instance` with the host and
borrows it for one request. `RequestPump::request` is the simple kernel-only
path. A host with terminal or window keyboard shortcuts uses
`request_with_host_input`; its callback observes `PumpHostEvent::Key` and
`PumpHostEvent::Text` before kernel dispatch and returns `Dispatch` or
`Consumed`. Consumed input returns `host_consumed:true`.

Host-only shortcuts are less portable than authored Slab signals. Every
automation-critical shortcut SHOULD also have a signal-bound UI affordance (for
example, a visible timer skip button) so standalone, browser, native, and TUI
automation can drive the same behavior.

### 6.1 Load and reload policy

Host-mounted pumps deny `doc.load`, `doc.open`, `doc.open_slir`, and
`doc.reload` by default. Replacing only the kernel instance can otherwise leave
generated typed-setter caches holding values the fresh instance has never
received. A denied request is an explicit `-32000` error and leaves the live
instance unchanged.

A host MAY opt in with `ReloadPolicy::Allow`. On every successful load or reload,
`PumpResponse.reloaded` is true and the wire result contains `reloaded:true`.
Before re-synchronizing its model, the host MUST call the generated
`Doc::invalidate_caches()`. That method clears every generated list
reconciliation cache, is idempotent, and does not mutate the kernel. The host
then reapplies all host-owned state through generated setters. Failed loads do
not set `reloaded` and retain both instance and caches.

### 6.2 Host-owned parameters

A mounted host commonly treats params and typed lists as projections of its own
model. A direct SDP `param.set` is transient when the host's next synchronization
writes that model again. Automation of such sessions SHOULD drive authored
signals, key/text input, and visible controls instead of writing host-owned
params. `param.set` remains appropriate for standalone sessions and params the
host explicitly delegates to SDP.

The host processes each `PumpResponse.effects` through the same ordered signal
handler used for local input. It MUST NOT run a second solver or bypass the
shared kernel to emulate SDP behavior.

### 6.3 Window-mounted viewer (`slab-native --port`)

`slab-native FILE.slab --port N` mounts the viewer's live window kernel as an
SDP session on `127.0.0.1:N` (`--port 0` picks a free port and prints it).
Requests are drained on the window's event loop through a `RequestPump`, so
automation drives exactly the instance the window paints: every mutating
request repaints the window, and window input (pointer, keyboard, IME) and SDP
requests interleave on one kernel. Framing, ordering, and the one-client
`session busy` rule are identical to `slab drive --port` (§1).

Divergences from a standalone `slab drive` session:

- The document is compiled from `FILE.slab` at startup; `doc.load`, `doc.open`,
  `doc.open_slir`, and `doc.reload` are denied per §6.1 (the window's renderer
  registers image and font resources once).
- The environment (viewport, scale, dark) tracks the real window; `env.set`
  cannot resize the OS window.
- Signals dispatched by SDP requests print to the viewer's stdout exactly like
  window-originated signals.
- `protocol.quit` closes the window and the process exits with status 0.

## 7. In-process embedding

A host that can run WebAssembly MAY speak SDP without a transport. The
`slab-abi` module (`crates/slab-abi`, built for `wasm32-unknown-unknown` with no
imports) exposes the whole protocol through these exports:

| Export | Returns | Meaning |
|---|---|---|
| `slab_abi_version()` | `u32` | ABI revision; version 1 hosts MUST refuse any other value. |
| `slab_alloc(len)` | pointer | 4-byte aligned block, `0` when unavailable. |
| `slab_free(ptr,len)` | none | Releases a block from `slab_alloc` or `slab_request`. |
| `slab_session_new()` | `u32` | Nonzero handle for a session with no document loaded. |
| `slab_session_free(handle)` | none | Destroys a session and its kernel state. |
| `slab_session_quit(handle)` | `u32` | `1` after `protocol.quit`, otherwise `0`. |
| `slab_request(handle,ptr,len)` | pointer | Length-prefixed response block. |

A request body is one NDJSON line passed as a `(ptr,len)` pair the host owns.
`slab_request` answers with a block holding a little-endian `u32` byte count
followed by that many UTF-8 bytes: exactly one JSON response object, without a
trailing newline, for protocol failures and unknown handles alike. The host
releases that block with `slab_free(ptr, 4 + n)`. Handles are opaque nonzero
`u32` values. A new session starts with the default 800x600 `gpu` environment,
so a terminal host sends `env.set` with `client: "tui"` and its cell-derived
pixel size before rendering.

Request envelopes, errors, determinism, addressing, and method semantics are
those of §1 through §6, and §8 applies unchanged: a host driving this module is
protocol-equivalent to a stdio or TCP client. Because the module has no
filesystem, such a host loads documents with `doc.open` or `doc.open_slir`
rather than `doc.load`.
The module documentation of `crates/slab-abi/src/lib.rs` is the detailed
reference for the calling convention.

## 8. Compatibility and versioning

`protocol.info.version` is the integer protocol major. Version 1 clients MUST
ignore unknown object fields and methods they do not call. Servers MAY add
methods, optional request fields, result fields, diagnostics, and error detail
without changing the major. They MUST preserve framing, ordering, existing field
types, and method semantics within a major.

Removing or renaming a method, changing required parameters or result types,
changing canonical key grammar, or changing deterministic dispatch semantics
requires a new major. Clients SHOULD call `protocol.info` when compatibility is
uncertain and MUST treat an unsupported method as `-32601`, not infer support
from package version alone.
