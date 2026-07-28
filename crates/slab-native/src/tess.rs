//! lyon tessellation of SLIR paths, rectangle strokes (dashed, per-side, or
//! squircle), and squircle fills, cached as GPU vertex/index buffers.
//! Coordinates stay in path-local logical units; the mesh vertex shader
//! scales/offsets/rotates.

use lyon::{
	math::{Box2D, Point, point},
	path::{
		Path, PathEvent, Winding,
		builder::{BorderRadii, PathBuilder},
		iterator::PathIterator,
	},
	tessellation::{
		BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
		StrokeVertex, VertexBuffers,
	},
};
use slab_kernel::{flatten::OpRect, slir::Doc, squircle::squircle_path};
use wgpu::util::DeviceExt;

/// GPU vertex and index buffers for one tessellated shape.
pub struct Mesh {
	/// Packed path-local positions consumed by the mesh pipeline.
	pub vbuf:        wgpu::Buffer,
	/// Triangle indices into [`Self::vbuf`].
	pub ibuf:        wgpu::Buffer,
	/// Number of indices submitted for each mesh instance.
	pub index_count: u32,
}

/// SLIR verb codes (spec/SLIR.md PATH): M L C Q Z.
const VM: u32 = 0;
const VL: u32 = 1;
const VC: u32 = 2;
const VQ: u32 = 3;
const VZ: u32 = 4;

fn lyon_path_data(verbs: impl IntoIterator<Item = u32>, coords: &[f64]) -> Option<Path> {
	let mut coordinate = 0usize;
	let mut builder = Path::builder();
	let mut open = false;
	let point = |index: usize| {
		Some(Point::new(*coords.get(index)? as f32, *coords.get(index.checked_add(1)?)? as f32))
	};
	for verb in verbs {
		match verb {
			VM => {
				if open {
					builder.end(false);
				}
				builder.begin(point(coordinate)?);
				open = true;
				coordinate = coordinate.checked_add(2)?;
			},
			VL => {
				if open {
					builder.line_to(point(coordinate)?);
				}
				coordinate = coordinate.checked_add(2)?;
			},
			VC => {
				if open {
					builder.cubic_bezier_to(
						point(coordinate)?,
						point(coordinate.checked_add(2)?)?,
						point(coordinate.checked_add(4)?)?,
					);
				}
				coordinate = coordinate.checked_add(6)?;
			},
			VQ => {
				if open {
					builder.quadratic_bezier_to(point(coordinate)?, point(coordinate.checked_add(2)?)?);
				}
				coordinate = coordinate.checked_add(4)?;
			},
			VZ => {
				if open {
					builder.end(true);
					open = false;
				}
			},
			_ => return None,
		}
	}
	if open {
		builder.end(false);
	}
	Some(builder.build())
}

fn lyon_path(d: &Doc, path: i32) -> Option<Path> {
	let path = usize::try_from(path).ok()?;
	let verb_offset = usize::try_from(*d.path_verb_off.get(path)?).ok()?;
	let verb_length = usize::try_from(*d.path_verb_len.get(path)?).ok()?;
	let coord_offset = usize::try_from(*d.path_coord_off.get(path)?).ok()?;
	let coord_length = usize::try_from(*d.path_coord_len.get(path)?).ok()?;
	let verbs = d
		.path_verbs
		.get(verb_offset..verb_offset.checked_add(verb_length)?)?;
	let coords = d
		.path_coords
		.get(coord_offset..coord_offset.checked_add(coord_length)?)?;
	lyon_path_data(verbs.iter().copied(), coords)
}

fn upload(device: &wgpu::Device, buf: &VertexBuffers<[f32; 2], u32>, label: &str) -> Option<Mesh> {
	if buf.indices.is_empty() {
		return None;
	}
	let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
		label:    Some(label),
		contents: bytemuck::cast_slice(&buf.vertices),
		usage:    wgpu::BufferUsages::VERTEX,
	});
	let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
		label:    Some(label),
		contents: bytemuck::cast_slice(&buf.indices),
		usage:    wgpu::BufferUsages::INDEX,
	});
	Some(Mesh { vbuf, ibuf, index_count: buf.indices.len() as u32 })
}

