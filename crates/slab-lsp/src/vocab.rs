//! Closed vocabulary tables for completion and hover. The authored surface is
//! kept here with context, value types, and deliberate v1 boundaries so editor
//! help agrees with compiler diagnostics.

pub const CONTAINERS: &[&str] = &[
    "box", "row", "col", "wrap", "grid", "stack", "canvas", "para", "group",
];
pub const LEAVES: &[&str] = &[
    "text", "span", "rect", "img", "path", "spacer", "hole", "divider", "icon",
];
pub const COLOR_FNS: &[&str] = &["rgb", "rgba", "oklch", "hsl"];
pub const PAINT_FNS: &[&str] = &["linear", "radial", "conic"];
pub const EASINGS: &[&str] = &["linear", "ease", "ease-in", "ease-out", "ease-in-out"];
pub const ANIM_MODES: &[&str] = &["loop", "once", "alternate"];
pub const POSITIONS9: &[&str] = &[
    "top-start",
    "top",
    "top-end",
    "start",
    "center",
    "end",
    "bottom-start",
    "bottom",
    "bottom-end",
];
pub const SIZING: &[&str] = &["hug", "fill"];
pub const GRAVITIES: &[&str] = &[
    "below-start",
    "below-center",
    "below-end",
    "above-start",
    "above-center",
    "above-end",
    "left-start",
    "left-center",
    "left-end",
    "right-start",
    "right-center",
    "right-end",
];
pub const PARAM_TYPES: &[&str] = &["text", "num", "pct", "color", "bool", "enum", "list"];
pub const CONDITIONS: &[&str] = &[
    "hover",
    "pressed",
    "focus",
    "focus-visible",
    "disabled",
    "selected",
    "composing",
    "dragging",
    "drop",
    "portrait",
    "landscape",
    "dark",
    "coarse",
    "web",
    "gpu",
    "tui",
    "svg",
    "png",
];

pub fn builtins() -> impl Iterator<Item = &'static str> {
    CONTAINERS.iter().chain(LEAVES.iter()).copied()
}

