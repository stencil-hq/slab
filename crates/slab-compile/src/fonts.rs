//! FONT metric table subsetting from shared runtime-font metrics.

use slab_fonts::{FontAsset, parse_metrics};
use slab_slir::FontE;
use std::collections::BTreeSet;

/// Build a FONT metric table subset to `cps`.
pub fn build_table(a: &FontAsset, cps: &BTreeSet<u32>) -> FontE {
    let metrics = parse_metrics(a.bytes).expect("vendored font parses");
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
        class: a.class,
        weight: a.weight,
        upem: metrics.upem,
        ascent: metrics.ascent,
        descent: metrics.descent,
        line_gap: metrics.line_gap,
        default_advance: metrics.default_advance,
        cmap,
        advances,
    }
}
