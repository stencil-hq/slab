//! A8 glyph atlas.
//!
//! Rasterized from runtime-registered or bundled fallback faces. Glyph ids and
//! pen positions come EXCLUSIVELY from kernel `text_glyphs`; this module only
//! turns (font, gid, px) into a shelf-packed alpha bitmap.

use std::collections::HashMap;

use ab_glyph::{Font, FontVec, GlyphId, point};

/// Atlas texture dimension (square, `R8Unorm`).
pub const ATLAS: u32 = 2048;

/// One cached glyph bitmap: normalized uv rect, device-px quad size, bearing
/// from the baseline pen position (device px).
#[derive(Clone, Copy)]
pub struct GlyphEntry {
	pub uv:      [f32; 4],
	pub size:    [f32; 2],
	pub bearing: [f32; 2],
}

/// A face provided by the host or bundled fallback assets.
pub struct Face {
	font:   FontVec,
	upem:   f32,
	height: f32,
}

impl Face {
	pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
		let font = FontVec::try_from_vec(bytes.to_vec()).ok()?;
		let upem = font.units_per_em().unwrap_or(1000.0);
		let height = font.height_unscaled();
		Some(Self { font, upem, height })
	}
}

/// Shelf-packed alpha atlas shared by every registered document.
/// Cache key: (doc id, FONT table index, glyph id, quarter-px quantized size).
pub struct Atlas {
	pub pixels: Vec<u8>,
	pub dirty:  bool,
	cache:      HashMap<(usize, i32, u32, u32), Option<GlyphEntry>>,
	pen:        (u32, u32),
	shelf_h:    u32,
	full_noted: bool,
}

impl Default for Atlas {
	fn default() -> Self {
		Self {
			pixels:     vec![0; (ATLAS * ATLAS) as usize],
			dirty:      false,
			cache:      HashMap::new(),
			pen:        (0, 0),
			shelf_h:    0,
			full_noted: false,
		}
	}
}

impl Atlas {
	/// Rasterize (or fetch) glyph `gid` of `face` at `px` device pixels.
	/// `None` for empty outlines (spaces) or when the atlas is full.
	pub fn entry(
		&mut self,
		doc: usize,
		font_ix: i32,
		face: &Face,
		gid: u32,
		px: f32,
	) -> Option<GlyphEntry> {
		let key = (doc, font_ix, gid, (px * 4.0).round() as u32);
		if let Some(e) = self.cache.get(&key) {
			return *e;
		}
		// PxScale is relative to the font's unscaled height; convert so `px`
		// means the typographic font size in device pixels.
		let scale = ab_glyph::PxScale::from(px * face.height / face.upem);
		let glyph = GlyphId(gid as u16).with_scale_and_position(scale, point(0.0, 0.0));
		let mut entry = None;
		if let Some(og) = face.font.outline_glyph(glyph) {
			let b = og.px_bounds();
			let gw = b.width().ceil() as u32 + 1;
			let gh = b.height().ceil() as u32 + 1;
			if let Some((ax, ay)) = self.alloc(gw, gh) {
				let pixels = &mut self.pixels;
				og.draw(|x, y, c| {
					let (px_, py_) = (ax + x, ay + y);
					if px_ < ATLAS && py_ < ATLAS {
						let i = (py_ * ATLAS + px_) as usize;
						let v = (c * 255.0) as u8;
						if v > pixels[i] {
							pixels[i] = v;
						}
					}
				});
				self.dirty = true;
				let a = ATLAS as f32;
				entry = Some(GlyphEntry {
					uv:      [ax as f32 / a, ay as f32 / a, gw as f32 / a, gh as f32 / a],
					size:    [gw as f32, gh as f32],
					bearing: [b.min.x, b.min.y],
				});
			} else if !self.full_noted {
				self.full_noted = true;
				eprintln!("slab-native: cap-atlas: glyph atlas full; further new glyphs skipped");
			}
		}
		self.cache.insert(key, entry);
		entry
	}

	/// Forget glyphs for changed FONT tables. Existing atlas pixels are
	/// harmless; new entries are allocated and replace their UVs in frames.
	pub fn invalidate_doc_fonts(&mut self, doc: usize, first_font: i32) {
		self
			.cache
			.retain(|(cached_doc, font, ..), _| *cached_doc != doc || *font < first_font);
	}

	fn alloc(&mut self, gw: u32, gh: u32) -> Option<(u32, u32)> {
		if gw + 1 > ATLAS || gh + 1 > ATLAS {
			return None;
		}
		if self.pen.0 + gw + 1 > ATLAS {
			self.pen.0 = 0;
			self.pen.1 += self.shelf_h + 1;
			self.shelf_h = 0;
		}
		if self.pen.1 + gh + 1 > ATLAS {
			return None;
		}
		let pos = self.pen;
		self.pen.0 += gw + 1;
		self.shelf_h = self.shelf_h.max(gh);
		Some(pos)
	}
}