pub const ATTR_DOCS: &[(&str, &str)] = &[
    ("w", "Width: number in u, `40%`, `hug`, or `fill[:weight]`."),
    (
        "h",
        "Height: number in u, `40%`, `hug`, or `fill[:weight]`.",
    ),
    ("min-w", "Lower bound on width in u."),
    ("max-w", "Upper bound on width in u."),
    ("min-h", "Lower bound on height in u."),
    ("max-h", "Upper bound on height in u."),
    (
        "pad",
        "Padding: `pad=16` (all), `pad=v,h`, or `pad=t,r,b,l`.",
    ),
    (
        "gap",
        "Space between children: `gap=8` or `gap=main,cross` (grid row / wrap line gap).",
    ),
    ("axis", "Layout axis of a box: `row` or `col`."),
    (
        "pack",
        "Main-axis distribution of children: `start|center|end|between`.",
    ),
    (
        "align",
        "Cross-axis alignment of children: `start|center|end|baseline|stretch` (stack: 9-position).",
    ),
    (
        "self",
        "Per-child alignment override; on stack children a 9-position anchor.",
    ),
    (
        "offset",
        "Ink-only nudge `offset=x,y` after layout — the declared overlap opt-in.",
    ),
    (
        "at",
        "Canvas position `at=x,y` (default addresses the child's top-left; see `anchor`).",
    ),
    (
        "anchor",
        "Which point of the child `at` addresses: 9-position `top-start`…`bottom-end`.",
    ),
    (
        "bg",
        "Background: color or gradient paint (`linear(...)`, `radial(...)`, `conic(...)`).",
    ),
    (
        "stroke",
        "Border / outline color or gradient paint (`linear(...)`, `radial(...)`, `conic(...)`).",
    ),
    ("stroke-w", "Stroke width in u (default 1)."),
    (
        "stroke-dash",
        "Dash pattern in u: `stroke-dash=16,14` (single value = even dashes).",
    ),
    (
        "stroke-align",
        "Stroke placement on the edge: `inside|center|outside` (default center).",
    ),
    (
        "stroke-sides",
        "Border only these sides: subset of `t,r,b,l` (radius ignored).",
    ),
    (
        "radius",
        "Corner radius in u (`999` ≈ pill; `full` = pill).",
    ),
    (
        "smooth",
        "Corner smoothing 0–1 (squircle; iOS ≈ 0.6). No-op unless `radius>0`; ink and clip only.",
    ),
    (
        "shadow",
        "Preset `sm|md|lg`, `[inset,]x,y,blur,color`, or a layered list of presets/token refs.",
    ),
    (
        "blur",
        "Self blur in u: node and children render to a layer, blur, composite.",
    ),
    (
        "backdrop",
        "Glass: `backdrop=blur[,saturation[,brightness]]` blurs what is already painted beneath the node.",
    ),
    (
        "backdrop-mask",
        "Progressive blur: any paint — backdrop strength scaled by the paint's alpha (requires `backdrop`).",
    ),
    (
        "grain",
        "Deterministic speckle over the node's fill area: `grain=amount[,size]` (amount 0–1, size in u).",
    ),
    (
        "mask",
        "Alpha fade: any paint — the node and its children fade by the paint's alpha over the border box.",
    ),
    (
        "animate",
        "`animate=NAME,dur[,loop|once|alternate][,easing][,delay]` — run named keyframes.",
    ),
    (
        "transition",
        "`transition=dur[,easing][,delay]` — ease this node's `when`-state patches.",
    ),
    (
        "opacity",
        "0–1; children blend first, then the group fades as one layer.",
    ),
    (
        "color",
        "Text color or gradient paint (maps over the text node's content box). Inherits.",
    ),
    ("family", "Font family. Inherits."),
    ("size", "Font size in u. Inherits."),
    ("weight", "Font weight (100–900). Inherits."),
    ("leading", "Line-height multiplier (default 1.4). Inherits."),
    ("tracking", "Letter-spacing in u, added after every glyph."),
    (
        "rotate",
        "Rotation in degrees; ±90/270 are layout-aware, arbitrary angles are ink-only.",
    ),
    (
        "scale",
        "Ink-only zoom about the node center: `scale=1.05` or `scale=sx,sy`. Never layout; hit-testing keeps the layout rect.",
    ),
    (
        "tilt",
        "Ink-only 3D perspective: `tilt=rx[,ry[,depth]]` in degrees (depth in u, default 800). Subtree flattens.",
    ),
    (
        "align-text",
        "Horizontal text alignment within the text box: `start|center|end`.",
    ),
    ("fit", "Image scaling: `cover|contain|stretch`."),
    (
        "src",
        "Image name/path: string, Text param, or Text item prop. Runtime registrations override compiled images; a missing image keeps layout but paints nothing.",
    ),
    (
        "d",
        "SVG path data: literal string, Text param, or Text item prop. `path` remains canvas-only; malformed runtime data paints nothing.",
    ),
    (
        "cols",
        "Grid column tracks: `cols=120,fill,hug` — same sizing vocabulary.",
    ),
    ("span", "Grid cell: occupy the next N columns."),
    (
        "style",
        "Token group applied as an attribute bundle, e.g. `style=text.title`; explicit attrs win.",
    ),
    (
        "key",
        "Stable identity for per-node state across reorders (reserved on every node).",
    ),
    (
        "act",
        "Activate signal name: primary pointer-up or Enter/Space on an enabled focusable node.",
    ),
    (
        "field",
        "Kernel-edited text field signal: every mutation emits the full committed text; add `multiline` for newline editing.",
    ),
    (
        "submit",
        "Submit signal name. Legal only on a `field=` text node; Enter emits the full committed text.",
    ),
    (
        "cancel",
        "Cancel signal name. Legal only on a `field=` text node; escape-blur emits the retained buffer text.",
    ),
    (
        "keys",
        "Comma-separated portable activation keys. Implies `focusable`; platform-specific key names are not supported.",
    ),
    (
        "scroll",
        "Cross-axis mode: `scroll=cross|both`. Bare `scroll` remains the main-axis flag.",
    ),
    (
        "item-extent",
        "Positive uniform item extent in u. Required on a top-level `virtual` each inside a main-axis scroll container.",
    ),
    (
        "overscan",
        "Extra virtual-list items materialized before and after the viewport; nonnegative number, default 4.",
    ),
    (
        "attach",
        "Exact full node key as a string, Text param, or Text item prop. Valid only on a direct child of `stack`/`canvas`; a missing anchor omits the overlay.",
    ),
    (
        "gravity",
        "Anchored-overlay side/alignment (`below|above|left|right` × `start|center|end`), default `below-start`; requires `attach=`.",
    ),
    (
        "collide",
        "Anchored-overlay viewport policy: `auto` flips then slides; `none` preserves placement. Default `auto`; host owns dismissal/focus trapping.",
    ),
    (
        "press",
        "Signal name fired on primary pointer-down before capture; implies `focusable`.",
    ),
    (
        "context",
        "Signal name fired on secondary pointer-down; does not press or focus the node.",
    ),
    (
        "dblclick",
        "Signal name fired when the host reports click count 2; suppresses that gesture's later Activate.",
    ),
    (
        "pointer-move",
        "Signal name emitted on every pointer move to the deepest enabled binding in the current hit path, or the captured owner path while captured.",
    ),
    (
        "pointer-up",
        "Signal name emitted once on primary release to the deepest enabled binding in the captured/current path.",
    ),
    (
        "drag",
        "DragStart signal name. Arms on primary down, starts after movement exceeds 4u, and implies `focusable`; pair with `drag-update=`, `drag-end=`, or `drag-ghost` as needed.",
    ),
    (
        "drag-update",
        "Requires `drag=` on the same node; emits on that source on the threshold-crossing move and every later active move.",
    ),
    (
        "drag-end",
        "Requires `drag=` on the same node; emits exactly once on that source after normal release or abnormal cancellation, with typed `cancelled`/`dropped` metadata.",
    ),
    (
        "drop",
        "Drop signal name on the deepest eligible target; metadata identifies the drag source.",
    ),
    (
        "resize",
        "Optional divider Resize signal. Pointer-up/keyboard adjustment emits the final extent as text; other nodes do not emit it.",
    ),
    (
        "role",
        "Accessibility role exported on the scene node. Open identifier or string, not a closed enum.",
    ),
    (
        "label",
        "Accessibility label: string literal, Text param, or Text item prop.",
    ),
    (
        "desc",
        "Accessibility description: string literal, Text param, or Text item prop.",
    ),
    (
        "checked",
        "Tri-state widget state: `false|true|mixed`, or a compatible Bool/Enum param or item prop. Valid on any node.",
    ),
    (
        "expanded",
        "Disclosure state exported as Bool; accepts a Bool literal, param, or item prop on any node.",
    ),
    (
        "selected",
        "Collection-item selection state exported as Bool; accepts a Bool literal, param, or item prop on any node.",
    ),
    (
        "active-descendant",
        "Full key of the currently active descendant: Text literal, param, or item prop on any node; static relationship keys are compile-validated.",
    ),
    (
        "controls",
        "Full key of a controlled node: Text literal, param, or item prop on any node; static relationship keys are compile-validated.",
    ),
    (
        "value-now",
        "Current numeric range value: Num literal, param, or item prop on any node; static ranges are compile-validated.",
    ),
    (
        "value-min",
        "Minimum numeric range value: Num literal, param, or item prop on any node; static ranges are compile-validated.",
    ),
    (
        "value-max",
        "Maximum numeric range value: Num literal, param, or item prop on any node; static ranges are compile-validated.",
    ),
    (
        "value-text",
        "Human-readable value text: Text literal, param, or item prop on any node.",
    ),
    (
        "modal",
        "Marks a modal surface; accepts a Bool literal, param, or item prop on any node.",
    ),
    (
        "live",
        "Live-region priority on any node: `off|polite|assertive`, or a compatible Enum param or item prop.",
    ),
    (
        "live-atomic",
        "Whether live-region announcements include the whole region; accepts a Bool literal, param, or item prop on any node.",
    ),
    (
        "level",
        "Hierarchy depth: positive-integer Num literal, compatible param, or item prop on any node.",
    ),
    (
        "pos-in-set",
        "One-based collection position: positive-integer Num literal, compatible param, or item prop; static values may not exceed `set-size`.",
    ),
    (
        "set-size",
        "Collection cardinality: positive-integer Num or `-1` for unknown, compatible param, or item prop on any node.",
    ),
    (
        "viewbox",
        "Positive square design-box size on a top-level `icon` declaration; default 24.",
    ),
];

