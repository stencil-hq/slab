//! Tests for [`fmt3`], the canonical `frame.json` number format.
//!
//! Expectations use the same binary-f64 algorithm as the implementation. The
//! decimal-looking surprises (`1.0005` becoming `"1"`, while `2.0005` becomes
//! `"2.001"`) reflect the numbers' actual f64 values.

use crate::value::fmt3;

/// Verifies integer formatting without a decimal suffix.
pub fn test_fmt3_integers() {
    assert_eq!(fmt3(0.0), "0", "0 -> 0");
    assert_eq!(fmt3(240.0), "240", "240");
    assert_eq!(fmt3(10_000_000.0), "10000000", "1e7");
    assert_eq!(fmt3(-8.0), "-8", "-8");
}

/// Verifies that values rounded to negative zero are emitted as zero.
pub fn test_fmt3_negzero() {
    assert_eq!(fmt3(-0.0001), "0", "-0.0001 rounds to -0 -> 0");
    assert_eq!(fmt3(-0.0), "0", "-0 -> 0");
}

/// Verifies round-half-even behavior at three decimal places.
pub fn test_fmt3_half_even() {
    assert_eq!(fmt3(0.0005), "0", "0.0005 -> 0 (half to even)");
    assert_eq!(fmt3(1.0005), "1", "1.0005 (f64 below half) -> 1");
    assert_eq!(fmt3(2.0005), "2.001", "2.0005 (f64 above half) -> 2.001");
    assert_eq!(fmt3(0.00025), "0", "0.00025 -> 0");
}

/// Verifies trailing-zero trimming and three-decimal rounding.
#[allow(clippy::approx_constant)]
pub fn test_fmt3_trim() {
    assert_eq!(fmt3(0.75), "0.75", "trailing zero trimmed");
    assert_eq!(fmt3(0.3), "0.3", "0.3");
    assert_eq!(fmt3(23.4567), "23.457", "3-decimal round");
    assert_eq!(fmt3(1.2345), "1.234", "1.2345 -> 1.234");
    assert_eq!(fmt3(-3.14159), "-3.142", "negative rounds");
    assert_eq!(fmt3(0.1 + 0.2), "0.3", "0.1+0.2 -> 0.3");
}
