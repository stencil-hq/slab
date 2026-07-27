//! FONT metric table subsetting from shared runtime-font metrics.

use slab_fonts::parse_metrics;
use slab_slir::FontE;
use std::collections::BTreeSet;

/// Build a FONT metric table for one face, subset to `cps`.
///
/// `class` supplies fallback metrics classification and `weight` is the
/// weight this table represents in the document, independent of the weight
/// declared inside `bytes`.
pub fn build_table(class: u8, weight: u16, bytes: &[u8], cps: &BTreeSet<u32>) -> FontE {
    let metrics = parse_metrics(bytes).expect("registered font parses");
    let mut cmap = Vec::new();
    let mut advances = Vec::new();
    for (&cp, (&gid, &advance)) in metrics
        .cps
        .iter()
        .zip(metrics.gids.iter().zip(metrics.advances.iter()))
    {
        if cps.contains(&cp) {
            cmap.push((cp, u16::try_from(gid).expect("vendored glyph ID fits u16")));
            advances.push(u16::try_from(advance).expect("vendored advance fits u16"));
        }
    }
    FontE {
        family: 0,
        class,
        weight,
        upem: metrics.upem,
        ascent: metrics.ascent,
        descent: metrics.descent,
        line_gap: metrics.line_gap,
        default_advance: metrics.default_advance,
        cmap,
        advances,
    }
}
