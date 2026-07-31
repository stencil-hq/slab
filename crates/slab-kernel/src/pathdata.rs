//! Canonical SVG path-data normalization shared by compilation and runtime.
//!
//! The full path grammar (`M L H V C S Q T A Z`, relative and absolute) is
//! lowered to absolute `M L C Q Z`. `H`/`V` lower to `L`; `S`/`T` reflect
//! their control point; `A` lowers to cubic Bézier segments of at most a
//! quarter turn each (degenerate arcs lower to their chord).

/// Move-to verb byte.
pub const VM: u8 = 0;
/// Line-to verb byte.
pub const VL: u8 = 1;
/// Cubic Bézier verb byte.
pub const VC: u8 = 2;
/// Quadratic Bézier verb byte.
pub const VQ: u8 = 3;
/// Close-path verb byte.
pub const VZ: u8 = 4;

struct NumScan<'a> {
	s:   &'a [u8],
	pos: usize,
}

impl NumScan<'_> {
	const fn skip_sep(&mut self) {
		while self.pos < self.s.len()
			&& matches!(self.s[self.pos], b' ' | b'\t' | b'\n' | b'\r' | b',')
		{
			self.pos += 1;
		}
	}

	fn peek_cmd(&mut self) -> Option<u8> {
		self.skip_sep();
		let c = *self.s.get(self.pos)?;
		c.is_ascii_alphabetic().then_some(c)
	}

	fn num(&mut self) -> Option<f64> {
		self.skip_sep();
		let start = self.pos;
		let mut seen_digit = false;
		let mut seen_dot = false;
		let mut seen_exp = false;
		while self.pos < self.s.len() {
			let c = self.s[self.pos];
			match c {
				b'0'..=b'9' => seen_digit = true,
				b'.' if !seen_dot && !seen_exp => seen_dot = true,
				b'-' | b'+' if self.pos == start => {},
				b'-' | b'+' if matches!(self.s[self.pos - 1], b'e' | b'E') => {},
				b'e' | b'E' if seen_digit && !seen_exp => {
					seen_exp = true;
					seen_dot = true;
				},
				_ => break,
			}
			self.pos += 1;
		}
		if !seen_digit {
			self.pos = start;
			return None;
		}
		let value: f64 = std::str::from_utf8(&self.s[start..self.pos])
			.ok()?
			.parse()
			.ok()?;
		value.is_finite().then_some(value)
	}
}

/// Normalizes path data, returning `None` for no drawable content or malformed
/// command sequences.
pub fn normalize(d: &str) -> Option<(Vec<u8>, Vec<f64>)> {
	let mut sc = NumScan { s: d.as_bytes(), pos: 0 };
	let mut verbs = Vec::new();
	let mut coords = Vec::new();
	let (mut cx, mut cy) = (0.0_f64, 0.0_f64);
	let (mut sx, mut sy) = (0.0_f64, 0.0_f64);
	let mut prev_cubic: Option<(f64, f64)> = None;
	let mut prev_quad: Option<(f64, f64)> = None;

	while let Some(cmd) = sc.peek_cmd() {
		sc.pos += 1;
		let rel = cmd.is_ascii_lowercase();
		let command = cmd.to_ascii_uppercase();
		if command == b'Z' {
			verbs.push(VZ);
			cx = sx;
			cy = sy;
			prev_cubic = None;
			prev_quad = None;
			continue;
		}
		let mut first = true;
		loop {
			let before = sc.pos;
			let has_number = match command {
				b'M' | b'L' | b'T' | b'H' | b'V' | b'C' | b'S' | b'Q' | b'A' => sc.num().is_some(),
				_ => false,
			};
			if !has_number {
				sc.pos = before;
				if first && command != b'M' {
					return None;
				}
				break;
			}
			sc.pos = before;
			macro_rules! number {
				() => {
					sc.num()?
				};
			}
			let absolute = |value: f64, base: f64| if rel { base + value } else { value };
			match command {
				b'M' => {
					let x = absolute(number!(), cx);
					let y = absolute(number!(), cy);
					cx = x;
					cy = y;
					if first {
						verbs.push(VM);
						sx = x;
						sy = y;
					} else {
						verbs.push(VL);
					}
					coords.extend([x, y]);
					prev_cubic = None;
					prev_quad = None;
				},
				b'L' => {
					cx = absolute(number!(), cx);
					cy = absolute(number!(), cy);
					verbs.push(VL);
					coords.extend([cx, cy]);
					prev_cubic = None;
					prev_quad = None;
				},
				b'H' => {
					cx = absolute(number!(), cx);
					verbs.push(VL);
					coords.extend([cx, cy]);
					prev_cubic = None;
					prev_quad = None;
				},
				b'V' => {
					cy = absolute(number!(), cy);
					verbs.push(VL);
					coords.extend([cx, cy]);
					prev_cubic = None;
					prev_quad = None;
				},
				b'C' | b'S' => {
					let (c1x, c1y) = if command == b'C' {
						(absolute(number!(), cx), absolute(number!(), cy))
					} else {
						match prev_cubic {
							Some((px, py)) => (2.0f64.mul_add(cx, -px), 2.0f64.mul_add(cy, -py)),
							None => (cx, cy),
						}
					};
					let c2x = absolute(number!(), cx);
					let c2y = absolute(number!(), cy);
					let ex = absolute(number!(), cx);
					let ey = absolute(number!(), cy);
					verbs.push(VC);
					coords.extend([c1x, c1y, c2x, c2y, ex, ey]);
					prev_cubic = Some((c2x, c2y));
					prev_quad = None;
					cx = ex;
					cy = ey;
				},
				b'Q' | b'T' => {
					let (qx, qy) = if command == b'Q' {
						(absolute(number!(), cx), absolute(number!(), cy))
					} else {
						match prev_quad {
							Some((px, py)) => (2.0f64.mul_add(cx, -px), 2.0f64.mul_add(cy, -py)),
							None => (cx, cy),
						}
					};
					let ex = absolute(number!(), cx);
					let ey = absolute(number!(), cy);
					verbs.push(VQ);
					coords.extend([qx, qy, ex, ey]);
					prev_quad = Some((qx, qy));
					prev_cubic = None;
					cx = ex;
					cy = ey;
				},
				b'A' => {
					let rx = number!();
					let ry = number!();
					let phi = number!().to_radians();
					let large_arc = number!() != 0.0;
					let sweep = number!() != 0.0;
					let ex = absolute(number!(), cx);
					let ey = absolute(number!(), cy);
					arc_to_cubics(
						&mut verbs,
						&mut coords,
						(cx, cy),
						(ex, ey),
						(rx, ry),
						phi,
						large_arc,
						sweep,
					);
					prev_cubic = None;
					prev_quad = None;
					cx = ex;
					cy = ey;
				},
				_ => return None,
			}
			first = false;
		}
	}

	sc.skip_sep();
	if verbs.is_empty() || sc.pos != sc.s.len() {
		return None;
	}
	Some((verbs, coords))
}

