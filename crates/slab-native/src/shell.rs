//! Reusable native application shell.
//!
//! The shell owns winit window creation, wgpu surface loss/reconfiguration,
//! pointer/keyboard/wheel/IME translation, click counting, clipboard editing,
//! dirty and motion redraw scheduling, suspend/resume, occlusion and AccessKit.
//! Hosts provide only document/model signal policy through [`ShellHost`] and
//! optional application user events through [`ShellEvent::User`]. This keeps
//! layout and interaction in the shared kernel.
//!
//! Use `EventLoop::<ShellEvent<MyEvent>>::with_user_event()`, construct a
//! [`NativeShell`] with the loop's proxy, and call `run_app`. Accessibility
//! events and application events then share the same winit user-event type.
//! A background SDP `RequestPump` can wake the loop by sending a host event;
//! its [`ShellHost::user_event`] implementation drains requests against the
//! supplied [`NativeDocument`](crate::NativeDocument) and returns `true`.

pub use winit;

pub use crate::view::{DefaultShellHost, NativeShell, ShellEvent, ShellHost, ShellOptions};
