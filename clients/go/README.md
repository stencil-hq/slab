# Slab for Go

Go client for [Slab](../../README.md). The whole toolchain — compiler, kernel,
and the Slab Drive Protocol (SDP) session layer — ships as an embedded
WebAssembly module and runs in-process on [wazero](https://wazero.io). No cgo,
no external binary, no network.

There is one kernel. This client translates terminal input and paints the cells
the kernel returns; it never reimplements layout, hit testing, focus, or
editing.

## Install

```sh
go get github.com/stencil-hq/slab/clients/go
```

The module needs Go 1.24 or newer. Its only dependencies are `wazero` (the
WebAssembly runtime) and `golang.org/x/term` (raw mode for the driver).

## Runtime API — package `slab`

A `Runtime` compiles the embedded module once. Sessions created from it are
cheap and independent: each holds at most one document plus its kernel state.

```go
ctx := context.Background()

rt, err := slab.NewRuntime(ctx)
defer rt.Close(ctx)

sess, err := rt.NewSession(ctx)
defer sess.Close(ctx)

// Compile a document. Open takes inline source; OpenFile reads the file on the
// host, because the WebAssembly module has no filesystem.
err = sess.OpenFile(ctx, "examples/10-settings.slab")
err = sess.Open(ctx, `col pad=8 { text "hi" }`, "inline.slab")

// Install precompiled SLIR (from `slab build FILE -o OUT.slir`) and skip the
// compiler entirely. Generated Go modules embed their SLIR and use this.
err = sess.OpenSLIR(ctx, slirBytes, "10-settings.slir")

// Size the document for an 80x24 terminal: width = cols*8, height = rows*16.
err = sess.SetEnvCells(ctx, 80, 24, true /*dark*/, false /*coarse*/)
err = sess.SetEnv(ctx, slab.EnvSpec{Width: 800, Height: 600, Client: "gpu"})

// Render. Plain drops styling; Caret paints the focused editor caret.
cells, err := sess.Cells(ctx, slab.CellsOptions{Caret: true})
fmt.Print(cells.Text) // cells.Cols, cells.Rows, cells.Notes

// Input. Every input helper returns the effects of the dispatch.
effects, err := sess.Key(ctx, "Tab")
effects, err = sess.Key(ctx, "ArrowRight", slab.ModCtrl)
effects, err = sess.Text(ctx, "hello")
effects, err = sess.Paste(ctx, "pasted")
effects, err = sess.Pointer(ctx, "move", 120, 64)
effects, err = sess.Click(ctx, 120, 64, slab.WithButton(slab.ButtonLeft))
effects, err = sess.ClickKey(ctx, "#reset")
effects, err = sess.Wheel(ctx, 120, 64, 48)

for _, signal := range effects.Signals {
    fmt.Println(signal.Name, signal.Item, signal.Text, signal.Meta.Key)
}

// Motion, introspection, shutdown.
err = sess.Advance(ctx, 16.67)
info, err := sess.Info(ctx) // file, params, themes, holes, signals, env, clock
err = sess.Quit(ctx)
```

### Anything else in the protocol

`Request` is the generic escape hatch and reaches every SDP method. It returns
the raw `result` payload, or a `*slab.ProtocolError` carrying the SDP code and
message.

```go
raw, err := sess.Request(ctx, "param.set", map[string]any{"name": "title", "value": "Hi"})
raw, err = sess.Request(ctx, "scene.tree", nil)

var protoErr *slab.ProtocolError
if errors.As(err, &protoErr) && protoErr.Code == -32601 {
    // unknown method
}
```

A document that does not compile returns a `*slab.CompileError` carrying every
diagnostic; the previously loaded document keeps running.

### Concurrency

One WebAssembly store is not reentrant, so every call funnels through a runtime
mutex. Sessions of the same runtime are safe to use from several goroutines,
but their calls serialize. Use separate runtimes for real parallelism.

## Terminal driver — package `slabtui`

`slabtui.Run` is the Go analogue of the Rust `slab-tui` host: raw mode, the
alternate screen, SGR (mode 1006) mouse tracking, bracketed paste, `SIGWINCH`
resize handling, clock ticks through `clock.advance`, and repaints from
`render.cells` with truecolor styling and the caret.

```go
err := slabtui.Run(ctx, sess, slabtui.Options{
    Dark: true,
    FPS:  30,
    OnSignal: func(signal slab.Signal) error {
        log.Println("signal", signal.Name)
        return nil
    },
})
```

Ctrl+C ends the run and is never forwarded to the document. `Run` returns when
the user quits, the input stream ends, the context is cancelled, or `OnSignal`
returns an error.

The input translation is exported for hosts that own their own event loop:

- `slabtui.Decoder` turns raw terminal bytes into `slabtui.Event` values,
  buffering partial escape sequences across reads.
- `slabtui.Dispatcher` applies one decoded event to a session, keeping the
  consecutive-click history a terminal does not provide.

```go
var decoder slabtui.Decoder
var dispatcher slabtui.Dispatcher
for _, event := range decoder.Feed(chunk) {
    effects, dispatched, err := dispatcher.Dispatch(ctx, sess, event)
    _ = effects
    _ = dispatched
    _ = err
}
```

Key names, modifiers, and the cell-to-layout mapping match the Rust host
exactly: `x = col*8 + 4`, `y = row*16 + 8`, three rows of wheel travel per
notch, and `Tab`, `Enter`, `Backspace`, `Delete`, `Escape`, `Insert`, `Home`,
`End`, `PageUp`, `PageDown`, `ArrowLeft`, `ArrowRight`, `ArrowUp`, `ArrowDown`,
`F1`..`F24`, or the literal printable character.

## Example

```sh
go run ./example                                   # examples/10-settings.slab
go run ./example -file examples/00-player.slab -light
```

The program opens the document, drives it interactively, and prints every
signal it saw on exit.

## The embedded module

`slab/slab_abi.wasm.gz` is the gzipped `wasm32-unknown-unknown` build of
`crates/slab-abi`. Regenerate it with `cargo run -q -p xtask -- abi-wasm`; never
hand-edit it. The module exposes the C ABI exports (`slab_abi_version`, `slab_alloc`,
`slab_free`, `slab_session_new`, `slab_session_free`, `slab_session_quit`,
`slab_request`) and imports nothing at all, so no WASI is required.

## Tests

```sh
cd clients/go && go test ./...
```

The suite compiles real documents, renders real grids, and drives real
repository examples through the decoder and dispatcher.
