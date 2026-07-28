//! Embeddable terminal host for Slab kernel instances.

mod app;
pub mod cli;
mod images;
mod interactive;
mod player;
mod script;

/// Crossterm types accepted by [`translate`].
pub use crossterm;

pub use app::{Host, Signal};
pub use images::{Images, Mode as ImageMode};
pub use interactive::{
    ClickTracker, Exit, Gallery, Painter, Terminal, Translated, Translator, Ui, resize, run,
    terminal_env, translate,
};
