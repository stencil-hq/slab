//! Attribute-name -> u16 id table. This table is normative: `spec/SLIR.md`
//! mirrors it and `slab_kernel::slir` must match. Ids 0..=38 are the authorable 0.5
//! attribute set; 39..=49 are compiler/kernel channels and 1.0/1.1
//! reserved-meaning attributes.

macro_rules! attr_table {
    ($( $const:ident = $id:literal, $name:literal; )*) => {
        $( pub const $const: u16 = $id; )*

        /// Author-facing (and dump) name for an attr id.
        pub fn attr_name(id: u16) -> Option<&'static str> {
            match id {
                $( $id => Some($name), )*
                _ => None,
            }
        }

        /// Attr id for an author-facing name.
        pub fn attr_id(name: &str) -> Option<u16> {
            match name {
                $( $name => Some($id), )*
                _ => None,
            }
        }

        /// Every `(id, name)` pair, ascending.
        pub const ATTRS: &[(u16, &str)] = &[ $( ($id, $name), )* ];
    };
}

attr_table! {
    W = 0, "w";
    H = 1, "h";
    MIN_W = 2, "min-w";
    MAX_W = 3, "max-w";
    MIN_H = 4, "min-h";
    MAX_H = 5, "max-h";
    PAD = 6, "pad";
    GAP = 7, "gap";
    AXIS = 8, "axis";
    PACK = 9, "pack";
    ALIGN = 10, "align";
    SELF_ALIGN = 11, "self";
    OFFSET = 12, "offset";
    AT = 13, "at";
    ANCHOR = 14, "anchor";
    BG = 15, "bg";
    STROKE = 16, "stroke";
    STROKE_W = 17, "stroke-w";
    STROKE_ALIGN = 18, "stroke-align";
    STROKE_SIDES = 19, "stroke-sides";
    STROKE_DASH = 20, "stroke-dash";
    RADIUS = 21, "radius";
    SHADOW = 22, "shadow";
    BLUR = 23, "blur";
    BACKDROP = 24, "backdrop";
    OPACITY = 25, "opacity";
    COLOR = 26, "color";
    FAMILY = 27, "family";
    SIZE = 28, "size";
    WEIGHT = 29, "weight";
    LEADING = 30, "leading";
    TRACKING = 31, "tracking";
    ROTATE = 32, "rotate";
    ALIGN_TEXT = 33, "align-text";
    FIT = 34, "fit";
    SRC = 35, "src";
    D = 36, "d";
    COLS = 37, "cols";
    SPAN = 38, "span";
    CONTENT = 39, "content";
    FLAGS = 40, "flags";
    ACT = 41, "act";
    FIELD = 42, "field";
    EACH = 43, "each";
    KEYS = 44, "keys";
    SCROLLBAR = 45, "scrollbar";
    SCROLLBAR_W = 46, "scrollbar-w";
    SCROLLBAR_FG = 47, "scrollbar-fg";
    SCROLLBAR_BG = 48, "scrollbar-bg";
    SUBMIT = 49, "submit";
    ITEM_EXTENT = 50, "item-extent";
    OVERSCAN = 51, "overscan";
    ATTACH = 52, "attach";
    GRAVITY = 53, "gravity";
    COLLIDE = 54, "collide";
    PRESS = 55, "press";
    CONTEXT = 56, "context";
    DBLCLICK = 57, "dblclick";
    DRAG = 58, "drag";
    DROP = 59, "drop";
    RESIZE = 60, "resize";
    ROLE = 61, "role";
    LABEL = 62, "label";
    DESC = 63, "desc";
    SCALE = 64, "scale";
    SMOOTH = 65, "smooth";
    GRAIN = 66, "grain";
    MASK = 67, "mask";
    BACKDROP_MASK = 68, "backdrop-mask";
    TILT = 69, "tilt";
    POINTER_MOVE = 70, "pointer-move";
    POINTER_UP = 71, "pointer-up";
    DRAG_UPDATE = 72, "drag-update";
    DRAG_END = 73, "drag-end";
    CHECKED = 74, "checked";
    EXPANDED = 75, "expanded";
    SELECTED = 76, "selected";
    ACTIVE_DESCENDANT = 77, "active-descendant";
    CONTROLS = 78, "controls";
    VALUE_NOW = 79, "value-now";
    VALUE_MIN = 80, "value-min";
    VALUE_MAX = 81, "value-max";
    VALUE_TEXT = 82, "value-text";
    MODAL = 83, "modal";
    LIVE = 84, "live";
    LIVE_ATOMIC = 85, "live-atomic";
    LEVEL = 86, "level";
    POS_IN_SET = 87, "pos-in-set";
    SET_SIZE = 88, "set-size";
}