pub const NODE_DOCS: &[(&str, &str)] = &[
    (
        "box",
        "Generic container; `axis=row|col` picks the direction.",
    ),
    (
        "row",
        "Children along the horizontal axis, in document order, with `gap`.",
    ),
    (
        "col",
        "Children along the vertical axis, in document order, with `gap`.",
    ),
    (
        "wrap",
        "Like `row`, but starts a new line when out of room.",
    ),
    ("grid", "Column tracks that agree across rows (`cols=...`)."),
    (
        "stack",
        "Children on top of each other; later = above. Overlap opt-in.",
    ),
    (
        "canvas",
        "Children where you say (`at=x,y`); SVG-ish leaves allowed. Overlap opt-in.",
    ),
    (
        "para",
        "Inline text flow: strings, `span`s, and `each` templates containing exactly one span wrap as one paragraph.",
    ),
    (
        "group",
        "Flow island: a plain box usable with `at`/`anchor` inside `canvas`.",
    ),
    ("text", "Single-style text leaf. Wraps by default."),
    ("span", "Styled run inside `para`."),
    ("rect", "Empty styled box."),
    (
        "img",
        "Image; `src` accepts a literal name, Text param, or Text item prop; `fit=cover|contain|stretch`.",
    ),
    (
        "path",
        "Vector path with literal or Text-valued SVG `d`; canvas only. Interior paint is `bg`, outline `stroke`.",
    ),
    (
        "spacer",
        "Sugar for `rect w=fill` (in a row) / `h=fill` (in a col).",
    ),
    (
        "slot",
        "Placeholder inside a `def` body where call-site children are injected.",
    ),
    (
        "hole",
        "`hole NAME` — host-filled viewport (§13.2). Both axes must be determinate; takes sizing attrs and `scroll`/`clip` flags; no children.",
    ),
    (
        "divider",
        "Focusable split handle between non-edge children of `row`/`col`; controls the previous pane. No collapse threshold or automatic initial allocation.",
    ),
    (
        "icon",
        "`icon NAME [size=N]`: square named vector usage; NAME may be literal, Text param, or Text item prop. Declare it at top level with `icon NAME [viewbox=N] { path ... }`. Unknown names keep layout and paint nothing.",
    ),
];

