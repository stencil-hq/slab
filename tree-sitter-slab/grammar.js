/**
 * Tree-sitter grammar for the Slab design language.
 *
 * Newlines are significant statement separators, so `\n` is NOT an extra;
 * only spaces/tabs/CR, comments, and `\`-line-continuations are skipped.
 * `;` is equivalent to a newline. Node headers end at newline, `{`, or `}`.
 *
 * The grammar models the complete authored surface, including typed/nested
 * lists, icon declarations, nested `each`, and the general attribute syntax.
 * Placement and type restrictions remain semantic compiler diagnostics.
 */

// Identifier: starts [A-Za-z_], continues [A-Za-z0-9_], `-` only when the
// next char is [A-Za-z_] (lookahead-free equivalent of the lexer regex).
const IDENT = /[A-Za-z_][A-Za-z0-9_]*(-[A-Za-z_][A-Za-z0-9_]*)*/;

// Flags are a closed lexical vocabulary; placement remains semantic.
const FLAGS = [
  'clip',
  'bleed',
  'scroll',
  'nowrap',
  'ellipsis',
  'inert',
  'focusable',
  'multiline',
  'sticky',
  'virtual',
  'drag-ghost',
  'escape-blur',
  'strike',
];

module.exports = grammar({
  name: 'slab',

  word: $ => $.identifier,

  extras: $ => [
    /[ \t\r]/,
    $.comment,
    token(/\\\r?\n/), // line continuation: `\` swallows the newline
  ],

  rules: {
    // document := stmt*  (statements separated by newlines / `;`)
    source_file: $ => repeat(choice($._statement, $._newline)),

    _statement: $ => choice(
      $.tokens_declaration,
      $.theme_declaration,
      $.params_declaration,
      $.def_declaration,
      $.icon_declaration,
      $.anim_declaration,
      $.when_top,
      $.node,
    ),

    // -- tokens ------------------------------------------------------------

    // tokens := "tokens" "{" entry* "}"
    tokens_declaration: $ => seq(
      'tokens',
      repeat($._newline),
      '{',
      repeat($._token_item),
      '}',
    ),

    // theme := "theme" IDENT "{" entry* "}"
    theme_declaration: $ => seq(
      'theme',
      field('name', $.identifier),
      repeat($._newline),
      '{',
      repeat($._token_item),
      '}',
    ),

    _token_item: $ => choice($.token_group, $.token_entry, $._newline),

    // Nested token group: `color { bg #0e1116; ... }` — `{` on the same line.
    token_group: $ => seq(
      field('name', $.identifier),
      '{',
      repeat($._token_item),
      '}',
    ),

    // Leaf token entry: `ink #e6edf3`, `soft 0,18,44,#00000073`.
    token_entry: $ => seq(
      field('name', $.identifier),
      field('value', $._value),
    ),

    // -- params (1.0) --------------------------------------------------------

    // params := "params" "{" (IDENT type "=" scalar)* "}"
    params_declaration: $ => seq(
      'params',
      repeat($._newline),
      '{',
      repeat(choice($.param_declaration, $._newline)),
      '}',
    ),

    // paramdecl := IDENT type "=" (scalar | list_literal)
    param_declaration: $ => seq(
      field('name', $.identifier),
      field('type', $.param_type),
      '=',
      field('default', choice($._scalar, $.list_literal)),
    ),

    // type := "text" | "num" | "pct" | "color" | "bool"
    //       | "enum" "(" IDENT ("," IDENT)* [","] ")"
    //       | "list" "(" UIDENT ")"
    param_type: $ => choice(
      'text',
      'num',
      'pct',
      'color',
      'bool',
      seq(
        'enum',
        '(',
        field('member', $.identifier),
        repeat(seq(',', field('member', $.identifier))),
        optional(','),
        ')',
      ),
      seq('list', '(', field('schema', $.identifier), ')'),
    ),

    // List-valued exported-def field declaration: `children=list(Tree)`.
    // Kept distinct from color_function so schema names highlight as types.
    list_schema: $ => prec(2, seq(
      'list',
      '(',
      field('schema', $.identifier),
      ')',
    )),

    list_literal: $ => seq(
      '[',
      repeat($._newline),
      optional($._list_items),
      ']',
    ),

    // Recursive tails give every newline a single owner while retaining
    // required commas, trailing commas, and multiline literals.
    _list_items: $ => seq(
      $.list_item,
      choice(
        seq(',', repeat($._newline), optional($._list_items)),
        repeat($._newline),
      ),
    ),

    list_item: $ => seq(
      field('name', $.identifier),
      '(',
      repeat($._newline),
      optional($._list_fields),
      ')',
    ),

    _list_fields: $ => seq(
      $.list_field,
      choice(
        seq(',', repeat($._newline), optional($._list_fields)),
        repeat($._newline),
      ),
    ),

    list_field: $ => seq(
      field('name', $.identifier),
      '=',
      field('value', choice($.list_literal, $._scalar)),
    ),

    // -- def ----------------------------------------------------------------

    // def := "def" UIDENT "(" [param ("," param)*] ")" ["export"] "{" children "}"
    def_declaration: $ => seq(
      'def',
      field('name', $.identifier),
      '(',
      optional(seq($.parameter, repeat(seq(',', $.parameter)))),
      ')',
      optional(field('export', $.export_flag)),
      repeat($._newline),
      field('body', $.block),
    ),

    // `export` sits on the header line, between `)` and `{`.
    export_flag: $ => 'export',

    // param := IDENT ["=" (scalar | list-schema)]
    parameter: $ => seq(
      field('name', $.identifier),
      optional(seq('=', field('default', choice($.list_schema, $._scalar)))),
    ),

    // -- icons ---------------------------------------------------------------

    // icon := "icon" IDENT ["viewbox" "=" scalar] "{" node* "}"
    // The compiler enforces a positive viewbox and one-or-more static paths.
    icon_declaration: $ => prec(3, seq(
      'icon',
      field('name', $.identifier),
      optional(field('viewbox', $.icon_viewbox)),
      repeat($._newline),
      field('body', $.block),
    )),

    icon_viewbox: $ => seq(
      'viewbox',
      '=',
      field('value', $._scalar),
    ),

    // -- anim -----------------------------------------------------------------

    // anim := "anim" IDENT "{" (PCT "{" attr* "}")* "}"
    anim_declaration: $ => seq(
      'anim',
      field('name', $.identifier),
      repeat($._newline),
      '{',
      repeat(choice($.keyframe, $._newline)),
      '}',
    ),

    // keyframe := PCT "{" attr* "}"
    keyframe: $ => seq(
      field('position', $.percent),
      repeat($._newline),
      '{',
      repeat(choice($.attribute, $._newline)),
      '}',
    ),

    // -- when -----------------------------------------------------------------

    // topwhen := "when" cond "{" tokens* "}"  (only tokens overrides inside)
    when_top: $ => seq(
      'when',
      $.condition,
      repeat($._newline),
      '{',
      repeat(choice($.tokens_declaration, $._newline)),
      '}',
    ),

    // when := "when" cond "{" (attr | flag | node | STRING)* "}"
    when_block: $ => seq(
      'when',
      $.condition,
      repeat($._newline),
      '{',
      repeat(choice($.attribute, $.flag, $.each_statement, $.node, $.string, $._newline)),
      '}',
    ),

    // cond := IDENT | "!" IDENT | ("w"|"h") CMP NUMBER | "theme(" IDENT ")"
    condition: $ => choice(
      seq('theme', '(', field('name', $.identifier), ')'),
      $.identifier,
      seq('!', $.identifier),
      $.comparison,
    ),

    // Size comparison: `w<420`, `h >= 200`.
    comparison: $ => seq(
      field('left', $.identifier),
      field('op', choice('<', '<=', '>', '>=')),
      field('right', $.number),
    ),

    // -- nodes ------------------------------------------------------------------

    // node := NAME ["#" IDENT] (arg | attr | flag)* ["{" children "}"]
    // prec.right keeps the header greedy: items accrue until newline/`{`/`}`.
    node: $ => prec.right(seq(
      field('name', choice($.identifier, 'slot', alias('icon', $.identifier))),
      optional(field('id', $.node_id)),
      repeat(choice($.attribute, $.flag, $._value)),
      optional(field('body', $.block)),
    )),

    // `#card` right after a node name (higher lexical precedence than hex_color).
    node_id: $ => token(prec(1, /#[0-9A-Za-z_-]+/)),

    // each := "each" ("param." IDENT | IDENT) ["#" IDENT]
    //         attr* ["virtual" attr*]
    // The split attribute runs keep `virtual` and following attrs in one header
    // while allowing the flag at any valid position among those attrs.
    // Bare targets address a List-valued field of the enclosing item schema.
    each_statement: $ => prec.right(3, seq(
      'each',
      field('target', choice($.reference, $.identifier)),
      optional(field('id', $.node_id)),
      optional($._each_attributes),
      optional(seq(
        field('flag', $.each_flag),
        optional($._each_attributes),
      )),
    )),

    each_flag: $ => 'virtual',

    // Right-recursive and hidden so every same-line `name=value` stays a
    // direct each child instead of being reconsidered as a sibling node.
    _each_attributes: $ => prec.right(5, seq(
      $.attribute,
      optional($._each_attributes),
    )),

    // children := (node | each | STRING | when)*
    block: $ => seq(
      '{',
      repeat(choice($.each_statement, $.node, $.string, $.when_block, $._newline)),
      '}',
    ),

    attribute: $ => seq(
      field('name', choice($.identifier, alias('scroll', $.identifier))),
      '=',
      field('value', $._value),
    ),

    // flag := one of the closed FLAGS set, surfaced as (flag (identifier)).
    flag: $ => alias(choice(...FLAGS), $.identifier),

    // -- values ---------------------------------------------------------------

    // Values include typed key-to-signal maps for `keys=Escape:close,F2:rename`.
    _value: $ => choice($.key_map, $.tuple, $._scalar),

    key_map: $ => seq($.key_binding, repeat(seq(',', $.key_binding))),

    key_binding: $ => seq(
      field('key', choice($.identifier, $.string)),
      ':',
      field('signal', choice($.identifier, $.string)),
    ),

    // tuple := scalar ("," scalar)+  e.g. `pad=4,10`, `0,18,44,#00000073`
    tuple: $ => seq($._scalar, repeat1(seq(',', $._scalar))),

    _scalar: $ => choice(
      $.number,
      $.percent,
      $.string,
      $.hex_color,
      $.reference,
      $.fill_size,
      $.color_function,
      $.identifier,
    ),

    // fill weight: `fill` or `fill:2`
    fill_size: $ => prec.right(seq('fill', optional(seq(':', $.number)))),

    // color fn: `oklch(72% 0.16 250)`, `linear(90, #fff 0%, #000 100%)` —
    // args are raw tokens until the balanced `)`.
    color_function: $ => seq(
      field('name', $.identifier),
      '(',
      repeat($._color_arg),
      ')',
    ),

    _color_arg: $ => choice(
      $.number,
      $.percent,
      $.hex_color,
      $.reference,
      $.identifier,
      $.string,
      ',',
      ':',
      $._newline,
      $._color_paren_group,
    ),

    _color_paren_group: $ => seq('(', repeat($._color_arg), ')'),

    // -- terminals ----------------------------------------------------------------

    // Dotted token path with no spaces: `color.bg`, `text.title`.
    reference: $ => token(
      /[A-Za-z_][A-Za-z0-9_]*(-[A-Za-z_][A-Za-z0-9_]*)*(\.[A-Za-z_][A-Za-z0-9_]*(-[A-Za-z_][A-Za-z0-9_]*)*)+/,
    ),

    identifier: $ => IDENT,

    // `-?` digits with optional `.` (also `.5`, `-.5`)
    number: $ => token(/-?(\d+\.?\d*|\.\d+)/),

    // number immediately followed by `%`
    percent: $ => token(/-?(\d+\.?\d*|\.\d+)%/),

    // `#` + hash word in value position: `#0e1116`, `#FFFFFF1A`
    hex_color: $ => token(/#[0-9A-Za-z_-]+/),

    // "..." with escapes; may contain raw newlines.
    string: $ => seq(
      '"',
      repeat(choice(
        $.escape_sequence,
        token.immediate(prec(1, /[^"\\]+/)),
      )),
      token.immediate('"'),
    ),

    // `\n \t \" \\ \_` (nbsp); any other `\x` = literal x.
    escape_sequence: $ => token.immediate(/\\[\s\S]/),

    // `// ...` to EOL; `/* ... */` block (no nesting).
    comment: $ => token(choice(
      seq('//', /[^\n]*/),
      seq('/*', /[^*]*\*+([^/*][^*]*\*+)*/, '/'),
    )),

    // `;` is equivalent to a newline (hidden separator token).
    _newline: $ => /;|\n/,
  },
});
