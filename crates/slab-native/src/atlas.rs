//! Hinted mask and color glyph atlases for the native renderer.
//!
//! The kernel owns shaping and device-independent pen positions. This module
//! uses Swash to rasterize those glyph ids with quarter-pixel positioning,
//! keeps mask and RGBA content separate, and grows or evicts atlas entries
//! without changing the kernel's text contract.

use std::collections::HashSet;

use etagere::{AllocId, BucketedAtlasAllocator, size2};
use lru::LruCache;
use rustc_hash::FxBuildHasher;
use swash::{
	FontRef,
	scale::{Render, ScaleContext, Source, StrikeWith, image::Content},
	zeno::{Format, Vector},
};

const SUBPIXEL_BINS: u32 = 4;
const MASK_INITIAL_SIZE: u32 = 512;
const COLOR_INITIAL_SIZE: u32 = 256;
const ATLAS_LIMIT: u32 = 4096;
const GLYPH_GUTTER: u32 = 1;

/// Selects the mask or RGBA glyph texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AtlasKind {
	Mask,
	Color,
}

impl AtlasKind {
	pub(crate) const fn channels(self) -> usize {
		match self {
			Self::Mask => 1,
			Self::Color => 4,
		}
	}
}

/// One cached glyph bitmap in device-pixel atlas coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GlyphEntry {
	/// Atlas x/y followed by width/height, all in device pixels.
	pub uv:      [f32; 4],
	pub size:    [f32; 2],
	/// Bitmap offset from the integer baseline pen position.
	pub bearing: [f32; 2],
	pub kind:    AtlasKind,
}

/// A host-provided or bundled face retained for Swash rasterization.
pub(crate) struct Face {
	bytes: Vec<u8>,
}

impl Face {
	pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
		FontRef::from_index(bytes, 0)?;
		Some(Self { bytes: bytes.to_vec() })
	}

	fn as_swash(&self) -> Option<FontRef<'_>> {
		FontRef::from_index(&self.bytes, 0)
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GlyphKey {
	doc:         usize,
	font:        i32,
	gid:         u32,
	px_quarters: u32,
	x_bin:       u8,
	y_bin:       u8,
}

#[derive(Clone, Copy)]
struct CachedGlyph {
	entry:      Option<GlyphEntry>,
	allocation: Option<AllocId>,
}

struct AtlasLayer {
	kind:     AtlasKind,
	size:     u32,
	max_size: u32,
	pixels:   Vec<u8>,
	packer:   BucketedAtlasAllocator,
	cache:    LruCache<GlyphKey, CachedGlyph, FxBuildHasher>,
	in_use:   HashSet<GlyphKey, FxBuildHasher>,
	/// Dirty row range `[first, end)` in the CPU image.
	dirty:    Option<(u32, u32)>,
}

impl AtlasLayer {
	fn new(kind: AtlasKind, initial_size: u32, max_size: u32) -> Self {
		let size = initial_size.min(max_size);
		Self {
			kind,
			size,
			max_size,
			pixels: vec![0; size as usize * size as usize * kind.channels()],
			packer: BucketedAtlasAllocator::new(size2(size as i32, size as i32)),
			cache: LruCache::unbounded_with_hasher(FxBuildHasher),
			in_use: HashSet::with_hasher(FxBuildHasher),
			dirty: None,
		}
	}

	fn begin_frame(&mut self) {
		self.in_use.clear();
	}

	/// Outer option distinguishes a cache miss from a cached empty glyph.
	fn get(&mut self, key: GlyphKey) -> Option<Option<GlyphEntry>> {
		let cached = self.cache.get(&key).copied()?;
		self.in_use.insert(key);
		Some(cached.entry)
	}

	fn cache_empty(&mut self, key: GlyphKey) {
		self
			.cache
			.put(key, CachedGlyph { entry: None, allocation: None });
		self.in_use.insert(key);
	}

