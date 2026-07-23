; Slab syntax highlighting against the generalized tree-sitter node contract.

(comment) @comment

; keywords are anonymous tokens in the grammar
[
  "tokens"
  "params"
  "list"
  "each"
  "def"
  "anim"
  "when"
  "slot"
  "theme"
  "icon"
] @keyword

(export_flag) @keyword
(icon_viewbox "viewbox" @property)

; fill sizing keyword (fill / fill:2); inner number re-captured below
(fill_size) @keyword
(fill_size (number) @number)

; declarations, params, and nested list schemas
(def_declaration name: (identifier) @type)
(anim_declaration name: (identifier) @function)
(icon_declaration name: (identifier) @constant)
(theme_declaration name: (identifier) @constant)
(parameter name: (identifier) @variable.parameter)
(token_group name: (identifier) @property)
(token_entry name: (identifier) @property)
(keyframe position: (percent) @label)
(list_item name: (identifier) @type)

(param_declaration name: (identifier) @variable)
(param_type) @type
(param_type member: (identifier) @constant)
(param_type schema: (identifier) @type)
(list_schema) @type
(list_schema schema: (identifier) @type)
(each_statement target: (identifier) @variable.special)
; `hole NAME` is the host-surface viewport (§13.2)
(node
  name: (identifier) @keyword
  (#eq? @keyword "hole"))

; builtin node names
(node
  name: (identifier) @tag
  (#any-of? @tag
    "box" "row" "col" "wrap" "grid" "stack" "canvas" "para" "group"
    "text" "span" "rect" "img" "path" "spacer" "slot" "divider" "icon"))
(node
  name: (identifier) @_icon
  (identifier) @constant
  (#eq? @_icon "icon"))

; component calls are Capitalized
(node
  name: (identifier) @type
  (#match? @type "^[A-Z]"))

(node_id) @label

; host bindings and gesture signals
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
(attribute name: (identifier) @property)
(flag (identifier) @attribute)
(each_statement flag: (each_flag) @attribute)

; closed value-keyword vocabulary in attribute value position
(attribute
  value: (identifier) @constant.builtin
  (#any-of? @constant.builtin
    "hug" "fill"
    "start" "center" "end" "baseline" "stretch" "between"
    "top-start" "top" "top-end" "bottom-start" "bottom" "bottom-end"
    "row" "col"
    "inside" "outside"
    "t" "r" "b" "l" "top" "right" "bottom" "left"
    "cover" "contain"
    "full" "sm" "md" "lg" "inset"
    "loop" "once" "alternate"
    "linear" "ease" "ease-in" "ease-out" "ease-in-out"
    "true" "false"
    "cross" "both" "current" "auto" "none"
    "below-start" "below-center" "below-end"
    "above-start" "above-center" "above-end"
    "left-start" "left-center" "left-end"
    "right-start" "right-center" "right-end"))
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

; condition identifiers: env flags, renderer classes, interaction states
(condition
  (identifier) @constant.builtin
  (#any-of? @constant.builtin
    "portrait" "landscape" "dark" "coarse" "web" "gpu" "tui" "svg" "png"
    "hover" "pressed" "focus" "focus-visible" "disabled" "selected"
    "composing" "dragging" "drop"))
(comparison (identifier) @variable.special)

; literals
(string) @string
(escape_sequence) @string.escape
(number) @number
(percent) @number
(hex_color) @constant
(reference) @constant
(color_function name: (identifier) @function)

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
