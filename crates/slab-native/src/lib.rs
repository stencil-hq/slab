//! slab-native — winit + wgpu driver over the hand-maintained Rust kernel (P7).
//!
//! The kernel owns layout, hit testing, focus, editing, motion and scroll;
//! this crate only translates winit events into kernel `Event`s, paints
//! `FrameOp`s through instanced wgpu pipelines, and surfaces `Effects`
//! (signals, caret/IME rects, cursor). The renderer is window-independent:
//! tests and `--headless-frame` render to a texture and read pixels back.

use std::time::{Duration, Instant};
use winit::event::MouseButton;
use winit::keyboard::{Key, NamedKey};
use winit::window::CursorIcon;

pub mod atlas;
pub mod demo;
pub mod gen_player;
pub mod gen_settings;
pub mod holes;
pub mod player;
pub mod renderer;
pub mod surface;
pub mod tess;
pub mod view;

const MULTI_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const MULTI_CLICK_DISTANCE_SQ: f64 = 4.0 * 4.0;

struct Click {
    at: Instant,
    x: f64,
    y: f64,
    button: u32,
    count: u32,
}

/// Counts consecutive native pointer-downs in document coordinates.
#[derive(Default)]
pub(crate) struct ClickCounter {
    last: Option<Click>,
}

impl ClickCounter {
    /// Records a pointer-down and returns its host-clock click count.
    pub(crate) fn pointer_down(&mut self, button: u32, x: f64, y: f64) -> u32 {
        self.click_at(Instant::now(), button, x, y)
    }

    fn click_at(&mut self, now: Instant, button: u32, x: f64, y: f64) -> u32 {
        let count = match &self.last {
            Some(last) => {
                let dx = x - last.x;
                let dy = y - last.y;
                if last.button == button
                    && now.duration_since(last.at) <= MULTI_CLICK_INTERVAL
                    && dx * dx + dy * dy <= MULTI_CLICK_DISTANCE_SQ
                {
                    last.count.saturating_add(1)
                } else {
                    1
                }
            }
            None => 1,
        };
        self.last = Some(Click {
            at: now,
            x,
            y,
            button,
            count,
        });
        count
    }
}

/// Returns a stable kernel button id for every winit mouse button.
pub(crate) fn mouse_button_id(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::Back => 3,
        MouseButton::Forward => 4,
        MouseButton::Other(button) => 5 + u32::from(button),
    }
}

/// Records a cursor sample and returns its document-space motion delta.
pub(crate) fn cursor_delta(previous: &mut Option<(f64, f64)>, current: (f64, f64)) -> (f64, f64) {
    let delta = previous.map_or((0.0, 0.0), |(x, y)| (current.0 - x, current.1 - y));
    *previous = Some(current);
    delta
}

/// Maps a kernel cursor effect to its native window cursor.
pub(crate) fn cursor_icon(cursor: u32) -> CursorIcon {
    match cursor {
        slab_kernel::dispatch::CUR_POINTER => CursorIcon::Pointer,
        slab_kernel::dispatch::CUR_TEXT => CursorIcon::Text,
        slab_kernel::dispatch::CUR_COL_RESIZE => CursorIcon::ColResize,
        slab_kernel::dispatch::CUR_ROW_RESIZE => CursorIcon::RowResize,
        _ => CursorIcon::Default,
    }
}

/// A decoded native document together with the image payloads that are kept
/// out of the kernel document and runtime-provided font faces.
pub struct NativeDocument {
    pub inst: slab_kernel::frame::Instance,
    pub imgs: Vec<Vec<u8>>,
    fonts: Vec<RegisteredFont>,
}

/// A face registered by the host. The original bytes are retained so each
/// renderer can construct an atlas face without consulting the SLIR payload.
pub struct RegisteredFont {
    pub name: String,
    pub weight: u32,
    pub bytes: Vec<u8>,
}

