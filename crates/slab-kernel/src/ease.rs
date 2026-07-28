//! Easing curves from SPEC §14, originally defined by the research motion
//! model.
//!
//! Every curve clamps its input to `[0, 1]` before evaluation, matching the
//! reference easing behavior.

/// Clamps an easing input to the closed unit interval.
pub const fn clamp01(t: f64) -> f64 {
	0.0_f64.max(1.0_f64.min(t))
}

/// Applies linear easing to `t`.
pub const fn linear(t: f64) -> f64 {
	clamp01(t)
}

/// Applies quadratic ease-in to `t`.
pub fn ease_in(t: f64) -> f64 {
	let t = clamp01(t);
	t * t
}

/// Applies quadratic ease-out to `t`.
pub fn ease_out(t: f64) -> f64 {
	let t = clamp01(t);
	(1.0 - t).mul_add(-(1.0 - t), 1.0)
}

/// Applies symmetric quadratic ease-in-out to `t`.
pub fn ease_in_out(t: f64) -> f64 {
	let t = clamp01(t);
	if t < 0.5 {
		2.0 * t * t
	} else {
		(2.0 * (1.0 - t)).mul_add(-(1.0 - t), 1.0)
	}
}
