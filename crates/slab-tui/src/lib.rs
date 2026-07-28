//! Embeddable terminal host for Slab kernel instances.

mod app;
pub mod cli;
mod images;
mod interactive;
mod player;
mod script;

pub use app::{
	E_CLOSE, E_KEY_DOWN, E_PASTE, E_POINTER_DOWN, E_POINTER_MOVE, E_POINTER_UP, E_TEXT, E_WHEEL,
	Host, HostKey, KeyHandling, M_ALT, M_CTRL, M_META, M_SHIFT, Signal, collect_signals, compile,
	host_key, instance, key_event, paste_event, pointer_button_event, pointer_event, text_event,
	wheel_event,
};
/// Crossterm types accepted by [`translate`].
pub use crossterm;
pub use images::{Images, Mode as ImageMode};
pub use interactive::{
	ClickTracker, Exit, Gallery, Painter, Terminal, Translated, Translator, Ui, resize, run,
	terminal_env, translate,
};
