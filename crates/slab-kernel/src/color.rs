//! Conversion between packed `0xRRGGBBAA` sRGB colors and `OKLab`, including
//! perceptual interpolation for animation.
//!
//! The conversion was originally ported from the `slab/color.py` research
//! prototype.
/// A color in the perceptually uniform `OKLab` color space.
#[derive(Clone, Debug)]
pub struct Lab {
	/// Perceived lightness.
	pub l: f64,
	/// Green–red opponent axis.
	pub a: f64,
	/// Blue–yellow opponent axis.
	pub b: f64,
}

/// Implements Rust's saturating float-to-unsigned-integer cast without `as`.
fn truncate_u32(value: f64) -> u32 {
	if value.is_nan() || value <= 0.0 {
		return 0;
	}
	if value >= f64::from(u32::MAX) {
		return u32::MAX;
	}

	let bits = value.to_bits();
	let exponent = i32::try_from((bits >> 52) & 0x7ff).expect("f64 exponent fits i32") - 1023;
	if exponent < 0 {
		return 0;
	}
	let significand = (bits & ((1_u64 << 52) - 1)) | (1_u64 << 52);
	let magnitude = significand >> u32::try_from(52 - exponent).expect("nonnegative right shift");
	u32::try_from(magnitude).expect("bounded f64 magnitude fits u32")
}

/// Converts an encoded 8-bit sRGB channel to linear light in the range 0–1.
pub fn linear_of(c: f64) -> f64 {
	let encoded = c / 255.0;
	if encoded <= 0.04045 {
		encoded / 12.92
	} else {
		((encoded + 0.055) / 1.055).powf(2.4)
	}
}

/// Converts a linear-light channel in the range 0–1 to encoded sRGB.
pub fn srgb_of(linear: f64) -> f64 {
	if linear <= 0.0031308 {
		12.92 * linear
	} else {
		1.055f64.mul_add(linear.powf(1.0 / 2.4), -0.055)
	}
}

/// Encodes a linear-light channel as an 8-bit sRGB channel, clamped to 0–255.
pub fn to8(linear: f64) -> u32 {
	let encoded = srgb_of(linear.max(0.0)) * 255.0;
	truncate_u32(encoded.round().clamp(0.0, 255.0))
}

/// Converts a packed `0xRRGGBBAA` sRGB color to `OKLab`.
pub fn oklab_from_rgba(rgba: u32) -> Lab {
	let red = linear_of(f64::from(rgba.wrapping_shr(24) & 0xff));
	let green = linear_of(f64::from(rgba.wrapping_shr(16) & 0xff));
	let blue = linear_of(f64::from(rgba.wrapping_shr(8) & 0xff));

	let l = 0.0514459929f64.mul_add(blue, 0.5363325363f64.mul_add(green, 0.4122214708 * red));
	let m = 0.1073969566f64.mul_add(blue, 0.6806995451f64.mul_add(green, 0.2119034982 * red));
	let s = 0.6299787005f64.mul_add(blue, 0.2817188376f64.mul_add(green, 0.0883024619 * red));
	let l_root = l.cbrt();
	let m_root = m.cbrt();
	let s_root = s.cbrt();

	Lab {
		l: 0.0040720468f64.mul_add(-s_root, 0.793617785f64.mul_add(m_root, 0.2104542553 * l_root)),
		a: 0.4505937099f64.mul_add(s_root, 2.428592205f64.mul_add(-m_root, 1.9779984951 * l_root)),
		b: 0.808675766f64.mul_add(-s_root, 0.7827717662f64.mul_add(m_root, 0.0259040371 * l_root)),
	}
}

/// Converts an `OKLab` color to packed `0xRRGGBBAA` sRGB.
///
/// The alpha value is masked to its low eight bits.
pub fn rgba_from_oklab(lab: &Lab, alpha: u32) -> u32 {
	let l_root = 0.2158037573f64.mul_add(lab.b, 0.3963377774f64.mul_add(lab.a, lab.l));
	let m_root = 0.0638541728f64.mul_add(-lab.b, 0.1055613458f64.mul_add(-lab.a, lab.l));
	let s_root = 1.291485548f64.mul_add(-lab.b, 0.0894841775f64.mul_add(-lab.a, lab.l));
	let l = (l_root * l_root) * l_root;
	let m = (m_root * m_root) * m_root;
	let s = (s_root * s_root) * s_root;

	let red = 0.2309699292f64.mul_add(s, 3.3077115913f64.mul_add(-m, 4.0767416621 * l));
	let green = 0.3413193965f64.mul_add(-s, 2.6097574011f64.mul_add(m, -1.2684380046 * l));
	let blue = 1.707614701f64.mul_add(s, 0.7034186147f64.mul_add(-m, -0.0041960863 * l));

	to8(red).wrapping_shl(24)
		| to8(green).wrapping_shl(16)
		| to8(blue).wrapping_shl(8)
		| (alpha & 0xff)
}

/// Perceptually interpolates two colors for animation, with linear alpha.
pub fn lerp_oklab(c1: u32, c2: u32, t: f64) -> u32 {
	let from = oklab_from_rgba(c1);
	let to = oklab_from_rgba(c2);
	let mixed = Lab {
		l: (to.l - from.l).mul_add(t, from.l),
		a: (to.a - from.a).mul_add(t, from.a),
		b: (to.b - from.b).mul_add(t, from.b),
	};
	let from_alpha = f64::from(c1 & 0xff);
	let to_alpha = f64::from(c2 & 0xff);
	let alpha = (to_alpha - from_alpha).mul_add(t, from_alpha).round();

	rgba_from_oklab(&mixed, truncate_u32(alpha))
}