	fn insert(
		&mut self,
		key: GlyphKey,
		data: &[u8],
		width: u32,
		height: u32,
		bearing: [f32; 2],
	) -> Option<GlyphEntry> {
		if width == 0 || height == 0 {
			self.cache_empty(key);
			return None;
		}
		let padded_width = width + GLYPH_GUTTER * 2;
		let padded_height = height + GLYPH_GUTTER * 2;
		let allocation = loop {
			if let Some(allocation) = self
				.packer
				.allocate(size2(padded_width as i32, padded_height as i32))
			{
				break allocation;
			}
			if self.evict_one() {
				continue;
			}
			if !self.grow() {
				return None;
			}
		};

		let rect = allocation.rectangle;
		let x = rect.min.x as u32 + GLYPH_GUTTER;
		let y = rect.min.y as u32 + GLYPH_GUTTER;
		self.clear_rect(
			rect.min.x as u32,
			rect.min.y as u32,
			rect.width() as u32,
			rect.height() as u32,
		);
		self.copy_image(x, y, width, height, data);
		let entry = GlyphEntry {
			uv: [x as f32, y as f32, width as f32, height as f32],
			size: [width as f32, height as f32],
			bearing,
			kind: self.kind,
		};
		self
			.cache
			.put(key, CachedGlyph { entry: Some(entry), allocation: Some(allocation.id) });
		self.in_use.insert(key);
		Some(entry)
	}

	fn evict_one(&mut self) -> bool {
		loop {
			let Some((&key, &cached)) = self.cache.peek_lru() else {
				return false;
			};
			if cached.allocation.is_some() && self.in_use.contains(&key) {
				return false;
			}
			let (_, cached) = self.cache.pop_lru().expect("peeked glyph cache entry");
			self.in_use.remove(&key);
			if let Some(id) = cached.allocation {
				self.packer.deallocate(id);
				return true;
			}
		}
	}

	fn grow(&mut self) -> bool {
		if self.size >= self.max_size {
			return false;
		}
		let old_size = self.size;
		let next_size = (old_size * 2).min(self.max_size);
		let channels = self.kind.channels();
		let mut next = vec![0; next_size as usize * next_size as usize * channels];
		let old_stride = old_size as usize * channels;
		let next_stride = next_size as usize * channels;
		for row in 0..old_size as usize {
			let old = row * old_stride..(row + 1) * old_stride;
			let new = row * next_stride..row * next_stride + old_stride;
			next[new].copy_from_slice(&self.pixels[old]);
		}
		self.pixels = next;
		self.size = next_size;
		self.packer.grow(size2(next_size as i32, next_size as i32));
		// The renderer recreates a resized GPU texture, so every preserved row
		// must be uploaded again.
		self.dirty = Some((0, next_size));
		true
	}

	fn clear_rect(&mut self, x: u32, y: u32, width: u32, height: u32) {
		let channels = self.kind.channels();
		let stride = self.size as usize * channels;
		let from = x as usize * channels;
		let len = width as usize * channels;
		for row in y as usize..(y + height) as usize {
			self.pixels[row * stride + from..row * stride + from + len].fill(0);
		}
		self.mark_dirty(y, height);
	}

	fn copy_image(&mut self, x: u32, y: u32, width: u32, height: u32, data: &[u8]) {
		let channels = self.kind.channels();
		let src_stride = width as usize * channels;
		let dst_stride = self.size as usize * channels;
		let dst_x = x as usize * channels;
		for row in 0..height as usize {
			let src = row * src_stride..(row + 1) * src_stride;
			let dst = (y as usize + row) * dst_stride + dst_x;
			self.pixels[dst..dst + src_stride].copy_from_slice(&data[src]);
		}
		self.mark_dirty(y, height);
	}

	fn mark_dirty(&mut self, y: u32, rows: u32) {
		let end = (y + rows).min(self.size);
		self.dirty = Some(match self.dirty {
			Some((first, old_end)) => (first.min(y), old_end.max(end)),
			None => (y, end),
		});
	}

	fn take_dirty(&mut self) -> Option<(u32, u32)> {
		let (first, end) = self.dirty.take()?;
		Some((first, end - first))
	}

	fn invalidate_doc_fonts(&mut self, doc: usize, first_font: i32) {
		let keys = self
			.cache
			.iter()
			.filter_map(|(key, _)| (key.doc == doc && key.font >= first_font).then_some(*key))
			.collect::<Vec<_>>();
		for key in keys {
			if let Some(cached) = self.cache.pop(&key)
				&& let Some(id) = cached.allocation
			{
				self.packer.deallocate(id);
			}
			self.in_use.remove(&key);
		}
	}
}

/// Shared Swash rasterizer and independent mask/color atlas caches.
pub(crate) struct Atlas {
	context: ScaleContext,
	mask:    AtlasLayer,
	color:   AtlasLayer,
}

impl Atlas {
	pub(crate) fn new(max_texture_dimension: u32) -> Self {
		let max_size = max_texture_dimension
			.min(ATLAS_LIMIT)
			.max(COLOR_INITIAL_SIZE);
		Self {
			context: ScaleContext::new(),
			mask:    AtlasLayer::new(AtlasKind::Mask, MASK_INITIAL_SIZE, max_size),
			color:   AtlasLayer::new(AtlasKind::Color, COLOR_INITIAL_SIZE, max_size),
		}
	}