/// lyon path for a squircle rounded rect (contract 6.4), translated by
/// `(dx, dy)` in box-local logical units.
fn squircle_lyon_path(w: f64, h: f64, r: f64, smooth: f64, dx: f64, dy: f64) -> Option<Path> {
	let (verbs, mut coords) = squircle_path(w, h, r, smooth);
	if dx != 0.0 || dy != 0.0 {
		for pair in coords.as_chunks_mut::<2>().0 {
			pair[0] += dx;
			pair[1] += dy;
		}
	}
	lyon_path_data(verbs.iter().copied().map(u32::from), &coords)
}

fn fill_path(device: &wgpu::Device, path: &Path, label: &str) -> Option<Mesh> {
	let mut buf: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
	let mut tess = FillTessellator::new();
	tess
		.tessellate_path(
			path,
			&FillOptions::non_zero().with_tolerance(0.02),
			&mut BuffersBuilder::new(&mut buf, |v: FillVertex| v.position().to_array()),
		)
		.ok()?;
	upload(device, &buf, label)
}

/// Tessellate the fill of SLIR path `path` (non-zero winding, like the
/// raster exporter). `None` when the path is empty or degenerate.
pub fn fill_mesh(device: &wgpu::Device, d: &Doc, path: i32) -> Option<Mesh> {
	fill_path(device, &lyon_path(d, path)?, "path-fill")
}

/// Tessellates a frame-runtime path fill.
pub fn fill_mesh_data(device: &wgpu::Device, verbs: &[u8], coords: &[f64]) -> Option<Mesh> {
	let path = lyon_path_data(verbs.iter().copied().map(u32::from), coords)?;
	fill_path(device, &path, "runtime-path-fill")
}

/// Tessellate the fill of a squircle rounded rect (`smooth` 0..=1) in
/// box-local logical coordinates.
pub fn squircle_fill_mesh(
	device: &wgpu::Device,
	w: f64,
	h: f64,
	r: f64,
	smooth: f64,
) -> Option<Mesh> {
	fill_path(device, &squircle_lyon_path(w, h, r, smooth, 0.0, 0.0)?, "squircle-fill")
}

/// Tessellate a stroke of SLIR path `path` at `width` logical units,
/// applying `dash` as alternating on/off lengths when present.
pub fn stroke_mesh(
	device: &wgpu::Device,
	d: &Doc,
	path: i32,
	width: f64,
	dash: Option<(f64, f64)>,
) -> Option<Mesh> {
	stroke_path(device, lyon_path(d, path)?, width, dash, "path-stroke")
}

/// Tessellates a frame-runtime path stroke.
pub fn stroke_mesh_data(
	device: &wgpu::Device,
	verbs: &[u8],
	coords: &[f64],
	width: f64,
	dash: Option<(f64, f64)>,
) -> Option<Mesh> {
	stroke_path(
		device,
		lyon_path_data(verbs.iter().copied().map(u32::from), coords)?,
		width,
		dash,
		"runtime-path-stroke",
	)
}

