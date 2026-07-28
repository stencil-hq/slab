//! Import-free binary frame and resource packets for native GPU hosts.

use slab_compile::render::RegisteredFont;
use slab_kernel::{
	dispatch::Effects,
	flatten::{Frame, FrameOp},
	frame,
	frame::Instance,
	frame_buf::FrameBuf,
	scene, slir as kernel_slir,
};
use slab_slir::Slir;

pub const RESOURCE_GRADIENT: u32 = 0;
pub const RESOURCE_PATH: u32 = 1;
pub const RESOURCE_FONT: u32 = 2;
pub const RESOURCE_IMAGE: u32 = 3;
pub const RESOURCE_SHADOW: u32 = 4;

const FRAME_VERSION: u32 = 1;
const RESOURCE_VERSION: u32 = 1;

const SECTION_U32S: u32 = 1;
const SECTION_F64S: u32 = 2;
const SECTION_STRINGS: u32 = 3;
const SECTION_UNCOVERED: u32 = 4;
const SECTION_RUNTIME_PATHS: u32 = 5;
const SECTION_DIAGNOSTICS: u32 = 6;
const SECTION_RESOURCES: u32 = 7;
const SECTION_EFFECTS: u32 = 8;

#[derive(Clone, Copy, Eq, PartialEq)]
struct ResourceRef {
	kind:       u32,
	index:      u32,
	generation: u32,
}

pub fn frame_packet(inst: &Instance, frame: &Frame, effects: &Effects, document: u32) -> Vec<u8> {
	let encoded = FrameBuf::encode_ref(frame, inst.dirty, inst.ms.active);
	let resources = frame_resources(inst, frame);
	let mut writer = Writer::with_capacity(
		64 + encoded.u32s.len() * 4 + encoded.f64s.len() * 8 + encoded.strings.len() * 16,
	);
	writer.bytes(b"SLFR");
	writer.u32(FRAME_VERSION);
	writer.u32(u32::from(encoded.dirty) | (u32::from(encoded.motion_active) << 1));
	writer.u32(document);
	writer.u32(8);

	writer.section(SECTION_U32S, |writer| {
		writer.count(encoded.u32s.len());
		for value in encoded.u32s {
			writer.u32(value);
		}
	});
	writer.section(SECTION_F64S, |writer| {
		writer.count(encoded.f64s.len());
		for value in encoded.f64s {
			writer.f64(value);
		}
	});
	writer.section(SECTION_STRINGS, |writer| {
		writer.count(encoded.strings.len());
		for value in &encoded.strings {
			writer.string(value);
		}
	});
	writer.section(SECTION_UNCOVERED, |writer| {
		writer.count(encoded.uncovered.len());
		for value in encoded.uncovered {
			writer.u32(value);
		}
	});
	writer.section(SECTION_RUNTIME_PATHS, |writer| {
		writer.count(encoded.rt_paths.len());
		for path in &encoded.rt_paths {
			writer.count(path.verbs.len());
			writer.count(path.coords.len());
			writer.bytes(&path.verbs);
			for coord in &path.coords {
				writer.f64(*coord);
			}
		}
	});
	writer.section(SECTION_DIAGNOSTICS, |writer| {
		writer.count(encoded.diagnostics.len());
		for diagnostic in &encoded.diagnostics {
			writer.string(&diagnostic.code);
			writer.u32(diagnostic.line);
			writer.string(&diagnostic.msg);
		}
	});
	writer.section(SECTION_RESOURCES, |writer| {
		writer.count(resources.len());
		for resource in resources {
			writer.u32(resource.kind);
			writer.u32(resource.index);
			writer.u32(resource.generation);
		}
	});
	writer.section(SECTION_EFFECTS, |writer| write_effects(writer, inst, effects));

	writer.finish()
}

