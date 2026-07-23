//! Kitty-graphics image painting: where the terminal speaks the kitty APC
//! protocol (kitty, ghostty, WezTerm), each document image is transmitted
//! once and placed over its Image op's cell rect, covering the shaded
//! placeholder the cell grid still paints. Terminals without support (or
//! `--images off`) simply keep the placeholder; iTerm2 uses its own OSC
//! 1337 protocol and is treated as unsupported.

use slab_kernel::{cells, flatten, slir};
use std::path::Path;

/// Host image-painting mode from `--images` (default `auto`).
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Auto,
    On,
    Off,
}

impl Mode {
    /// Parses the `--images` flag value.
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "auto" => Some(Mode::Auto),
            "on" => Some(Mode::On),
            "off" => Some(Mode::Off),
            _ => None,
        }
    }
}

/// Reports whether the running terminal is on the kitty-APC allowlist.
fn terminal_supported() -> bool {
    let term = std::env::var("TERM").unwrap_or_default();
    if term.contains("kitty") {
        return true;
    }
    matches!(
        std::env::var("TERM_PROGRAM").as_deref(),
        Ok("ghostty") | Ok("WezTerm")
    )
}

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

/// One retained placement: (image index, col, row, cols, rows).
type Placement = (i32, i32, i32, i32, i32);

/// Kitty-graphics painter with one placement per visible Image op.
pub struct Images {
    enabled: bool,
    /// PNG bytes per document image; empty = unavailable, placeholder stays.
    data: Vec<Vec<u8>>,
    transmitted: Vec<bool>,
    placed: Vec<Placement>,
}

impl Images {
    /// Resolves image bytes for a decoded document: embedded SLIR payloads
    /// first, then the `src` path against the document's directory. Missing
    /// or non-PNG data stays empty so the cell placeholder shows instead.
    pub fn new(mode: Mode, doc: &slir::Doc, embedded: &[Vec<u8>], base_dir: &Path) -> Images {
        let enabled = match mode {
            Mode::On => true,
            Mode::Off => false,
            Mode::Auto => terminal_supported(),
        };
        let mut data = Vec::with_capacity(doc.img_src.len());
        for (i, &src) in doc.img_src.iter().enumerate() {
            let mut bytes = embedded.get(i).cloned().unwrap_or_default();
            if bytes.is_empty() && enabled {
                let src = &doc.strs[usize::try_from(src).expect("string ref exceeds usize")];
                if !src.is_empty() {
                    bytes = std::fs::read(base_dir.join(src)).unwrap_or_default();
                }
            }
            if !bytes.starts_with(&PNG_MAGIC) {
                bytes = Vec::new();
            }
            data.push(bytes);
        }
        let transmitted = vec![false; data.len()];
        Images {
            enabled,
            data,
            transmitted,
            placed: Vec::new(),
        }
    }

    /// Appends the escape sequences for this frame's Image ops to `buf`.
    ///
    /// Placements are compared against the previous frame and rewritten only
    /// on change (or `force`, after a full repaint cleared the screen).
    /// Returns `true` when anything was written (the cursor moved).
    pub fn paint(&mut self, buf: &mut String, fr: &flatten::Frame, force: bool) -> bool {
        if !self.enabled {
            return false;
        }
        let mut desired: Vec<Placement> = Vec::new();
        for op in &fr.ops {
            let flatten::FrameOp::Image(image) = op else {
                continue;
            };
            let Ok(index) = usize::try_from(image.img) else {
                continue;
            };
            if self.data.get(index).is_none_or(Vec::is_empty) {
                continue;
            }
            let col = cells::cell_col(image.x);
            let row = cells::cell_row(image.y);
            let cols = cells::cell_col(image.x + image.w) - col;
            let rows = cells::cell_row(image.y + image.h) - row;
            if cols <= 0 || rows <= 0 || col < 0 || row < 0 {
                continue;
            }
            desired.push((image.img, col, row, cols, rows));
        }
        if !force && desired == self.placed {
            return false;
        }
        let mut wrote = false;
        // Every re-place starts clean: drop all previous placements (each
        // `a=p` without a placement id creates an additional one).
        let mut dropped: Vec<i32> = Vec::new();
        for &(img, ..) in &self.placed {
            if !dropped.contains(&img) {
                buf.push_str(&format!("\x1b_Gq=2,a=d,d=i,i={}\x1b\\", img + 1));
                dropped.push(img);
                wrote = true;
            }
        }
        for &(img, col, row, cols, rows) in &desired {
            let index = usize::try_from(img).expect("checked above");
            if !self.transmitted[index] {
                transmit(buf, img + 1, &self.data[index]);
                self.transmitted[index] = true;
            }
            // Kitty places at the cursor: move there, then anchor the
            // placement scaled into the op's cell rect.
            buf.push_str(&format!(
                "\x1b[{};{}H\x1b_Gq=2,a=p,i={},c={cols},r={rows}\x1b\\",
                row + 1,
                col + 1,
                img + 1,
            ));
            wrote = true;
        }
        self.placed = desired;
        wrote
    }

    /// Drop every placement and transmitted image (gallery switch: the next
    /// document reuses the same ids for different pictures).
    pub fn clear(&self, buf: &mut String) {
        if self.enabled && !self.placed.is_empty() {
            buf.push_str("\x1b_Gq=2,a=d,d=A\x1b\\");
        }
    }
}

/// Transmits PNG bytes as chunked base64 (`a=t`, format 100).
fn transmit(buf: &mut String, id: i32, bytes: &[u8]) {
    let encoded = base64(bytes);
    let mut chunks = encoded.as_bytes().chunks(4096).peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = u8::from(chunks.peek().is_some());
        if first {
            buf.push_str(&format!("\x1b_Gq=2,f=100,a=t,i={id},m={more};"));
            first = false;
        } else {
            buf.push_str(&format!("\x1b_Gq=2,m={more};"));
        }
        buf.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
        buf.push_str("\x1b\\");
    }
}

/// Standard base64 (RFC 4648, with padding); avoids an external dependency.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let word = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(char::from(
            TABLE[usize::try_from(word >> 18).expect("6-bit index")],
        ));
        out.push(char::from(
            TABLE[usize::try_from((word >> 12) & 63).expect("6-bit index")],
        ));
        out.push(if chunk.len() > 1 {
            char::from(TABLE[usize::try_from((word >> 6) & 63).expect("6-bit index")])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(TABLE[usize::try_from(word & 63).expect("6-bit index")])
        } else {
            '='
        });
    }
    out
}
