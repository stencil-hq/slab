; Slab syntax highlighting (standard tree-sitter capture names).
; Specific patterns first: the tree-sitter highlighter gives earlier
; patterns precedence.

; comment: line and block comments
(comment) @comment

; string: literals and escapes
(string) @string
(escape_sequence) @string.escape

; number: plain numbers and percents
(number) @number
(percent) @number

; constant: colors and token references
(hex_color) @string.special
(reference) @constant
(node_id) @constant
(theme_declaration
  name: (identifier) @constant)

; color function: name as function call, e.g. oklch(...) rgb(...)
(color_function
  name: (identifier) @function)

; keyword: structural keywords (anonymous tokens)
[
  "import"
  "tokens"
  "params"
  "def"
  "anim"
  "when"
  "slot"
  "theme"
  "each"
  "icon"
] @keyword

(fill_size "fill" @keyword)
(export_flag) @keyword
(icon_viewbox "viewbox" @property)

; parameter and nested-list types
(param_type) @type.builtin
(param_type
  member: (identifier) @constant)
(param_type
  schema: (identifier) @type)
(list_schema) @type.builtin
(list_schema
  schema: (identifier) @type)

; declaration names
(def_declaration
  name: (identifier) @type)
(anim_declaration
  name: (identifier) @function)
(icon_declaration
  name: (identifier) @constant)
(parameter
  name: (identifier) @variable)
(param_declaration
  name: (identifier) @variable)
(params_declaration
  name: (identifier) @namespace)

(list_item
  name: (identifier) @type)
(each_statement
  target: (identifier) @variable.member)
; node names: `hole` is structural, other lowercase builtins -> tag,
; Capitalized -> component type
(node
  name: (identifier) @keyword
  (#eq? @keyword "hole"))
(node
  name: (identifier) @tag
  (#any-of? @tag
    "box" "row" "col" "wrap" "grid" "stack" "canvas" "para" "group"
    "text" "span" "rect" "img" "path" "spacer" "divider" "icon"))
(node
  name: (identifier) @type
  (#match? @type "^[A-Z]"))
(node
  name: (identifier) @_icon
  (identifier) @constant
  (#eq? @_icon "icon"))

; attributes, token entries, flags; interaction bindings are attributes
(attribute
  name: (identifier) @attribute
  (#any-of? @attribute
    "act" "field" "submit" "keys" "press" "context" "dblclick"
    "pointer-move" "pointer-up" "drag" "drag-update" "drag-end" "drop" "resize"))
(attribute
  name: (identifier) @attribute
  (#any-of? @attribute
    "role" "label" "desc" "checked" "expanded" "selected"
    "active-descendant" "controls" "value-now" "value-min" "value-max"
    "value-text" "modal" "live" "live-atomic" "level" "pos-in-set" "set-size"))
(attribute
  name: (identifier) @property)
(token_entry
  name: (identifier) @property)
(token_group
  name: (identifier) @property)
(flag
  (identifier) @attribute)
(each_statement
  flag: (each_flag) @attribute)

; closed scrolling, overlay, icon-paint, and accessibility values
(attribute
  name: (identifier) @_scroll
  value: (identifier) @constant.builtin
  (#eq? @_scroll "scroll")
  (#any-of? @constant.builtin "cross" "both"))
(attribute
  name: (identifier) @_gravity
  value: (identifier) @constant.builtin
  (#eq? @_gravity "gravity")
  (#any-of? @constant.builtin
    "below-start" "below-center" "below-end"
    "above-start" "above-center" "above-end"
    "left-start" "left-center" "left-end"
    "right-start" "right-center" "right-end"))
(attribute
  name: (identifier) @_collide
  value: (identifier) @constant.builtin
  (#eq? @_collide "collide")
  (#any-of? @constant.builtin "auto" "none"))
(attribute
  name: (identifier) @_checked
  value: (identifier) @constant.builtin
  (#eq? @_checked "checked")
  (#any-of? @constant.builtin "false" "true" "mixed"))
(attribute
  name: (identifier) @_live
  value: (identifier) @constant.builtin
  (#eq? @_live "live")
  (#any-of? @constant.builtin "off" "polite" "assertive"))
(attribute
  name: (identifier) @_a11y_bool
  value: (identifier) @constant.builtin
  (#any-of? @_a11y_bool "expanded" "selected" "modal" "live-atomic")
  (#any-of? @constant.builtin "false" "true"))
(attribute
  value: (identifier) @constant.builtin
  (#eq? @constant.builtin "current"))

; when conditions: env / renderer-class ids and kernel-owned states
(condition
  (identifier) @constant.builtin
  (#any-of? @constant.builtin
    "portrait" "landscape" "dark" "coarse"
    "web" "gpu" "tui" "svg" "png"
    "hover" "pressed" "focus" "focus-visible" "disabled" "selected"
    "composing" "dragging" "drop"))
(comparison
  left: (identifier) @variable)
(condition
  (identifier) @variable)

; operators and punctuation
[
  "<"
  "<="
  ">"
  ">="
  "="
  "!"
] @operator

[
  "{"
  "}"
  "("
  ")"
  "["
  "]"
] @punctuation.bracket

[
  ","
  ":"
] @punctuation.delimiter

; fallback: bare value keywords (hug, center, ease-in-out, ...)
(identifier) @variable