pub fn resource_packet(
	slir: &Slir,
	inst: &Instance,
	fonts: &[RegisteredFont],
	kind: u32,
	index: u32,
) -> Option<Vec<u8>> {
	let mut writer = Writer::with_capacity(256);
	writer.bytes(b"SLRS");
	writer.u32(RESOURCE_VERSION);
	writer.u32(kind);
	writer.u32(index);
	let generation_at = writer.len();
	writer.u32(0);

	let generation = match kind {
		RESOURCE_GRADIENT => {
			let gradient = slir.grads.get(usize::try_from(index).ok()?)?;
			writer.u32(u32::from(gradient.kind));
			writer.f64(gradient.angle);
			writer.count(gradient.stops.len());
			for (position, color) in &gradient.stops {
				writer.f64(*position);
				writer.u32(*color);
			}
			0
		},
		RESOURCE_PATH => {
			let path = slir.paths.get(usize::try_from(index).ok()?)?;
			writer.count(path.verbs.len());
			writer.count(path.coords.len());
			writer.bytes(&path.verbs);
			for coord in &path.coords {
				writer.f64(*coord);
			}
			0
		},
		RESOURCE_FONT => {
			let doc = inst.doc();
			let font = usize::try_from(index).ok()?;
			let bytes = slab_compile::render::face_bytes(doc, font, fonts)?;
			let family = doc
				.strs
				.get(doc.font_family[font] as usize)
				.map_or("", String::as_str);
			writer.u32(doc.font_class[font]);
			writer.u32(doc.font_weight[font]);
			writer.string(family);
			writer.count(bytes.len());
			writer.bytes(bytes);
			0
		},
		RESOURCE_IMAGE => {
			let image = i32::try_from(index).ok()?;
			let (width, height, format, generation) = frame::inst_img_info(inst, image)?;
			let bytes = frame::inst_img_bytes(inst, image);
			writer.u32(width);
			writer.u32(height);
			writer.u32(format);
			writer.count(bytes.len());
			writer.bytes(bytes);
			generation
		},
		RESOURCE_SHADOW => {
			let shadow = slir.shadows.get(usize::try_from(index).ok()?)?;
			writer.f64(shadow.x);
			writer.f64(shadow.y);
			writer.f64(shadow.blur);
			writer.f64(shadow.spread);
			writer.u32(shadow.rgba);
			writer.u32(u32::from(shadow.inset));
			0
		},
		_ => return None,
	};
	writer.patch_u32(generation_at, generation);
	Some(writer.finish())
}

fn frame_resources(inst: &Instance, frame: &Frame) -> Vec<ResourceRef> {
	let mut resources = Vec::new();
	for op in &frame.ops {
		match op {
			FrameOp::Rect(rect) => {
				paint_resource(&mut resources, rect.bg_kind, rect.bg);
				paint_resource(&mut resources, rect.stroke_kind, rect.stroke);
				for shadow in rect.shadow_off..rect.shadow_off.saturating_add(rect.shadow_len) {
					if let Ok(index) = u32::try_from(shadow) {
						push_resource(&mut resources, RESOURCE_SHADOW, index, 0);
					}
				}
			},
			FrameOp::Text(text) => {
				if let Ok(index) = u32::try_from(text.font) {
					push_resource(&mut resources, RESOURCE_FONT, index, 0);
				}
				paint_resource(&mut resources, text.color_kind, text.color);
			},
			FrameOp::Image(image) => {
				if let Ok(index) = u32::try_from(image.img)
					&& let Some((_, _, _, generation)) = frame::inst_img_info(inst, image.img)
				{
					push_resource(&mut resources, RESOURCE_IMAGE, index, generation);
				}
			},
			FrameOp::PathDraw(path) => {
				if let Ok(index) = u32::try_from(path.path) {
					push_resource(&mut resources, RESOURCE_PATH, index, 0);
				}
				paint_resource(&mut resources, path.bg_kind, path.bg);
				paint_resource(&mut resources, path.stroke_kind, path.stroke);
			},
			FrameOp::GroupPush(group) => {
				paint_resource(&mut resources, group.mask_kind, group.mask);
			},
			FrameOp::Backdrop(backdrop) => {
				paint_resource(&mut resources, backdrop.mask_kind, backdrop.mask);
			},
			FrameOp::ClipPush(_)
			| FrameOp::ClipPop
			| FrameOp::GroupPop
			| FrameOp::RotatePush(_)
			| FrameOp::RotatePop
			| FrameOp::ScalePush(_)
			| FrameOp::ScalePop
			| FrameOp::TiltPush(_)
			| FrameOp::TiltPop => {},
		}
	}
	resources
}

