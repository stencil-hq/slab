# slab-lang (Python)

Run Slab documents from Python. The distribution is `slab-lang`; the import
package is `slab`.

The package embeds `slab_abi.wasm.gz`, a zero-import WebAssembly module that
contains the Slab compiler, the kernel, and the Slab Drive Protocol (SDP)
session layer. Nothing else is compiled on your machine and no `slab` binary is
needed: `.slab` sources are parsed and compiled **on the fly**, in process, at
runtime.

## Install

```sh
uv pip install slab-lang
```

The only runtime dependency is [`wasmtime`](https://pypi.org/project/wasmtime/).

### Installing from a path

To consume a checkout instead of PyPI, `uv add /path/to/slab-lang/packages/pyslab`
works, but note a uv pitfall: uv rewrites the absolute path to a **relative**
one under `[tool.uv.sources]`, and on macOS (where `/tmp` and friends are
symlinks) the recorded `../..` chain can be wrong, which later breaks
`uv run --project <dir>` from any other directory with
`cannot normalize a relative path beyond the base directory`. Mitigation: after
`uv add`, edit `[tool.uv.sources]` back to the absolute path (uv accepts it and
never rewrites an existing entry), or run `uv add` with fully resolved paths
(`realpath`) for both the project and the dependency. Details and a minimal
repro: `docs/uv-path-dep-rewrite.md`.

## Quick start

```python
import slab

with slab.open_file("examples/10-settings.slab") as session:
    session.set_env_cells(cols=100, rows=32, dark=True)
    print(session.render_cells(plain=True).text)
    effects = session.click(key="#save")
    for signal in effects.signals:
        print(signal.name, signal.text)
```

`slab.open_source(source, name="<source>")` does the same for inline text:

```python
import slab

with slab.open_source('col pad=8 { text "hello" size=14 }') as session:
    session.set_env_cells(cols=40, rows=8)
    assert "hello" in session.render_cells(plain=True).text
```

A compile failure is data, not an exception. `Session.open` returns a
`LoadResult` whose `ok` is `False` and whose `diags` list carries the
diagnostics, and the previous document keeps running.

## Precompiled documents

When you would rather not ship the source, build SLIR ahead of time and install
it without running the compiler. `doc.open_slir` has the same result shape and
the same failure semantics as on-the-fly compilation.

```sh
slab build examples/10-settings.slab -o settings.slir
```

```python
import pathlib

import slab

with slab.open_slir(pathlib.Path("settings.slir").read_bytes()) as session:
    session.set_env_cells(cols=100, rows=32)
    print(session.render_cells(plain=True).text)
```

`Session.open_slir(slir, name="<slir>")` does the same on an existing session.

## Terminal driver

```sh
python -m slab examples/10-settings.slab --dark --fps 60 \
    --set title=Tonight --set compact=true
```

`slab.tui.run` is the same driver as an API. It puts the terminal into raw mode
with the alternate screen and SGR mouse reporting (mode 1006), forwards keys,
text, pointer, and wheel input to the kernel (counting consecutive presses so
`dblclick=` binders fire, like the Rust driver), repaints the cells the kernel
returns, advances the motion clock, and reports signals through an `on_signal`
callback. Ctrl+C quits.

`--set param=value` (repeatable) and `--theme NAME` override declared params
and the active theme before the first frame, with the same value typing as the
`slab-tui` CLI: text, num, pct, color, bool, and enum scalars are written as
plain strings (`--set done_pct=25%`), and a list param takes a JSON array
(`--set 'tasks=[{"key":"1","title":"write docs"}]'`). A rejected `--set` or
`--theme` exits with status 2.

A host that owns live state (a clock, timers) passes `on_tick`; the driver
calls it between frames so the host never has to fork the loop:

```python
import time

import slab
from slab import tui

def tick(session: slab.Session) -> None:
    session.set_param("clock", time.strftime("%H:%M:%S"))

with slab.open_file("clock.slab") as session:
    tui.run(session, on_tick=tick, tick_interval=1.0)
```

For hosts that do need their own loop, every piece the driver is built from is
public API: `Terminal` (raw-mode lifecycle), `Decoder` (bytes to events, with
click counting), `ClickTracker`, `paint` (grid repaint), `pointer_units`, and
`WHEEL_STEP`.

The kernel owns layout, hit testing, focus, and editing. This package only
translates terminal input and paints the cells that come back.

## Typed lists from Python

A list-driven host treats its typed `list(...)` params as projections of its
own model: rebuild the whole projection on every mutation and write it in one
atomic bulk `param.set`. `Session.set_list` is that write:

```python
session.set_list("tasks", [
    {"key": "1", "title": "write docs", "done": False},
    {"key": "2", "title": "ship 0.2", "done": True},
])
```

Each row maps item field names to values. The optional `"key"` entry is the
item's stable identity: the kernel diffs by key, so focus, scroll offsets,
hover, and the virtualized window survive a full re-projection. Omitted fields
keep the item type's declared defaults. The whole batch is validated before
the first mutation — a bad row (for example a duplicate key) rejects the write
and leaves the previous list untouched.

The same write is available through the generic form
`session.request("param.set", {"sets": {"tasks": rows, "count": "3 left"}})`,
which mixes scalar and list params in one atomic call.

For single-item updates there are the `list.*` methods from `spec/SDP.md`
§5.2, one of which has a typed helper:

```python
session.set_list_field("tasks", 0, "done", "bool", True)   # list.set_field
session.request("list.set_key", {"param": "tasks", "path": "", "index": 0, "key": "9"})
session.request("list.set_len", {"param": "tasks", "path": "", "n": 10})
session.request("list.window", {"each": "#list/tasks"})     # {"start": s, "end": e}
session.request("list.reveal_item", {"each": "#list/tasks", "index": 42, "align": 0})
```

`kind` in `set_list_field` names the field's declared type: `text`, `num`,
`pct`, `color`, `bool`, or `enum`.

## API

| Name | Purpose |
|---|---|
| `slab.Runtime` | Compiled WebAssembly module plus one instance; creates sessions |
| `slab.Session` | One live document: request, input, render, clock |
| `slab.open_source` / `slab.open_file` / `slab.open_slir` | Convenience constructors that own their runtime |
| `slab.ProtocolError` | An SDP `error` object, with `code` and `message` |
| `slab.Effects`, `slab.Signal`, `slab.SignalMeta` | Decoded result of every input method; `meta.key` is always the emitter node path, with optional `hit_key` (deepest pointer hit target) and `pressed_key` (keyboard activation key) |
| `slab.Cells` | `render.cells` output: `text`, `cols`, `rows`, `notes`, `lines` |
| `slab.DocInfo` | `doc.info` output: params, themes, holes, signals, env, clock |
| `slab.LoadResult` | `doc.open` / `doc.open_slir` / `doc.load` output: `ok` plus `diags` |
| `Session.set_list` / `Session.set_list_field` | Typed list writes: atomic bulk replace and one-field update |
| `Session.request` | Generic escape hatch for every other SDP method |

Every SDP method listed in `spec/SDP.md` is reachable through
`Session.request(method, params)`.

## Development

```sh
cd packages/pyslab
uv venv
uv pip install -e '.[dev]'
uv run pytest
```

`slab_abi.wasm.gz` is a generated artifact. Rebuild it from the repository root
with `cargo run -q -p xtask -- abi-wasm`; never edit it by hand.
