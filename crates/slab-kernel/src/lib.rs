//! The deterministic layout, interaction, editing, and rendering kernel shared
//! by every slab runtime.
//!
//! # Maintenance contract
//!
//! Kernel output must remain byte-identical across native and WebAssembly
//! builds:
//! - integer arithmetic uses 32-bit two's-complement wrapping operations;
//! - all model math uses `f64`;
//! - unordered maps are never iterated when their order can affect output;
//! - `sin`, `cos`, `powf`, and `cbrt` results pass through the existing
//!   domain-level quantization before they affect frame output; and
//! - float-to-string formatting stays outside the kernel.

pub mod capability;
pub mod caps;
pub mod cells;
pub mod color;
pub mod dispatch;
pub mod dumpjson;
pub mod ease;
pub mod edit;
pub mod flatten;
pub mod focus;
pub mod frame;
pub mod frame_buf;
pub mod graphemes;
pub mod hit;
pub mod layout;
pub mod list;
pub mod motion;
pub mod pathdata;
pub mod rt;
pub mod scene;
pub mod slir;
pub mod squircle;
pub mod style;
pub mod test_cells;
pub mod test_color;
pub mod test_divider;
pub mod test_ease;
pub mod test_edit;
pub mod test_fmt3;
pub mod test_font_register;
pub mod test_gesture;
pub mod test_graphemes;
pub mod test_hit;
pub mod test_layout;
pub mod test_list;
pub mod test_motion;
pub mod test_multiline;
pub mod test_textm;
pub mod test_value;
pub mod test_when;
pub mod textm;
pub mod value;
pub mod when;
