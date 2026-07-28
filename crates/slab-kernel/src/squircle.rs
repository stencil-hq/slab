//! Canonical squircle (smooth-corner) outline shared by every painter.
//!
//! Implements the Figma corner-smoothing parameterization: each corner is a
//! circular arc flanked by two cubic segments whose combined edge footprint is
//! `(1 + smooth) · radius`. The constructor is the single source of truth —
//! painters convert its verbs into their native path types instead of forking
//! per-client formulas (SPEC §7 `smooth`).

/// Per-corner construction constants for one uniform radius and smoothing.
struct Corner {
	/// Edge footprint of the whole corner.
	p:       f64,
	/// Cubic flank coefficients from the Figma parameterization.
	a:       f64,
	b:       f64,
	c:       f64,
	d:       f64,
	/// Chord of the central circular arc.
	arc_len: f64,
	/// Cubic control-handle length approximating the central arc.
	handle:  f64,
}

fn corner(radius: f64, smooth: f64, max_radius: f64) -> Corner {
	let radius = radius.min(max_radius);
	// Near the geometric limit the smoothing collapses so flanks never
	// overlap the opposite corner (Figma's behavior).
	let smooth = if radius > max_radius / 2.0 {
		smooth * (1.0 - (radius - max_radius / 2.0) / (max_radius / 2.0))
	} else {
		smooth
	};
	let p = ((1.0 + smooth) * radius).min(max_radius);
	let arc_measure = 90.0 * (1.0 - smooth);
	let arc_len = (arc_measure / 2.0).to_radians().sin() * radius * 2.0_f64.sqrt();
	let angle_alpha = (90.0 - arc_measure) / 2.0;
	let p3_to_p4 = radius * (angle_alpha / 2.0).to_radians().tan();
	let angle_beta = 45.0 * smooth;
	let c = p3_to_p4 * angle_beta.to_radians().cos();
	let d = c * angle_beta.to_radians().tan();
	let b = (p - arc_len - c - d) / 3.0;
	let a = 2.0 * b;
	let handle = 4.0 / 3.0 * (arc_measure / 4.0).to_radians().tan() * radius;
	Corner { p, a, b, c, d, arc_len, handle }
}

/// Path verb: move-to (two coordinates).
pub const V_MOVE: u8 = 0;
/// Path verb: line-to (two coordinates).
pub const V_LINE: u8 = 1;
/// Path verb: cubic-to (six coordinates).
pub const V_CUBIC: u8 = 2;
/// Path verb: close (no coordinates).
pub const V_CLOSE: u8 = 4;