	pub(crate) fn begin_frame(&mut self) {
		self.mask.begin_frame();
		self.color.begin_frame();
	}

	/// Rasterizes or fetches one glyph at a quarter-pixel x/y offset.
	pub(crate) fn entry(
		&mut self,
		doc: usize,
		font: i32,
		face: &Face,
		gid: u32,
		px: f32,
		x_bin: u8,
		y_bin: u8,
	) -> Option<GlyphEntry> {
		let px_quarters = (px * SUBPIXEL_BINS as f32).round().max(1.0) as u32;
		let key = GlyphKey { doc, font, gid, px_quarters, x_bin, y_bin };
		if let Some(entry) = self.mask.get(key) {
			return entry;
		}
		if let Some(entry) = self.color.get(key) {
			return entry;
		}

		let Some(font_ref) = face.as_swash() else {
			self.mask.cache_empty(key);
			return None;
		};
		let glyph_id = u16::try_from(gid).ok()?;
		let size = px_quarters as f32 / SUBPIXEL_BINS as f32;
		let mut scaler = self.context.builder(font_ref).size(size).hint(true).build();
		let sources =
			[Source::ColorOutline(0), Source::ColorBitmap(StrikeWith::BestFit), Source::Outline];
		let image = Render::new(&sources)
			.format(Format::Alpha)
			.offset(Vector::new(
				x_bin as f32 / SUBPIXEL_BINS as f32,
				y_bin as f32 / SUBPIXEL_BINS as f32,
			))
			.render(&mut scaler, glyph_id);
		let Some(mut image) = image else {
			self.mask.cache_empty(key);
			return None;
		};
		let kind = match image.content {
			Content::Mask => AtlasKind::Mask,
			Content::Color => AtlasKind::Color,
			Content::SubpixelMask => {
				// Slab intentionally uses grayscale AA on modern high-DPI panels.
				// Collapse an unexpected LCD mask to one conservative coverage.
				image.data = image
					.data
					.as_chunks::<4>().0.iter()
					.map(|pixel| pixel[0].max(pixel[1]).max(pixel[2]))
					.collect();
				AtlasKind::Mask
			},
		};
		let width = image.placement.width;
		let height = image.placement.height;
		let bearing = [image.placement.left as f32, -(image.placement.top as f32)];
		self
			.layer_mut(kind)
			.insert(key, &image.data, width, height, bearing)
	}

	pub(crate) fn invalidate_doc_fonts(&mut self, doc: usize, first_font: i32) {
		self.mask.invalidate_doc_fonts(doc, first_font);
		self.color.invalidate_doc_fonts(doc, first_font);
	}

	pub(crate) const fn size(&self, kind: AtlasKind) -> u32 {
		self.layer(kind).size
	}

	pub(crate) fn pixels(&self, kind: AtlasKind) -> &[u8] {
		&self.layer(kind).pixels
	}

	pub(crate) fn take_dirty(&mut self, kind: AtlasKind) -> Option<(u32, u32)> {
		self.layer_mut(kind).take_dirty()
	}

	const fn layer(&self, kind: AtlasKind) -> &AtlasLayer {
		match kind {
			AtlasKind::Mask => &self.mask,
			AtlasKind::Color => &self.color,
		}
	}

	const fn layer_mut(&mut self, kind: AtlasKind) -> &mut AtlasLayer {
		match kind {
			AtlasKind::Mask => &mut self.mask,
			AtlasKind::Color => &mut self.color,
		}
	}
}

/// Splits a device-pixel coordinate into an integer quad origin and a
/// quarter-pixel rasterization bin.
pub(crate) fn subpixel(pen: f32) -> (f32, u8) {
	let base = pen.floor();
	let bin = ((pen - base) * SUBPIXEL_BINS as f32).round() as u32;
	if bin >= SUBPIXEL_BINS {
		(base + 1.0, 0)
	} else {
		(base, bin as u8)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn subpixel_bins_preserve_integer_origin_at_rounding_boundary() {
		assert_eq!(subpixel(12.12), (12.0, 0));
		assert_eq!(subpixel(12.26), (12.0, 1));
		assert_eq!(subpixel(12.74), (12.0, 3));
		assert_eq!(subpixel(12.99), (13.0, 0));
	}
}
