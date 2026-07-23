//! Color and paint parsing, ported from the 0.5 reference implementation.
//! Colors: `#rgb/#rgba/#rrggbb/#rrggbbaa`, `rgb()/rgba()`, `oklch()`, and the
//! tiny named set. Paints: a solid color or `linear(angle, color stop%, ...)`
//! / `radial(...)` / `conic(from, color stop%, ...)` gradients.

pub type Rgba = [u8; 4];

fn srgb(x: f64) -> f64 {
    if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    }
}

fn oklab_to_rgb(l: f64, a: f64, b: f64) -> [u8; 3] {
    let l_ = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m_ = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s_ = (l - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);
    let r = 4.076_741_662_1 * l_ - 3.307_711_591_3 * m_ + 0.230_969_929_2 * s_;
    let g = -1.268_438_004_6 * l_ + 2.609_757_401_1 * m_ - 0.341_319_396_5 * s_;
    let bl = -0.004_196_086_3 * l_ - 0.703_418_614_7 * m_ + 1.707_614_701_0 * s_;
    let to8 = |v: f64| (srgb(v.max(0.0)) * 255.0).round().clamp(0.0, 255.0) as u8;
    [to8(r), to8(g), to8(bl)]
}

fn split_ws(s: &str) -> Vec<&str> {
    s.split(|c: char| c == ',' || c == '/' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect()
}

fn num_pct(p: &str, pct_scale: f64) -> Option<f64> {
    if let Some(stripped) = p.strip_suffix('%') {
        stripped.parse::<f64>().ok().map(|v| v * pct_scale)
    } else {
        p.parse::<f64>().ok()
    }
}

/// Parse a solid slab color string into straight-alpha RGBA.
/// `none`/`transparent` and unparseable input both return `None`.
pub fn parse_rgba(s: &str) -> Option<Rgba> {
    let s = s.trim();
    if s.is_empty() || s == "none" || s == "transparent" {
        return None;
    }
    match s {
        "white" => return Some([255, 255, 255, 255]),
        "black" => return Some([0, 0, 0, 255]),
        _ => {}
    }
    if let Some(h) = s.strip_prefix('#') {
        let mut h: String = h.to_string();
        if h.len() == 3 || h.len() == 4 {
            h = h.chars().flat_map(|c| [c, c]).collect();
        }
        if h.len() == 6 {
            h.push_str("ff");
        }
        if h.len() == 8 {
            let b = u32::from_str_radix(&h, 16).ok()?;
            return Some([(b >> 24) as u8, (b >> 16) as u8, (b >> 8) as u8, b as u8]);
        }
        return None;
    }
    if let Some(inner) = s
        .strip_prefix("rgba(")
        .or_else(|| s.strip_prefix("rgb("))
        .and_then(|r| r.strip_suffix(')'))
    {
        let parts = split_ws(inner);
        if parts.len() < 3 {
            return None;
        }
        let mut rgb = [0u8; 3];
        for (i, p) in parts.iter().take(3).enumerate() {
            let v = num_pct(p, 2.55)?;
            rgb[i] = (v.min(255.0)) as u8;
        }
        let mut a = 255u8;
        if let Some(p) = parts.get(3) {
            let v = if p.ends_with('%') {
                num_pct(p, 2.55)?
            } else {
                p.parse::<f64>().ok()? * 255.0
            };
            a = (v.round().min(255.0)) as u8;
        }
        return Some([rgb[0], rgb[1], rgb[2], a]);
    }
    if let Some(inner) = s.strip_prefix("oklch(").and_then(|r| r.strip_suffix(')')) {
        let parts = split_ws(inner);
        if parts.len() < 3 {
            return None;
        }
        let l = if parts[0].ends_with('%') {
            parts[0].trim_end_matches('%').parse::<f64>().ok()? / 100.0
        } else {
            parts[0].parse::<f64>().ok()?
        };
        let c: f64 = parts[1].parse().ok()?;
        let h: f64 = parts[2].parse().ok()?;
        let rgb = oklab_to_rgb(l, c * h.to_radians().cos(), c * h.to_radians().sin());
        let mut a = 255u8;
        if let Some(p) = parts.get(3) {
            let v = if p.ends_with('%') {
                num_pct(p, 2.55)?
            } else {
                p.parse::<f64>().ok()? * 255.0
            };
            a = (v.round().min(255.0)) as u8;
        }
        return Some([rgb[0], rgb[1], rgb[2], a]);
    }
    None
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    /// 0..=1
    pub offset: f64,
    pub rgba: Rgba,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    Solid(Rgba),
    /// Angle in degrees, 0 = up, 90 = right (CSS convention).
    Linear {
        angle: f64,
        stops: Vec<Stop>,
    },
    /// Centered, radius = half-diagonal (cover).
    Radial {
        stops: Vec<Stop>,
    },
    /// Centered sweep, clockwise from `from` degrees (0 = up, CSS convention).
    Conic {
        from: f64,
        stops: Vec<Stop>,
    },
    /// `none` / `transparent`.
    None,
}

fn split_top(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in s.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if ch == ',' && depth == 0 {
            out.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

fn parse_stops(parts: &[String]) -> Option<Vec<Stop>> {
    let mut raw: Vec<(Rgba, Option<f64>)> = Vec::new();
    for p in parts {
        let mut color_s = p.as_str();
        let mut off = None;
        if let Some((head, tail)) = p.rsplit_once(' ')
            && tail.ends_with('%')
        {
            off = Some(tail.trim_end_matches('%').parse::<f64>().ok()? / 100.0);
            color_s = head.trim_end();
        }
        let c = parse_rgba(color_s)?;
        raw.push((c, off));
    }
    if raw.is_empty() {
        return None;
    }
    if raw[0].1.is_none() {
        raw[0].1 = Some(0.0);
    }
    let last = raw.len() - 1;
    if raw[last].1.is_none() {
        raw[last].1 = Some(1.0);
    }
    // fill missing offsets: interiors spread evenly between neighbors
    let mut i = 0;
    while i < raw.len() {
        if raw[i].1.is_none() {
            let mut j = i;
            while raw[j].1.is_none() {
                j += 1;
            }
            let lo = raw[i - 1].1.unwrap();
            let hi = raw[j].1.unwrap();
            let n = (j - i + 1) as f64;
            for (step, item) in raw[i..j].iter_mut().enumerate() {
                item.1 = Some(lo + (hi - lo) * (step as f64 + 1.0) / n);
            }
            i = j;
        }
        i += 1;
    }
    Some(
        raw.into_iter()
            .map(|(c, o)| Stop {
                offset: o.unwrap().clamp(0.0, 1.0),
                rgba: c,
            })
            .collect(),
    )
}

/// A solid, a gradient paint, `Paint::None` for none/transparent, or `None`
/// (outer Option) when the string does not parse as a paint at all.
pub fn parse_paint(s: &str) -> Option<Paint> {
    let s = s.trim();
    if s == "none" || s == "transparent" {
        return Some(Paint::None);
    }
    let (kind, inner) = if let Some(r) = s.strip_prefix("linear(").and_then(|r| r.strip_suffix(')'))
    {
        ("linear", r)
    } else if let Some(r) = s.strip_prefix("radial(").and_then(|r| r.strip_suffix(')')) {
        ("radial", r)
    } else if let Some(r) = s.strip_prefix("conic(").and_then(|r| r.strip_suffix(')')) {
        ("conic", r)
    } else {
        return parse_rgba(s).map(Paint::Solid);
    };
    let mut parts = split_top(inner);
    let mut angle = 180.0f64; // default: top -> bottom
    if kind == "linear"
        && let Some(first) = parts.first()
        && let Ok(a) = first.parse::<f64>()
    {
        angle = a;
        parts.remove(0);
    }
    if kind == "conic" {
        // the from-angle is required; a leading color is a parse failure
        angle = parts.first()?.parse::<f64>().ok()?;
        parts.remove(0);
    }
    let stops = parse_stops(&parts)?;
    if stops.is_empty() {
        return None;
    }
    Some(match kind {
        "linear" => Paint::Linear {
            angle: angle.rem_euclid(360.0),
            stops,
        },
        "conic" => Paint::Conic {
            from: angle.rem_euclid(360.0),
            stops,
        },
        _ => Paint::Radial { stops },
    })
}

/// rgba8 word: byte 0 = r, byte 1 = g, byte 2 = b, byte 3 = a.
pub fn rgba_word(c: Rgba) -> u32 {
    u32::from_le_bytes(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_forms() {
        assert_eq!(parse_rgba("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(parse_rgba("#0e1116"), Some([14, 17, 22, 255]));
        assert_eq!(parse_rgba("#00000080"), Some([0, 0, 0, 128]));
        assert_eq!(parse_rgba("none"), None);
    }

    #[test]
    fn gradient_stops_spread() {
        let p = parse_paint("linear(90, #000 0%, #fff)").unwrap();
        match p {
            Paint::Linear { angle, stops } => {
                assert_eq!(angle, 90.0);
                assert_eq!(stops[0].offset, 0.0);
                assert_eq!(stops[1].offset, 1.0);
            }
            _ => panic!("not linear"),
        }
    }

    #[test]
    fn conic_requires_angle() {
        assert_eq!(parse_paint("conic(#000,#fff)"), None);
        assert_eq!(parse_paint("conic(#000 0%, #fff 100%)"), None);
    }

    #[test]
    fn conic_parses() {
        let p = parse_paint("conic(45, #000, #fff)").unwrap();
        match p {
            Paint::Conic { from, stops } => {
                assert_eq!(from, 45.0);
                assert_eq!(stops[0].offset, 0.0);
                assert_eq!(stops[1].offset, 1.0);
            }
            _ => panic!("not conic"),
        }
    }

    #[test]
    fn conic_from_normalizes() {
        match parse_paint("conic(-90, #000, #fff)").unwrap() {
            Paint::Conic { from, .. } => assert_eq!(from, 270.0),
            _ => panic!("not conic"),
        }
    }

    #[test]
    fn oklch_parses() {
        // oklch(72% 0.16 250) — the SPEC §10 example accent
        let c = parse_rgba("oklch(72% 0.16 250)").unwrap();
        assert_eq!(c[3], 255);
        assert!(c[2] > c[0], "hue 250 should lean blue: {c:?}");
    }
}