fn paint_resource(resources: &mut Vec<ResourceRef>, kind: u32, handle: u32) {
	if kind == 2 {
		push_resource(resources, RESOURCE_GRADIENT, handle, 0);
	}
}

fn push_resource(resources: &mut Vec<ResourceRef>, kind: u32, index: u32, generation: u32) {
	let resource = ResourceRef { kind, index, generation };
	if !resources.contains(&resource) {
		resources.push(resource);
	}
}

fn write_effects(writer: &mut Writer, inst: &Instance, effects: &Effects) {
	let flags = u32::from(effects.repaint)
		| (u32::from(effects.has_caret) << 1)
		| (u32::from(effects.has_ime) << 2);
	writer.u32(flags);
	writer.u32(effects.cursor);
	if effects.focus == kernel_slir::NONE {
		writer.string("");
	} else {
		writer.string(&scene::key_of(inst.doc(), &inst.st.lists, effects.focus));
	}
	for value in [
		effects.caret_x,
		effects.caret_y,
		effects.caret_w,
		effects.caret_h,
		effects.ime_x,
		effects.ime_y,
		effects.ime_w,
		effects.ime_h,
	] {
		writer.f64(value);
	}
	writer.count(effects.sig_name.len());
	for (index, name) in effects.sig_name.iter().copied().enumerate() {
		let meta = &effects.sig_meta[index];
		writer.string(kernel_slir::str_at(inst.doc(), name));
		writer.string(&effects.sig_text[index]);
		writer.string(&effects.sig_item[index]);
		for value in [meta.x, meta.y, meta.dx, meta.dy, meta.drag_dx, meta.drag_dy] {
			writer.f64(value);
		}
		writer.u32(meta.mods);
		writer.u32(meta.button);
		writer.u32(meta.clicks);
		writer.string(&meta.key);
		writer.string(&meta.hit_key);
		writer.string(&meta.pressed_key);
		writer.string(&meta.src_key);
		writer.string(&meta.src_item);
		writer.u32(u32::from(meta.cancelled));
		writer.u32(u32::from(meta.dropped));
	}
	writer.count(effects.scrolls.len());
	for scroll in &effects.scrolls {
		writer.string(&scroll.key);
		writer.u32(scroll.axis);
		writer.f64(scroll.off);
	}
}

struct Writer {
	bytes: Vec<u8>,
}

impl Writer {
	fn with_capacity(capacity: usize) -> Self {
		Self { bytes: Vec::with_capacity(capacity) }
	}

	fn finish(self) -> Vec<u8> {
		self.bytes
	}

	const fn len(&self) -> usize {
		self.bytes.len()
	}

	fn bytes(&mut self, bytes: &[u8]) {
		self.bytes.extend_from_slice(bytes);
	}

	fn u32(&mut self, value: u32) {
		self.bytes.extend_from_slice(&value.to_le_bytes());
	}

	fn f64(&mut self, value: f64) {
		self.bytes.extend_from_slice(&value.to_le_bytes());
	}

	fn count(&mut self, value: usize) {
		self.u32(u32::try_from(value).expect("GPU ABI section must fit u32"));
	}

	fn string(&mut self, value: &str) {
		self.count(value.len());
		self.bytes(value.as_bytes());
	}

	fn patch_u32(&mut self, offset: usize, value: u32) {
		self.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
	}

	fn section(&mut self, kind: u32, write: impl FnOnce(&mut Self)) {
		self.u32(kind);
		let length_at = self.len();
		self.u32(0);
		let payload_at = self.len();
		write(self);
		let length = self.len() - payload_at;
		self
			.patch_u32(length_at, u32::try_from(length).expect("GPU ABI section length must fit u32"));
	}
}
