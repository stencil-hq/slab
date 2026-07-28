//! Canonical SVG path-data normalization shared by compilation and runtime.
//!
//! The full path grammar (`M L H V C S Q T A Z`, relative and absolute) is
//! lowered to absolute `M L C Q Z`. `H`/`V` lower to `L`; `S`/`T` reflect
//! their control point; `A` currently lowers to its chord.

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
					let _ = (number!(), number!(), number!(), number!(), number!());
					let ex = absolute(number!(), cx);
					let ey = absolute(number!(), cy);
					verbs.push(VL);
					coords.extend([ex, ey]);
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

/// Returns `(min_x, min_y, max_x, max_y)` over normalized on-curve and control
/// points, or `None` when `coords` contains no complete point.
pub fn bounds(coords: &[f64]) -> Option<(f64, f64, f64, f64)> {
	let mut points = coords.chunks_exact(2);
	let first = points.next()?;
	let (mut min_x, mut min_y, mut max_x, mut max_y) = (first[0], first[1], first[0], first[1]);
	for point in points {
		min_x = min_x.min(point[0]);
		min_y = min_y.min(point[1]);
		max_x = max_x.max(point[0]);
		max_y = max_y.max(point[1]);
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
}
