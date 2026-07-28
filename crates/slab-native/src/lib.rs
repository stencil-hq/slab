//! slab-native — winit + wgpu driver over the hand-maintained Rust kernel (P7).
//!
//! The kernel owns layout, hit testing, focus, editing, motion and scroll;
//! this crate only translates winit events into kernel `Event`s, paints
//! `FrameOp`s through instanced wgpu pipelines, and surfaces `Effects`
//! (signals, caret/IME rects, cursor). The renderer is window-independent:
//! tests and `--headless-frame` render to a texture and read pixels back.
//!
//! External hosts should start with [`shell`], which composes [`input`],
//! [`a11y`], surface management and [`renderer`] into the same loop exercised
//! by the in-repo viewer.

pub mod a11y;
pub mod atlas;
pub mod demo;
pub mod gen_player;
pub mod gen_settings;
pub mod holes;
pub mod input;
pub mod player;
pub mod renderer;
pub(crate) mod sdp;
pub mod shell;
pub mod surface;
pub mod tess;
pub mod view;

/// A decoded native document together with the image payloads that are kept
/// out of the kernel document and runtime-provided font faces.
pub struct NativeDocument {
	pub inst: slab_kernel::frame::Instance,
	pub imgs: Vec<Vec<u8>>,
	fonts:    Vec<RegisteredFont>,
}

/// A face registered by the host. The original bytes are retained so each
/// renderer can construct an atlas face without consulting the SLIR payload.
pub struct RegisteredFont {
	pub name:   String,
	pub weight: u32,
	pub bytes:  Vec<u8>,
}

impl NativeDocument {
	pub fn decode(bytes: &[u8]) -> Result<Self, String> {
		let (inst, imgs) = slab_slir::instance(bytes)?;
		Ok(Self { inst, imgs, fonts: Vec::new() })
	}

	/// Wraps a generated Rust document's public `inst` and `imgs` fields for
	/// use with [`shell::NativeShell`].
	pub const fn from_parts(inst: slab_kernel::frame::Instance, imgs: Vec<Vec<u8>>) -> Self {
		Self { inst, imgs, fonts: Vec::new() }
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
		self
			.fonts
			.push(RegisteredFont { name: name.to_owned(), weight, bytes });
		true
	}

	/// Selects a compiler-declared theme. The renderer synchronizes resolved
	/// color resources from the document before every frame build.
	pub fn set_theme(&mut self, name: &str) -> bool {
		slab_kernel::frame::inst_set_theme(&mut self.inst, name)
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
	doc_id:       Option<usize>,
}

impl NativeDriver {
	pub const fn new(document: NativeDocument, renderer: renderer::Renderer) -> Self {
		Self { document, renderer, doc_id: None }
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

	/// Selects a theme and immediately refreshes registered GPU color tables.
	pub fn set_theme(&mut self, name: &str) -> bool {
		if !self.document.set_theme(name) {
			return false;
		}
		if let Some(doc_id) = self.doc_id {
			self
				.renderer
				.refresh_registered_colors(doc_id, &self.document.inst.doc);
		}
		true
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
		power_preference:       wgpu::PowerPreference::default(),
		force_fallback_adapter: false,
		compatible_surface:     surface,
		apply_limit_buckets:    false,
	}))
	.ok()?;
	let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
		label: Some("slab-native"),
		..Default::default()
	}))
	.ok()?;
	Some((adapter, device, queue))
}
