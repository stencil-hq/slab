//! Hit testing follows reverse paint order.
//!
//! A candidate hits when the point, transformed into each rotated ancestor's
//! local space from outermost to innermost, lies inside the candidate's rounded
//! rectangle and every clipping ancestor's rounded rectangle. Each transform
//! rotates by `-deg` about `(cx, cy)`. Rounded radii use the same
//! `min(radius, width / 2, height / 2)` clamp as the painters.
//! Inert subtrees never hit.

use crate::{
	scene::{self, Scene},
	slir::{F_CLIP, F_INERT},
};

/// The circle constant used to convert degrees to radians.
pub const PI: f64 = std::f64::consts::PI;

/// Returns the sine of an angle measured in degrees.
pub fn sin_deg(deg: f64) -> f64 {
	deg.to_radians().sin()
}

/// Returns the cosine of an angle measured in degrees.
pub fn cos_deg(deg: f64) -> f64 {
	deg.to_radians().cos()
}

/// Reports whether a point lies within a rounded rectangle.
///
/// Only the four `radius × radius` corner squares can reject a point that is
/// inside the rectangle's bounds. The radius is clamped in the same way as it
/// is by the painters.
pub fn in_rounded_rect(px: f64, py: f64, x: f64, y: f64, w: f64, h: f64, radius: f64) -> bool {
	if px < x || py < y || px > x + w || py > y + h {
		return false;
	}

	let radius = 0.0_f64.max(radius.min((w / 2.0).min(h / 2.0)));
	if radius <= 0.0 {
		return true;
	}

	let corner_x = if px < x + radius {
		Some(x + radius)
	} else if px > x + w - radius {
		Some(x + w - radius)
	} else {
		None
	};
	let corner_y = if py < y + radius {
		Some(y + radius)
	} else if py > y + h - radius {
		Some(y + h - radius)
	} else {
		None
	};

	let (Some(corner_x), Some(corner_y)) = (corner_x, corner_y) else {
		return true;
	};
	let dx = px - corner_x;
	let dy = py - corner_y;
	dy.mul_add(dy, dx * dx) <= radius * radius
}

/// Transforms a point into the local space of a node rotated by `deg` about
/// `(cx, cy)`, returning its x coordinate.
pub fn unrotate_x(px: f64, py: f64, deg: f64, cx: f64, cy: f64) -> f64 {
	let cosine = cos_deg(0.0 - deg);
	let sine = sin_deg(0.0 - deg);
	(py - cy).mul_add(-sine, (px - cx).mul_add(cosine, cx))
}

/// Transforms a point into the local space of a node rotated by `deg` about
/// `(cx, cy)`, returning its y coordinate.
pub fn unrotate_y(px: f64, py: f64, deg: f64, cx: f64, cy: f64) -> f64 {
	let cosine = cos_deg(0.0 - deg);
	let sine = sin_deg(0.0 - deg);
	(py - cy).mul_add(cosine, (px - cx).mul_add(sine, cy))
}

/// Reports whether a point is inside a scene node after applying every
/// ancestor transform from outermost to innermost and rejecting points outside
/// any clipping ancestor.
pub fn contains(scene: &Scene, node: i32, x: f64, y: f64) -> bool {
	let mut chain = Vec::new();
	scene::chain(scene, node, &mut chain);
	let mut px = x;
	let mut py = y;

	for link in chain {
		let index = usize::try_from(link).expect("scene indices must be non-negative");
		let entry = &scene.entries[index];
		let rotation = entry.rot_deg;
		if rotation != 0.0 {
			let local_x = unrotate_x(px, py, rotation, entry.rot_cx, entry.rot_cy);
			let local_y = unrotate_y(px, py, rotation, entry.rot_cx, entry.rot_cy);
			px = local_x;
			py = local_y;
		}

		let inside = in_rounded_rect(
			px,
			py,
			entry.x,
			entry.y,
			entry.w,
			entry.h,
			entry.radius,
		);
		if link == node {
			return inside;
		}
		if entry.flags & F_CLIP != 0 && !inside {
			return false;
		}
	}

	false
}

/// Writes the topmost hit at `(x, y)` as scene indices from root to target.
///
/// Candidates are tried in reverse paint order. Inert nodes never hit, and a
/// miss leaves `out` empty.
pub fn hit_test(scene: &Scene, x: f64, y: f64, out: &mut Vec<i32>) {
	out.clear();
	let node_count = scene::count(scene);

	for offset in 0..node_count {
		let node = node_count.wrapping_sub(1).wrapping_sub(offset);
		let index = usize::try_from(node).expect("scene indices must be non-negative");
		if scene.entries[index].flags & F_INERT != 0 {
			continue;
		}
		if contains(scene, node, x, y) {
			scene::chain(scene, node, out);
			return;
		}
	}
}
