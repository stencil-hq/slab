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
python -m slab examples/10-settings.slab --dark --fps 60
```

`slab.tui.run` is the same driver as an API. It puts the terminal into raw mode
with the alternate screen and SGR mouse reporting (mode 1006), forwards keys,
text, pointer, and wheel input to the kernel, repaints the cells the kernel
returns, advances the motion clock, and reports signals through an `on_signal`
callback. Ctrl+C quits.

The kernel owns layout, hit testing, focus, and editing. This package only
translates terminal input and paints the cells that come back.

## API

| Name | Purpose |
|---|---|
| `slab.Runtime` | Compiled WebAssembly module plus one instance; creates sessions |
| `slab.Session` | One live document: request, input, render, clock |
| `slab.open_source` / `slab.open_file` / `slab.open_slir` | Convenience constructors that own their runtime |
| `slab.ProtocolError` | An SDP `error` object, with `code` and `message` |
| `slab.Effects`, `slab.Signal`, `slab.SignalMeta` | Decoded result of every input method |
| `slab.Cells` | `render.cells` output: `text`, `cols`, `rows`, `notes` |
| `slab.EnvSpec` | `env.set` payload; terminal sizing is `cols * 8` by `rows * 16` |
| `slab.DocInfo` | `doc.info` output: params, themes, holes, signals, env, clock |
| `slab.LoadResult` | `doc.open` / `doc.open_slir` / `doc.load` output: `ok` plus `diags` |
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