pub const FLAG_DOCS: &[(&str, &str)] = &[
    ("clip", "Clip children to the content box."),
    (
        "bleed",
        "Let children exceed bounds without clipping (intentional and silent).",
    ),
    (
        "scroll",
        "Enable main-axis scrolling. Use `scroll=cross|both` for cross-axis ownership.",
    ),
    ("nowrap", "Do not wrap text; overflow instead."),
    (
        "ellipsis",
        "Truncate the last line with `…` when out of room.",
    ),
    ("inert", "Subtree ignored by hit testing and focus."),
    ("focusable", "Participates in tab order."),
    (
        "multiline",
        "Allow newline editing; legal only on a `field=` text node.",
    ),
    (
        "sticky",
        "Pin a direct child of a main-axis scroll container to its start edge. Cross/end-edge sticky is deliberately unsupported.",
    ),
    (
        "virtual",
        "Window a top-level `each` in a main-axis scroll container. Requires positive `item-extent=`; nested virtual each is unsupported.",
    ),
    (
        "drag-ghost",
        "Valid only on a `drag=` source. While active, paint a 0.72-opacity cursor-following duplicate above content; it is excluded from hit testing, scene export, and accessibility.",
    ),
];

pub const KEYWORD_DOCS: &[(&str, &str)] = &[
    (
        "tokens",
        "Design-token block: nestable named groups of values.",
    ),
    (
        "def",
        "Define a component (Capitalized name) with parameters.",
    ),
    (
        "anim",
        "Named keyframe animation: `anim name { 0% {...} 100% {...} }`.",
    ),
    (
        "when",
        "Conditional patch: renderer class, state ident, env flag, or `w|h` comparison.",
    ),
    (
        "slot",
        "Placeholder inside a `def` body where call-site children are injected.",
    ),
    (
        "params",
        "Typed host inputs: scalar/enum values or recursive `list(ExportedDef)` values, all with required defaults.",
    ),
    (
        "export",
        "After a def's `)`: the component is a standalone host-instantiable unit (§13.4).",
    ),
    (
        "theme",
        "Named token theme declaration: `theme NAME { ... }`.",
    ),
    (
        "list",
        "Typed recursive list schema: `list(ExportedDef)`. List-valued def fields may nest or recurse.",
    ),
    (
        "each",
        "List template: `each param.NAME` at a root or `each list_field` when nested. It has no block; `hole` remains forbidden. Direct `para` use requires a template containing exactly one `span`.",
    ),
    (
        "icon",
        "Top-level declaration `icon NAME [viewbox=N] { path ... }`; body must contain one or more static paths.",
    ),
];

