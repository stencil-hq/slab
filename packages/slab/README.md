# @stencil-hq/slab

The slab compiler, packaged as an npm CLI with zero Rust on the host. Its
compiler WASM is bundled inside this package. Browser components use the other
npm package, `@stencil-hq/wslab`, which contains the web runtime together with
the Rust kernel's WASM binding glue and external sidecar.

## Install

```sh
bunx @stencil-hq/slab render doc.slab -o out.png
```

No Rust toolchain, no bun required at run time — the wasm module is bundled
inside. Node ≥ 20 (or any recent bun).

## Commands

```
slab check FILE                              print diagnostics (exit 1 on errors)
slab build FILE -o OUT.slir                  compile to SLIR
slab dump FILE.slir                          print the canonical slir-dump text
slab render FILE -o OUT.{svg,png,apng,txt}   static export
slab gen wc FILE -o DIR [--tag NAME]         emit a web component and its runtime/WASM sidecars
slab gen rust FILE -o OUT.rs                 emit a typed Rust module
```

`render` infers the output kind from the extension; `--client tui` with no
`-o` writes cells to stdout. See `slab render --help` for the full flag set.

## Use in a page

```sh
bunx @stencil-hq/slab gen wc doc.slab -o dist --tag my-doc
```

```html
<script type="module" src="./dist/doc.js"></script>
<my-doc style="display:block;width:800px;height:600px"></my-doc>
```

The generated module registers the custom element and is emitted alongside the
shared web runtime and kernel WASM sidecar. Keep those files together when
deploying the output. Set attributes/properties for params and listen on
`CustomEvent`s for signals (see the `signals` export).

## Library use

- `@stencil-hq/slab` — this compiler, renderer, and code-generation CLI, with
  its compiler WASM bundled for Node or Bun.
- `@stencil-hq/wslab` — the `SlabElement` base class, DOM `Painter`, web
  runtime, and Rust-kernel WASM glue/sidecar used by generated components.