/// Lowers one elliptical-arc segment to cubic Béziers (SVG 1.1 F.6.5),
/// splitting the sweep into segments of at most a quarter turn. Coincident
/// endpoints omit the segment entirely (SVG F.6.2) and zero radii lower to
/// the chord line (SVG F.6.6 degeneracies).
///
/// Transcendentals go through the pure-Rust `libm` crate, never the platform
/// libm: this runs at compile time and its exact f64 results are serialized
/// into SLIR, so native and WASM compilers must produce identical bytes.
fn arc_to_cubics(
	verbs: &mut Vec<u8>,
	coords: &mut Vec<f64>,
	(x1, y1): (f64, f64),
	(x2, y2): (f64, f64),
	(rx, ry): (f64, f64),
	phi: f64,
	large_arc: bool,
	sweep: bool,
) {
	let (mut rx, mut ry) = (rx.abs(), ry.abs());
	if x1 == x2 && y1 == y2 {
		return;
	}
	if rx == 0.0 || ry == 0.0 {
		verbs.push(VL);
		coords.extend([x2, y2]);
		return;
	}
	let (sin_phi, cos_phi) = libm::sincos(phi);
	// Endpoint -> center parameterization (F.6.5).
	let dx = f64::midpoint(x1, -x2);
	let dy = f64::midpoint(y1, -y2);
	let x1p = sin_phi.mul_add(dy, cos_phi * dx);
	let y1p = cos_phi.mul_add(dy, -sin_phi * dx);
	let lambda = (x1p / rx).mul_add(x1p / rx, (y1p / ry).powi(2));
	if lambda > 1.0 {
		let scale = lambda.sqrt();
		rx *= scale;
		ry *= scale;
	}
	let radii = rx * ry;
	let rxs = rx * y1p;
	let rys = ry * x1p;
	let numerator = rys.mul_add(-rys, rxs.mul_add(-rxs, radii * radii)).max(0.0);
	let denominator = rxs.mul_add(rxs, rys * rys);
	let mut coefficient = (numerator / denominator).sqrt();
	if large_arc == sweep {
		coefficient = -coefficient;
	}
	let cxp = coefficient * rx * y1p / ry;
	let cyp = -coefficient * ry * x1p / rx;
	let center_x = cos_phi.mul_add(cxp, -sin_phi * cyp) + f64::midpoint(x1, x2);
	let center_y = sin_phi.mul_add(cxp, cos_phi * cyp) + f64::midpoint(y1, y2);
	let start = libm::atan2((y1p - cyp) / ry, (x1p - cxp) / rx);
	let end = libm::atan2((-y1p - cyp) / ry, (-x1p - cxp) / rx);
	let mut delta = end - start;
	if sweep && delta < 0.0 {
		delta += std::f64::consts::TAU;
	} else if !sweep && delta > 0.0 {
		delta -= std::f64::consts::TAU;
	}
	let point = |t: f64| {
		let (sin_t, cos_t) = libm::sincos(t);
		(
			ry.mul_add(-sin_phi * sin_t, rx.mul_add(cos_phi * cos_t, center_x)),
			ry.mul_add(cos_phi * sin_t, rx.mul_add(sin_phi * cos_t, center_y)),
		)
	};
	let derivative = |t: f64| {
		let (sin_t, cos_t) = libm::sincos(t);
		(
			ry.mul_add(-sin_phi * cos_t, -rx * cos_phi * sin_t),
			ry.mul_add(cos_phi * cos_t, -rx * sin_phi * sin_t),
		)
	};
	#[allow(
		clippy::cast_possible_truncation,
		clippy::cast_sign_loss,
		reason = "segment count is in 1..=4"
	)]
	let segments = (delta.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
	let step = delta / segments as f64;
	// Cubic approximation of an elliptical sweep: controls follow the arc
	// tangents with the standard 4/3 * tan(step/4) handle length.
	let handle = 4.0 / 3.0 * libm::tan(step / 4.0);
	let (mut px, mut py) = (x1, y1);
	let mut t = start;
	for segment in 0..segments {
		let t_next = t + step;
		// Pin the final endpoint to the authored coordinates exactly.
		let (ex, ey) = if segment + 1 == segments {
			(x2, y2)
		} else {
			point(t_next)
		};
		let (d1x, d1y) = derivative(t);
		let (d2x, d2y) = derivative(t_next);
		verbs.push(VC);
		coords.extend([
			handle.mul_add(d1x, px),
			handle.mul_add(d1y, py),
			handle.mul_add(-d2x, ex),
			handle.mul_add(-d2y, ey),
			ex,
			ey,
		]);
		px = ex;
		py = ey;
		t = t_next;
	}
}