pub const TYPE_DOCS: &[(&str, &str)] = &[
    ("text", "Text value."),
    ("num", "Unitless number / logical-unit value."),
    ("pct", "Percentage value."),
    ("color", "Color or paint value."),
    ("bool", "Boolean `true|false`."),
    ("enum", "Closed identifier set: `enum(first, second, ...)`."),
    (
        "list",
        "Persistent typed list: `list(ExportedDef)`. Schemas may contain nested/recursive list fields.",
    ),
];

pub const VALUE_DOCS: &[(&str, &str)] = &[
    ("hug", "Size to content."),
    (
        "fill",
        "Share of the parent's leftover space (`fill:2` = weight 2).",
    ),
    ("start", "Align to the start edge."),
    ("center", "Align to the center."),
    ("end", "Align to the end edge."),
    ("baseline", "Align text baselines."),
    ("stretch", "Stretch to the container's cross size."),
    ("between", "Distribute leftover space between children."),
    ("row", "Horizontal axis."),
    ("col", "Vertical axis."),
    ("inside", "Stroke inside the edge."),
    ("outside", "Stroke outside the edge."),
    ("cover", "Scale to cover the box; crop overflow."),
    ("contain", "Scale to fit entirely inside the box."),
    ("full", "Pill radius."),
    ("sm", "Small shadow preset."),
    ("md", "Medium shadow preset."),
    ("lg", "Large shadow preset."),
    ("inset", "Inner shadow; paints above the fill."),
    ("loop", "Repeat forever."),
    ("once", "Play once and hold the last frame."),
    ("alternate", "Ping-pong between first and last frame."),
    ("linear", "Constant-rate easing."),
    ("ease", "Standard ease curve."),
    ("ease-in", "Accelerate from rest."),
    ("ease-out", "Decelerate to rest."),
    ("ease-in-out", "Accelerate then decelerate."),
    ("t", "Top side."),
    ("r", "Right side."),
    ("b", "Bottom side."),
    ("l", "Left side."),
    ("top", "Top side / top anchor."),
    ("right", "Right side."),
    ("bottom", "Bottom side / bottom anchor."),
    ("left", "Left side."),
    ("top-start", "Top-start anchor."),
    ("top-end", "Top-end anchor."),
    ("bottom-start", "Bottom-start anchor."),
    ("bottom-end", "Bottom-end anchor."),
    ("cross", "Enable only the scroll container's cross axis."),
    ("both", "Enable both main- and cross-axis scrolling."),
    (
        "current",
        "Inside an icon declaration only: resolve `bg`/`stroke` to the icon usage's inherited `color`.",
    ),
    (
        "auto",
        "Anchored overlay collision policy: flip on main-axis overflow, then slide into the viewport.",
    ),
    ("none", "Disable anchored-overlay collision adjustment."),
    ("below-start", "Place below the anchor, start-aligned."),
    ("below-center", "Place below the anchor, center-aligned."),
    ("below-end", "Place below the anchor, end-aligned."),
    ("above-start", "Place above the anchor, start-aligned."),
    ("above-center", "Place above the anchor, center-aligned."),
    ("above-end", "Place above the anchor, end-aligned."),
    ("left-start", "Place left of the anchor, start-aligned."),
    ("left-center", "Place left of the anchor, center-aligned."),
    ("left-end", "Place left of the anchor, end-aligned."),
    ("right-start", "Place right of the anchor, start-aligned."),
    ("right-center", "Place right of the anchor, center-aligned."),
    ("right-end", "Place right of the anchor, end-aligned."),
    (
        "dragging",
        "Kernel-owned state on the active drag source; cleared on release/cancel.",
    ),
    (
        "drop",
        "Kernel-owned state on the current eligible drop target; cleared on leave/release/cancel.",
    ),
    (
        "selected",
        "Host-driven `when selected` interaction state; distinct from the `selected=` accessibility attribute.",
    ),
    ("false", "Boolean false."),
    ("true", "Boolean true."),
    ("mixed", "Indeterminate `checked=` state."),
    ("off", "Disable live-region announcements."),
    (
        "polite",
        "Announce live-region changes when assistive technology is idle.",
    ),
    (
        "assertive",
        "Announce live-region changes with high priority.",
    ),
];

