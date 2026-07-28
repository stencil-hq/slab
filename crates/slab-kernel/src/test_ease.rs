//! Known easing values, all exactly representable in `f64`.

use crate::ease;

/// Verifies linear easing and its endpoint clamping.
pub fn test_linear() {
	assert_eq!(ease::linear(0.25), 0.25, "linear(0.25) == 0.25");
	assert_eq!(ease::linear(-1.0), 0.0, "linear clamps below 0");
	assert_eq!(ease::linear(2.0), 1.0, "linear clamps above 1");
}

/// Verifies quadratic ease-in at its endpoints and midpoint.
pub fn test_ease_in() {
	assert_eq!(ease::ease_in(0.5), 0.25, "ease_in(0.5) == 0.25");
	assert_eq!(ease::ease_in(0.0), 0.0, "ease_in(0) == 0");
	assert_eq!(ease::ease_in(1.0), 1.0, "ease_in(1) == 1");
}

/// Verifies quadratic ease-out at its endpoints and midpoint.
pub fn test_ease_out() {
	assert_eq!(ease::ease_out(0.5), 0.75, "ease_out(0.5) == 0.75");
	assert_eq!(ease::ease_out(0.0), 0.0, "ease_out(0) == 0");
	assert_eq!(ease::ease_out(1.0), 1.0, "ease_out(1) == 1");
}

/// Verifies piecewise quadratic ease-in-out across both halves and endpoints.
pub fn test_ease_in_out() {
	assert_eq!(ease::ease_in_out(0.5), 0.5, "ease_in_out(0.5) == 0.5");
	assert_eq!(ease::ease_in_out(0.25), 0.125, "ease_in_out(0.25) == 0.125");
	assert_eq!(ease::ease_in_out(0.75), 0.875, "ease_in_out(0.75) == 0.875");
	assert_eq!(ease::ease_in_out(0.0), 0.0, "ease_in_out(0) == 0");
	assert_eq!(ease::ease_in_out(1.0), 1.0, "ease_in_out(1) == 1");
}
