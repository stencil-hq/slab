# tree-sitter-slab

Tree-sitter grammar for the [Slab](../spec/SPEC.md) design language (v1.0.0).

Slab is newline-sensitive: newlines and `;` separate statements, node headers
end at newline / `{` / `}`, and `\` at end of line continues a line. This
grammar expresses that without an external scanner — `\n` is a real token,
not an extra.

## Layout

- `grammar.js` — the grammar (pure JS, no `scanner.c`)
- `src/` — generated parser (committed; consumers compile `src/parser.c`)
- `queries/highlights.scm` — standard-capture highlighting
- `queries/indents.scm` — brace/paren indentation
- `test/corpus/` — corpus tests

## Development

```sh
bun install
bunx tree-sitter generate
bunx tree-sitter test
bunx tree-sitter parse ../examples/*.slab --quiet --stat
```

## Notable parse rules

- `#word` immediately after a node name is a `node_id`; in value position it
  is a `hex_color`. Disambiguated by lexical precedence per parser state.
- `reference` (`color.bg`) is a single token: dotted idents with no spaces.
- `color_function` (`oklch(72% 0.16 250)`) captures raw balanced args as
  scalar tokens.
- `fill` in value position is a `fill_size`, optionally `fill:2`.
- Keywords `tokens params def anim when slot fill export` and the flag set
  are anonymous tokens extracted from `identifier` via the `word` rule.
- 1.0 additions: `params { name type = default }` blocks (types
  `text num pct color bool enum(...)`) and the `export` flag between a
  def's `)` and its body. `hole`, `act=`, `field=`, and `param.name` refs
  parse with the existing node / attribute / reference rules.
