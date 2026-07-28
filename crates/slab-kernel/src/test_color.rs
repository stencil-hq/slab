//! Math intrinsics, exact `OKLab` round trips, and interpolation midpoints
//! checked against the reference outputs from `slab/color.py`.

use crate::color;

/// Returns whether two floating-point values differ by at most `eps`.
pub fn approx(a: f64, b: f64, eps: f64) -> bool {
	(a - b).abs() <= eps
}

/// Returns whether an RGBA color survives an exact `OKLab` round trip.
pub fn rt_ok(color: u32) -> bool {
	let lab = color::oklab_from_rgba(color);
	color::rgba_from_oklab(&lab, color & 0xff) == color
}

/// Checks the floating-point intrinsics used by the color conversion.
pub fn test_math_intrinsics() {
	assert!(approx(8.0_f64.cbrt(), 2.0, 1.0e-12), "cbrt(8) == 2");
	assert!(approx(27.0_f64.cbrt(), 3.0, 1.0e-12), "cbrt(27) == 3");
	assert!(approx(0.001_f64.cbrt(), 0.1, 1.0e-15), "cbrt(0.001) == 0.1");
	assert!(approx((-8.0_f64).cbrt(), -2.0, 1.0e-12), "cbrt(-8) == -2");
	assert_eq!(0.0_f64.cbrt(), 0.0, "cbrt(0) == 0");
	assert_eq!(1.0_f64.cbrt(), 1.0, "cbrt(1) == 1");
	assert!(approx(9.0_f64.sqrt(), 3.0, 1.0e-12), "pow(9, 0.5) == 3");
}

/// Checks exact RGBA-to-OKLab-to-RGBA round trips.
pub fn test_roundtrip() {
	assert!(rt_ok(0xff0000ff), "red round-trips");
	assert!(rt_ok(0x00ff00ff), "green round-trips");
	assert!(rt_ok(0x0000ffff), "blue round-trips");
	assert!(rt_ok(0x336699ff), "steel round-trips");
	assert!(rt_ok(0x808080ff), "gray round-trips");
	assert!(rt_ok(0xffffffff), "white round-trips");
	assert!(rt_ok(0x000000ff), "black round-trips");
	assert!(rt_ok(0x12345680), "alpha is preserved");
}

/// Checks `OKLab` interpolation endpoints and reference intermediate colors.
pub fn test_lerp() {
	assert_eq!(color::lerp_oklab(0xff0000ff, 0x0000ffff, 0.0), 0xff0000ff, "t=0 endpoint");
	assert_eq!(color::lerp_oklab(0xff0000ff, 0x0000ffff, 1.0), 0x0000ffff, "t=1 endpoint");
	assert_eq!(color::lerp_oklab(0x000000ff, 0xffffffff, 0.5), 0x636363ff, "black/white midpoint");
	assert_eq!(color::lerp_oklab(0xff0000ff, 0x0000ffff, 0.5), 0x8c53a2ff, "red/blue midpoint");
	assert_eq!(color::lerp_oklab(0x336699ff, 0xff8000ff, 0.25), 0x6e7387ff, "steel/orange quarter");
	assert_eq!(color::lerp_oklab(0x11223300, 0x112233ff, 0.5), 0x11223380, "alpha midpoint");
}
