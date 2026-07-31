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

Fresh releases: bun's minimum-release-age gate blocks packages younger than
24h (`… blocked by minimum-release-age: 86400 seconds`). Add `[install]
minimumReleaseAge = 0` to a project `bunfig.toml`. Note `bunx` ignores a cwd
bunfig; use `bun add @stencil-hq/slab` and run `./node_modules/.bin/slab`.

## Commands

```
slab check FILE                              print diagnostics (exit 1 on errors)
slab build FILE -o OUT.slir                  compile to SLIR
slab dump FILE.slir                          print the canonical slir-dump text
slab fmt FILE... [--check]                   canonical formatting ('-' filters stdin to stdout)
slab render FILE -o OUT.{svg,png,apng,txt} [--theme NAME]   static export
slab gen wc FILE -o DIR [--tag NAME]         emit a web component and its runtime/WASM sidecars
slab gen react FILE -o DIR [--tag NAME]      emit a web component plus a typed React wrapper
slab dev FILE [-o DIR] [--tag NAME] [--host HOST] [--port N]
                                                serve a live web-component preview
slab gen rust FILE -o OUT.rs                 emit a typed Rust module
slab drive                                    requires the native slab-cli
slab --version                                package + compiler version and git hash
```

`render` infers the output kind from the extension; `--client tui` with no
`-o` writes cells to stdout. See `slab render --help` for the full flag set.
`slab check` prints both the embedded compiler version and this package
version before diagnostics, making a stale global install or lockfile visible.
`--state` previews document-global states only; it does not target one node.

`gen wc` and `gen react` emit `.slir` files that generated JavaScript fetches
at runtime. `gen rust` emits `OUT.slir` beside `OUT.rs`; the module uses
`include_bytes!` to include that sidecar, so keep the pair together in source.

Interactive/drive commands (`slab drive`, the SDP server used by
`@stencil-hq/dslab`) live in the native CLI only: install with
`cargo install --git https://github.com/stencil-hq/slab slab-cli`.

## Live development

```sh
slab dev doc.slab --port 3000
```

`dev` generates the same web component as `gen wc`, serves a built-in preview,
and watches the source directory recursively. The default output directory is
`doc.dev`, and the default address is `http://127.0.0.1:3000/`. Use `--port 0`
to select an available port.

After a valid edit, `dev` rebuilds and reloads connected previews. A compile
failure keeps the last valid component available and shows the latest
diagnostics in the preview. The server recovers after the next valid edit.

## Use in a page

```sh
bunx @stencil-hq/slab gen wc doc.slab -o dist --tag my-doc
```

```html
<script type="module" src="./dist/doc.js"></script>
<my-doc style="display:block;width:800px;height:600px"></my-doc>
```

The generated module registers the custom element and is emitted alongside its
`.slir` document, the shared web runtime, and the kernel WASM sidecar. Keep
those files together when deploying the output. Set attributes/properties for
`CustomEvent`s for signals (see the `signals` export).

## Library use

- `@stencil-hq/slab` — this compiler, renderer, and code-generation CLI, with
  its compiler WASM bundled for Node or Bun.
- `@stencil-hq/wslab` — the `SlabElement` base class, DOM `Painter`, web
  runtime, and Rust-kernel WASM glue/sidecar used by generated components.

## Bundler plugins

Vite and Bun plugins ship as subpath exports, letting you import `.slab`
files directly:

```ts
import './hero.slab'; // defines <slab-hero> (and one element per exported def)
```

Each import compiles through the bundled WASM compiler into a web-component
ES module whose runtime import points at `@stencil-hq/wslab` — add that
package to your app:

```sh
bun add -d @stencil-hq/slab
bun add @stencil-hq/wslab   # runtime dependency of the generated modules
```

### Vite

```ts
// vite.config.ts
import { defineConfig } from 'vite';
import slab from '@stencil-hq/slab/vite';

export default defineConfig({
   plugins: [slab()],
});
```

Options: `declarations` (default `true`) — write a sibling `<name>.d.slab.ts`
declaration next to each imported `.slab` (skipped when byte-identical). The
`.slab` source and every image asset it references are registered as watch
dependencies.

### Bun

```ts
// preload.ts — register for the Bun runtime / test runner
import slab from '@stencil-hq/slab/bun';

Bun.plugin(slab());
```

```ts
// or in a Bun.build pipeline
import slab from '@stencil-hq/slab/bun';

await Bun.build({ entrypoints: ['./app.ts'], plugins: [slab()] });
```

Bun's dev server does a full page reload on change; the HMR byte-swap below
is Vite-only.

### HMR semantics (Vite dev server)

The generated module guards its registration
(`if (!customElements.get(tag)) customElements.define(tag, Cls)`), and the
custom-element registry can never re-register a tag. Re-importing an edited
module therefore keeps the OLD class registered.

Instead of pretending otherwise, the module gets a self-accepting footer that
swaps document bytes through the stable registered class:

1. fetch the updated module's external SLIR,
2. `Registered.hotReplaceSlir(bytes)` — future mounts of the tag decode the
   new document,
3. `el.loadSlir(bytes)` on every mounted instance — live elements re-mount
   in place.

What survives a hot swap: element attributes (they live on the DOM element
and are re-applied), params, lists, scroll/divider offsets, runtime images,
and env. What resets: focus, edit state, and any imperative state your code
set on the old kernel instance. A full reload is still the only way to
re-evaluate your own module code.

### Typed imports

Bundler transforms are invisible to `tsc`, so the plugins write a sibling
declaration file per import: `hero.slab` → `hero.d.slab.ts`, containing the
element classes and the `signals` export. TypeScript 5 picks these up when
you enable:

```jsonc
// tsconfig.json
{
   "compilerOptions": {
      "allowArbitraryExtensions": true
   }
}
```

Commit the generated `.d.slab.ts` files or add them to `.gitignore` — either
works; they regenerate on the next dev-server or build run.

### Kernel WASM sidecar

`@stencil-hq/wslab` loads the Rust kernel via
`new URL('./wasm/slab_kernel_bg.wasm', import.meta.url)`. Vite understands
this pattern and copies the sidecar into the bundle. If your bundler breaks
`import.meta.url` resolution, copy `node_modules/@stencil-hq/wslab/wasm/`
next to your bundle output (or serve it under `/wasm/`); the element reports
the exact failing URL in an inline error if it cannot load.

### Plugin limitations

- HMR swaps document bytes, not code: edits to your own TS/JS still follow
  normal module HMR/reload rules.
- Bun has no partial reload for `.slab` edits — the dev server fully reloads.
- The declaration writer touches your source tree (sibling `.d.slab.ts`
  files); pass `{ declarations: false }` if you'd rather not.
- Compile errors throw with the compiler's formatted diagnostics and fail the
  build/import; warnings pass through as bundler warnings (Vite) or silently
  (Bun runtime).