/// Tessellate a rectangle stroke (dashed, per-side, or squircle) in
/// box-local logical coordinates.
pub fn rect_stroke_mesh(device: &wgpu::Device, rect: &OpRect) -> Option<Mesh> {
	let half = rect.stroke_w / 2.0;
	let offset = match rect.stroke_align {
		1 => half,
		2 => -half,
		_ => 0.0,
	};
	let (x0, y0) = (offset as f32, offset as f32);
	let (x1, y1) = ((rect.w - offset) as f32, (rect.h - offset) as f32);
	if x1 <= x0 || y1 <= y0 {
		return None;
	}

	let radius = ((rect.radius - offset).max(0.0) as f32)
		.min((x1 - x0) / 2.0)
		.min((y1 - y0) / 2.0);
	let path = if rect.stroke_sides == 15 {
		if rect.smooth > 0.0 && radius > 0.0 {
			// squircle ring: the inset box with contract-6.4 corners
			squircle_lyon_path(
				f64::from(x1 - x0),
				f64::from(y1 - y0),
				f64::from(radius),
				rect.smooth,
				f64::from(x0),
				f64::from(y0),
			)?
		} else {
			let mut path = Path::builder();
			path.add_rounded_rectangle(
				&Box2D::new(point(x0, y0), point(x1, y1)),
				&BorderRadii::new(radius),
				Winding::Positive,
			);
			path.build()
		}
	} else {
		let mut path = Path::builder();
		let sides = [
			(1, point(x0, y0), point(x1, y0)),
			(4, point(x0, y1), point(x1, y1)),
			(8, point(x0, y0), point(x0, y1)),
			(2, point(x1, y0), point(x1, y1)),
		];
		for (side, from, to) in sides {
			if rect.stroke_sides & side == 0 {
				continue;
			}
			path.begin(from);
			path.line_to(to);
			path.end(false);
		}
		path.build()
	};

	stroke_path(
		device,
		path,
		rect.stroke_w,
		rect.has_dash.then_some((rect.dash_on, rect.dash_off)),
		"rect-stroke",
	)
}

fn stroke_path(
	device: &wgpu::Device,
	path: Path,
	width: f64,
	dash: Option<(f64, f64)>,
	label: &str,
) -> Option<Mesh> {
	let path = if let Some((on, off)) = dash {
		dashed_path(&path, on as f32, off as f32).unwrap_or(path)
	} else {
		path
	};
	let mut buf: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
	let mut tess = StrokeTessellator::new();
	tess
		.tessellate_path(
			&path,
			&StrokeOptions::default()
				.with_line_width(width as f32)
				.with_tolerance(0.02),
			&mut BuffersBuilder::new(&mut buf, |v: StrokeVertex| v.position().to_array()),
		)
		.ok()?;
	upload(device, &buf, label)
}

fn dashed_path(path: &Path, on: f32, off: f32) -> Option<Path> {
	if !on.is_finite() || !off.is_finite() || on < 0.0 || off < 0.0 || on + off <= f32::EPSILON {
		return None;
	}

	let mut output = Path::builder();
	let mut dash = DashState::new(on, off);
	for event in path.iter().flattened(0.02) {
		match event {
			PathEvent::Begin { .. } => dash.restart(&mut output),
			PathEvent::Line { from, to } => dash.line(&mut output, from, to),
			PathEvent::End { last, first, close } => {
				if close {
					dash.line(&mut output, last, first);
				}
				dash.finish(&mut output);
			},
			PathEvent::Quadratic { .. } | PathEvent::Cubic { .. } => unreachable!(),
		}
	}
	Some(output.build())
}

struct DashState {
	on:        f32,
	off:       f32,
	paint:     bool,
	remaining: f32,
	open:      bool,
}

impl DashState {
	const fn new(on: f32, off: f32) -> Self {
		Self { on, off, paint: true, remaining: on, open: false }
	}

	fn restart(&mut self, output: &mut impl PathBuilder) {
		self.finish(output);
		self.paint = true;
		self.remaining = self.on;
	}

	fn finish(&mut self, output: &mut impl PathBuilder) {
		if self.open {
			output.end(false);
			self.open = false;
		}
	}

	fn line(&mut self, output: &mut impl PathBuilder, from: Point, to: Point) {
		let delta = to - from;
		let length = delta.length();
		if length <= f32::EPSILON {
			return;
		}
		let direction = delta / length;
		let mut at = 0.0;
		while at < length {
			while self.remaining <= f32::EPSILON {
				self.finish(output);
				self.paint = !self.paint;
				self.remaining = if self.paint { self.on } else { self.off };
			}

			let next_at = (at + self.remaining).min(length);
			if next_at <= at {
				break;
			}
			if self.paint {
				if !self.open {
					output.begin(from + direction * at, &[]);
					self.open = true;
				}
				output.line_to(from + direction * next_at, &[]);
			}
			self.remaining -= next_at - at;
			at = next_at;
		}
	}
}