/// Returns `(min_x, min_y, max_x, max_y)` over normalized on-curve and control
/// points, or `None` when `coords` contains no complete point.
pub fn bounds(coords: &[f64]) -> Option<(f64, f64, f64, f64)> {
	let (points, _) = coords.as_chunks::<2>();
	let ([min_x, min_y], rest) = points.split_first()?;
	let (mut min_x, mut min_y, mut max_x, mut max_y) = (*min_x, *min_y, *min_x, *min_y);
	for &[x, y] in rest {
		min_x = min_x.min(x);
		min_y = min_y.min(y);
		max_x = max_x.max(x);
		max_y = max_y.max(y);
	}
	Some((min_x, min_y, max_x, max_y))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn absolute_mlz() {
		let (verbs, coords) = normalize("M0 104 L60 68 Z").unwrap();
		assert_eq!(verbs, vec![VM, VL, VZ]);
		assert_eq!(coords, vec![0.0, 104.0, 60.0, 68.0]);
	}

	#[test]
	fn hv_lower_and_implicit_l() {
		let (verbs, coords) = normalize("M10 10 H20 V30 L5 5 10 10").unwrap();
		assert_eq!(verbs, vec![VM, VL, VL, VL, VL]);
		assert_eq!(coords[2..4], [20.0, 10.0]);
		assert_eq!(coords[4..6], [20.0, 30.0]);
	}

	#[test]
	fn relative_and_smooth() {
		let (verbs, _) = normalize("m10 10 c 5 0 10 5 10 10 s 10 10 20 10").unwrap();
		assert_eq!(verbs, vec![VM, VC, VC]);
	}

	#[test]
	fn rejects_trailing_junk_and_non_finite_numbers() {
		assert!(normalize("M0 0 !").is_none());
		assert!(normalize("M0 0 L1e999 2").is_none());
	}

	#[test]
	fn reports_control_point_bounds() {
		let (_, coords) = normalize("M-2 3 C8 -4 5 12 7 9").unwrap();
		assert_eq!(bounds(&coords), Some((-2.0, -4.0, 8.0, 12.0)));
	}

	#[test]
	fn arc_lowers_to_quarter_turn_cubics() {
		// Half circle: r=7 from (10,3) to (10,17) -> two 90-degree cubics.
		let (verbs, coords) = normalize("M10 3 A7 7 0 1 0 10 17").unwrap();
		assert_eq!(verbs, vec![VM, VC, VC]);
		// The authored endpoint is pinned exactly.
		assert_eq!(coords[coords.len() - 2..], [10.0, 17.0]);
		// The sweep passes through the circle's leftmost point (3, 10).
		let (mid_x, mid_y) = (coords[6], coords[7]);
		assert!((mid_x - 3.0).abs() < 1e-9, "mid_x={mid_x}");
		assert!((mid_y - 10.0).abs() < 1e-9, "mid_y={mid_y}");
	}

	#[test]
	fn degenerate_arc_lowers_to_chord() {
		let (verbs, coords) = normalize("M0 0 A0 5 0 0 1 10 0").unwrap();
		assert_eq!(verbs, vec![VM, VL]);
		assert_eq!(coords[2..], [10.0, 0.0]);
	}

	#[test]
	fn coincident_arc_endpoints_emit_no_segment() {
		let (verbs, coords) = normalize("M0 0 A5 5 0 1 0 0 0").unwrap();
		assert_eq!(verbs, vec![VM]);
		assert_eq!(coords, vec![0.0, 0.0]);
	}
}