impl NativeDocument {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let (inst, imgs) = slab_slir::instance(bytes)?;
        Ok(Self {
            inst,
            imgs,
            fonts: Vec::new(),
        })
    }

    /// Registers a face for both kernel measurement and native glyph painting.
    /// Returns false when `bytes` is not a supported font.
    pub fn register_font(&mut self, name: &str, bytes: Vec<u8>) -> bool {
        let Some(metrics) = slab_fonts::parse_metrics(&bytes) else {
            return false;
        };
        let gids = metrics.gids;
        let advances = metrics.advances;
        let weight = u32::from(metrics.weight);
        slab_kernel::frame::inst_font_register(
            &mut self.inst,
            name,
            weight,
            u32::from(metrics.upem),
            i32::from(metrics.ascent),
            i32::from(metrics.descent),
            i32::from(metrics.line_gap),
            u32::from(metrics.default_advance),
            &metrics.cps,
            &gids,
            &advances,
        );
        self.fonts.push(RegisteredFont {
            name: name.to_owned(),
            weight,
            bytes,
        });
        true
    }

    pub fn registered_fonts(&self) -> &[RegisteredFont] {
        &self.fonts
    }
}

/// Native runtime owner for one document and its renderer resources.
///
/// This is the public registration path: a face is appended to the kernel's
/// metric tables and the renderer immediately refreshes atlas resources for
/// the appended FONT table.
pub struct NativeDriver {
    pub document: NativeDocument,
    pub renderer: renderer::Renderer,
    doc_id: Option<usize>,
}

impl NativeDriver {
    pub fn new(document: NativeDocument, renderer: renderer::Renderer) -> Self {
        Self {
            document,
            renderer,
            doc_id: None,
        }
    }

    pub fn register_document(&mut self) -> usize {
        let doc_id = self.renderer.register_doc(
            &self.document.inst.doc,
            &self.document.imgs,
            self.document.registered_fonts(),
        );
        self.doc_id = Some(doc_id);
        doc_id
    }

    /// Registers a face for layout and painting, invalidating the glyph atlas
    /// entries associated with the newly appended FONT table.
    pub fn register_font(&mut self, name: &str, bytes: Vec<u8>) -> bool {
        let first_font = self.document.inst.doc.font_upem.len();
        if !self.document.register_font(name, bytes) {
            return false;
        }
        if let Some(doc_id) = self.doc_id {
            self.renderer.refresh_registered_fonts(
                doc_id,
                &self.document.inst.doc,
                self.document.registered_fonts(),
                first_font,
            );
        }
        true
    }
}

/// Request a device/queue. `surface` narrows adapter selection for windowed
/// use; `None` is the headless path (Metal supports surfaceless adapters).
pub fn request_device(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'_>>,
) -> Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        force_fallback_adapter: false,
        compatible_surface: surface,
        apply_limit_buckets: false,
    }))
    .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("slab-native"),
        ..Default::default()
    }))
    .ok()?;
    Some((adapter, device, queue))
}

