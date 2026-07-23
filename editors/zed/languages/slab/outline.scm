; Outline entries for defs, icons, anims, params, token groups, and id-carrying nodes.

(def_declaration
  "def" @context
  name: (identifier) @name) @item

(icon_declaration
  "icon" @context
  name: (identifier) @name) @item

(anim_declaration
  "anim" @context
  name: (identifier) @name) @item

(tokens_declaration
  "tokens" @name) @item

(token_group
  name: (identifier) @name) @item

(params_declaration
  "params" @name) @item

(param_declaration
  name: (identifier) @name
  type: (param_type) @context) @item

(node
  name: (identifier) @context
  id: (node_id) @name) @item