/// Builds the squircle outline for a `w × h` box with uniform `radius` and
/// smoothing.
///
/// Corner smoothing is Figma-style `smooth` (0..1), origin at the box top-left,
/// clockwise. Returns document-encoded path data: verbs `0` move, `1` line,
/// `2` cubic, `4` close, with coordinates in layout units.
///
/// `smooth == 0` degenerates to the plain rounded rectangle (single-arc
/// cubics); callers should prefer their native rounded rect in that case.
pub fn squircle_path(w: f64, h: f64, radius: f64, smooth: f64) -> (Vec<u8>, Vec<f64>) {
	let max_radius = (w.min(h) / 2.0).max(0.0);
	let k = corner(radius.max(0.0), smooth.clamp(0.0, 1.0), max_radius);
	let mut verbs = Vec::with_capacity(14);
	let mut coords = Vec::with_capacity(76);

	// One corner is three segments in travel order: flank cubic, central-arc
	// cubic, mirrored flank cubic. Displacements are for the top-right corner
	// (traveling +x along the top edge); later corners rotate them 90°
	// clockwise per step: (dx, dy) → (-dy, dx).
	let flank_in = [(k.a, 0.0), (k.a + k.b, 0.0), (k.a + k.b + k.c, k.d)];
	let flank_out = [(k.d, k.c), (k.d, k.b + k.c), (k.d, k.a + k.b + k.c)];
	// Unit tangents at the arc join points; the handle length already carries
	// the radius factor.
	let t_in_len = k.c.hypot(k.d);
	let (tin, tout) = if t_in_len > 0.0 {
		((k.c / t_in_len, k.d / t_in_len), (k.d / t_in_len, k.c / t_in_len))
	} else {
		// smooth == 0: flanks vanish and the arc spans the full quarter turn.
		((1.0, 0.0), (0.0, 1.0))
	};

	let rot = |(dx, dy): (f64, f64), quarter_turns: u32| -> (f64, f64) {
		match quarter_turns % 4 {
			0 => (dx, dy),
			1 => (-dy, dx),
			2 => (-dx, -dy),
			_ => (dy, -dx),
		}
	};

	// Corner start points and the straight edges that precede them.
	let starts = [(w - k.p, 0.0), (w, h - k.p), (k.p, h), (0.0, k.p)];

	verbs.push(V_MOVE);
	coords.extend([starts[0].0, starts[0].1]);
	for (turn, start) in starts.iter().enumerate() {
		let turn = u32::try_from(turn).expect("four corners");
		if turn > 0 {
			verbs.push(V_LINE);
			coords.extend([start.0, start.1]);
		}
		let mut cursor = *start;
		let flank = |verbs: &mut Vec<u8>,
		             coords: &mut Vec<f64>,
		             cursor: &mut (f64, f64),
		             pts: [(f64, f64); 3]| {
			verbs.push(V_CUBIC);
			for (dx, dy) in pts {
				let (dx, dy) = rot((dx, dy), turn);
				coords.extend([cursor.0 + dx, cursor.1 + dy]);
			}
			let (ex, ey) = rot(pts[2], turn);
			*cursor = (cursor.0 + ex, cursor.1 + ey);
		};
		flank(&mut verbs, &mut coords, &mut cursor, flank_in);
		// Central arc as a single cubic: controls follow the join tangents.
		let (sx, sy) = cursor;
		let (adx, ady) = rot((k.arc_len, k.arc_len), turn);
		let (ex, ey) = (sx + adx, sy + ady);
		let (t0x, t0y) = rot(tin, turn);
		let (t1x, t1y) = rot(tout, turn);
		verbs.push(V_CUBIC);
		coords.extend([
			k.handle.mul_add(t0x, sx),
			k.handle.mul_add(t0y, sy),
			k.handle.mul_add(-t1x, ex),
			k.handle.mul_add(-t1y, ey),
			ex,
			ey,
		]);
		cursor = (ex, ey);
		flank(&mut verbs, &mut coords, &mut cursor, flank_out);
	}
	verbs.push(V_CLOSE);
	(verbs, coords)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn endpoint_after(verbs: &[u8], coords: &[f64], upto: usize) -> (f64, f64) {
		let mut ci = 0;
		let mut end = (0.0, 0.0);
		for verb in &verbs[..upto] {
			let n = match verb {
				&V_MOVE | &V_LINE => 2,
				&V_CUBIC => 6,
				_ => 0,
			};
			if n > 0 {
				end = (coords[ci + n - 2], coords[ci + n - 1]);
				ci += n;
			}
		}
		end
	}

	#[test]
	fn corners_land_on_edges() {
		let (verbs, coords) = squircle_path(200.0, 100.0, 24.0, 0.6);
		// Move + 4 × (line? + 3 cubics) + close; first corner has no line.
		assert_eq!(verbs[0], V_MOVE);
		assert_eq!(*verbs.last().unwrap(), V_CLOSE);
		// After the first corner (verbs[0..=3]) the cursor is on the right edge.
		let (x, y) = endpoint_after(&verbs, &coords, 4);
		assert!((x - 200.0).abs() < 1e-9, "x={x}");
		assert!((y - 38.4).abs() < 1e-9, "y={y}"); // p = 1.6·24
	}

	#[test]
	fn zero_smooth_matches_circular_corner() {
		let (verbs, coords) = squircle_path(100.0, 100.0, 20.0, 0.0);
		// Flank cubics collapse to zero length; the arc cubic starts at the
		// classic rounded-rect tangent point.
		assert_eq!(verbs[0], V_MOVE);
		assert!((coords[0] - 80.0).abs() < 1e-9);
		let (x, y) = endpoint_after(&verbs, &coords, 4);
		assert!((x - 100.0).abs() < 1e-9);
		assert!((y - 20.0).abs() < 1e-9);
	}

	#[test]
	fn radius_clamps_to_half_extent() {
		let (_, coords) = squircle_path(40.0, 40.0, 999.0, 0.6);
		for pair in coords.chunks(2) {
			assert!(pair[0] >= -1e-9 && pair[0] <= 40.0 + 1e-9);
			assert!(pair[1] >= -1e-9 && pair[1] <= 40.0 + 1e-9);
		}
	}
}