const ALIGN: &[&str] = &[
    "start",
    "center",
    "end",
    "baseline",
    "stretch",
    "top-start",
    "top",
    "top-end",
    "bottom-start",
    "bottom",
    "bottom-end",
];

/// Closed value vocabularies per attribute (empty slice = numeric only).
pub const ATTR_VALUES: &[(&str, &[&str])] = &[
    ("w", SIZING),
    ("h", SIZING),
    ("min-w", &[]),
    ("max-w", &[]),
    ("min-h", &[]),
    ("max-h", &[]),
    ("axis", &["row", "col"]),
    ("pack", &["start", "center", "end", "between"]),
    ("align", ALIGN),
    ("self", ALIGN),
    ("anchor", POSITIONS9),
    ("stroke-align", &["inside", "center", "outside"]),
    (
        "stroke-sides",
        &["t", "r", "b", "l", "top", "right", "bottom", "left"],
    ),
    ("fit", &["cover", "contain", "stretch"]),
    ("radius", &["full"]),
    ("shadow", &["sm", "md", "lg", "inset"]),
    (
        "animate",
        &[
            "loop",
            "once",
            "alternate",
            "linear",
            "ease",
            "ease-in",
            "ease-out",
            "ease-in-out",
        ],
    ),
    ("transition", EASINGS),
    ("align-text", &["start", "center", "end"]),
    ("cols", SIZING),
    ("scroll", &["cross", "both"]),
    ("gravity", GRAVITIES),
    ("collide", &["auto", "none"]),
    ("checked", &["false", "true", "mixed"]),
    ("expanded", &["false", "true"]),
    ("selected", &["false", "true"]),
    ("modal", &["false", "true"]),
    ("live", &["off", "polite", "assertive"]),
    ("live-atomic", &["false", "true"]),
];

pub const COLOR_ATTRS: &[&str] = &["bg", "stroke", "color", "shadow", "mask", "backdrop-mask"];

pub fn lookup<'t>(table: &'t [(&str, &str)], key: &str) -> Option<&'t str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

pub fn attr_values(attr: &str) -> Option<&'static [&'static str]> {
    ATTR_VALUES
        .iter()
        .find(|(k, _)| *k == attr)
        .map(|(_, v)| *v)
}