/// Normalizes winit keyboard keys to the kernel event vocabulary.
pub(crate) fn key_name(key: &Key) -> Option<String> {
    let name = match key {
        Key::Named(named) => match named {
            NamedKey::Enter => "Enter",
            NamedKey::Tab => "Tab",
            NamedKey::Space => " ",
            NamedKey::Escape => "Escape",
            NamedKey::Backspace => "Backspace",
            NamedKey::Delete => "Delete",
            NamedKey::Insert => "Insert",
            NamedKey::Home => "Home",
            NamedKey::End => "End",
            NamedKey::PageUp => "PageUp",
            NamedKey::PageDown => "PageDown",
            NamedKey::ArrowLeft => "ArrowLeft",
            NamedKey::ArrowRight => "ArrowRight",
            NamedKey::ArrowUp => "ArrowUp",
            NamedKey::ArrowDown => "ArrowDown",
            NamedKey::F1 => "F1",
            NamedKey::F2 => "F2",
            NamedKey::F3 => "F3",
            NamedKey::F4 => "F4",
            NamedKey::F5 => "F5",
            NamedKey::F6 => "F6",
            NamedKey::F7 => "F7",
            NamedKey::F8 => "F8",
            NamedKey::F9 => "F9",
            NamedKey::F10 => "F10",
            NamedKey::F11 => "F11",
            NamedKey::F12 => "F12",
            NamedKey::F13 => "F13",
            NamedKey::F14 => "F14",
            NamedKey::F15 => "F15",
            NamedKey::F16 => "F16",
            NamedKey::F17 => "F17",
            NamedKey::F18 => "F18",
            NamedKey::F19 => "F19",
            NamedKey::F20 => "F20",
            NamedKey::F21 => "F21",
            NamedKey::F22 => "F22",
            NamedKey::F23 => "F23",
            NamedKey::F24 => "F24",
            _ => return None,
        },
        Key::Character(character) => return Some(character.to_string()),
        _ => return None,
    };
    Some(name.to_string())
}

#[cfg(test)]
mod native_input_tests {
    use super::{ClickCounter, cursor_delta, mouse_button_id};
    use std::time::{Duration, Instant};
    use winit::event::MouseButton;

    #[test]
    fn cursor_motion_requires_a_prior_sample() {
        let mut previous = None;

        assert_eq!(cursor_delta(&mut previous, (10.0, 20.0)), (0.0, 0.0));
        assert_eq!(cursor_delta(&mut previous, (13.0, 18.0)), (3.0, -2.0));
        previous = None;
        assert_eq!(cursor_delta(&mut previous, (50.0, 60.0)), (0.0, 0.0));
    }

    #[test]
    fn mouse_button_ids_are_stable_and_disjoint() {
        assert_eq!(mouse_button_id(MouseButton::Left), 0);
        assert_eq!(mouse_button_id(MouseButton::Middle), 1);
        assert_eq!(mouse_button_id(MouseButton::Right), 2);
        assert_eq!(mouse_button_id(MouseButton::Back), 3);
        assert_eq!(mouse_button_id(MouseButton::Forward), 4);
        assert_eq!(mouse_button_id(MouseButton::Other(0)), 5);
        assert_eq!(
            mouse_button_id(MouseButton::Other(u16::MAX)),
            5 + u32::from(u16::MAX)
        );
    }

    #[test]
    fn first_click_starts_at_one_and_matching_click_increments() {
        let start = Instant::now();
        let mut clicks = ClickCounter::default();

        assert_eq!(clicks.click_at(start, 0, 10.0, 20.0), 1);
        assert_eq!(
            clicks.click_at(start + Duration::from_millis(100), 0, 10.0, 20.0),
            2
        );
    }

    #[test]
    fn click_after_interval_resets() {
        let start = Instant::now();
        let mut clicks = ClickCounter::default();

        assert_eq!(clicks.click_at(start, 0, 10.0, 20.0), 1);
        assert_eq!(
            clicks.click_at(start + Duration::from_millis(501), 0, 10.0, 20.0),
            1
        );
    }

    #[test]
    fn click_beyond_distance_resets() {
        let start = Instant::now();
        let mut clicks = ClickCounter::default();

        assert_eq!(clicks.click_at(start, 0, 10.0, 20.0), 1);
        assert_eq!(
            clicks.click_at(start + Duration::from_millis(100), 0, 13.0, 24.0),
            1
        );
    }

    #[test]
    fn click_with_different_button_resets() {
        let start = Instant::now();
        let mut clicks = ClickCounter::default();

        assert_eq!(clicks.click_at(start, 0, 10.0, 20.0), 1);
        assert_eq!(
            clicks.click_at(start + Duration::from_millis(100), 2, 10.0, 20.0),
            1
        );
    }
}
