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
on `127.0.0.1:N`, accepts one connection at a time, and retains the session
across sequential connections. `protocol.quit` terminates either transport.
Binary payloads embedded in JSON use padded RFC 4648 base64. SVG and terminal
cell payloads remain UTF-8 strings.

A request has this form:

```json
{"id": 17, "method": "input.click", "params": {"key": "#toolbar/#save"}}
```

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
| `doc.reload` | none | Reloads the current path with `doc.load` semantics. |
| `doc.info` | none | File, declarations, themes, holes, signals, environment, and clock. |
| `env.get` | none | `{width,height,client,dark,coarse,theme}`. |
| `env.set` | any `env.get` fields | Atomically merges supplied fields; theme validation runs last. |
| `clock.get` | none | `{t}` in milliseconds. |
| `clock.advance` | `{ms:number}` | Requires finite `ms >= 0`; returns the new `{t}`. |
| `param.set` | `{name,value}` or `{sets:{...}}` | Validates the entire write atomically and returns `{ok:true}`. |
| `param.get` | `{name}` | Returns the live kernel `{value}`. |
| `field.set` | `{key,text}` | Returns `{ok:true,changed}`. |
| `field.get` | `{key}` | Returns committed edit text or resolved initial content. |
| `state.set` | `{name,on}` | Sets a global runtime state. |
| `state.node` | `{key,name,on}` | Sets state on one resolved node. |
| `focus.get` | none | `{focus,key,visible}`; `slir::NONE` represents no focus. |
| `focus.set` | `{key,visible?}` | Moves focus; `visible` defaults to true. |

A successful standalone load preserves the desired environment, registered
fonts, and valid named theme, but creates a fresh instance: params, lists,
states, focus, edits, scroll offsets, image registrations, and hole sizes reset.
A compile failure is returned as `{ok:false,diags}` and leaves the prior document
running. If a requested theme does not exist, the authored base theme is used
and `theme_reset:true` is returned.

### 5.2 Images, scrolling, lists, dividers, and holes

| Method | Parameters | Result / behavior |
|---|---|---|
| `img.register` | `{name,w,h,format:1,rgba:[u8...]}` or `{name,w,h,format:0,png_b64}` | Returns unified image index `{img}`. |
| `img.unregister` | `{name}` | Removes a runtime registration. |
| `img.info` | `{img}` | `{w,h,format,generation}`. |
| `img.data` | `{img}` | Base64 `{data,bytes}`. |
| `scroll.get` | `{key,axis:0|1}` | `{axis,off}`. |
| `scroll.set` | `{key,axis:0|1,off}` | Sets and returns the clamped offset. |
| `scroll.reveal` | `{key,margin}` | Minimally reveals a node through all scroll ancestors. |
| `list.get_len` | `{param,path}` | `{len}` for a typed list path. |
| `list.set_len` | `{param,path,n}` | Resizes one list. |
| `list.set_field` | `{param,path,index,field,kind,value}` | Sets one typed item field. |
| `list.set_key` | `{param,path,index,key}` | Sets one stable item key. |
| `list.reveal_item` | `{each,index,align}` | Reveals a virtual item; align is 0 nearest, 1 start, 2 center, 3 end. |
| `list.window` | `{each}` | Returns materialized half-open `{start,end}`. |
| `divider.get` | `{key}` | `{extent}`. |
| `divider.set` | `{key,extent}` | Sets the divider overlay. |
| `hole.list` | none | Visible hole geometry and clipping. |
| `hole.size` | `{name,w,h}` or `{hole,w,h}` | Records host-content size. |

List data paths such as `0.children` address values inside a typed list
parameter; they are not scene keys. `each` locators use the grammar in §4.

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
| `input.event` | one trace event | Dispatches a validated kernel event. |
| `input.pointer` | `{type:"move"|"down"|"up",x,y,button?,clicks?,mods?}` | Dispatches one pointer event. |
| `input.click` | `{key,...}` or `{x,y,...}` | Dispatches move, down, and up and merges their effects. |
| `input.wheel` | `{x,y,dx?,dy,mods?}` | Dispatches one wheel event. |
| `input.key` | `{key,mods?}` | Dispatches one key-down, subject to a mounted host callback. |
| `input.text` | `{text}` | Dispatches text input, subject to a mounted host callback. |
| `input.paste` | `{text}` | Dispatches paste input, subject to a mounted host callback. |
| `render.png` | `{scale?,path?}` | PNG bytes/data, dimensions, and notes. |
| `render.svg` | `{path?}` | UTF-8 SVG/data, byte count, and notes. |
| `render.cells` | `{plain?,path?}` | UTF-8 terminal cells, dimensions, and notes. |
| `render.apng` | `{dur?,fps?,scale?,path?}` | Deterministic APNG, frame count, and advanced clock. |

Modifiers are `shift`, `alt`, `ctrl`, and `meta`. Input success returns
`{effects,t}`. Effects contain repaint, ordered signals and metadata, changed
scroll offsets, caret and IME rectangles, cursor, and focus. A host-consumed
key/text result additionally contains `host_consumed:true` and is not dispatched
to the kernel.

### 5.4 Accessibility and diagnostics

A render `path` writes the payload and returns its path and byte count instead
of embedding data. `doc.load` diagnostics have ordered
`{level,code,msg,line,remedy?}` entries. Render `notes` and diagnostics embedded
in `frame.dump` are ordered deterministic runtime observations, including
layout degradation and missing-resource reports. Automation MUST inspect them;
hosts SHOULD expose them to developers rather than silently dropping them.

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

Host-mounted pumps deny `doc.load` and `doc.reload` by default. Replacing only
the kernel instance can otherwise leave generated typed-setter caches holding
values the fresh instance has never received. A denied request is an explicit
`-32000` error and leaves the live instance unchanged.

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

## 7. Compatibility and versioning

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
