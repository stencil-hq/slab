//! `slir-dump`: canonical, line-oriented text rendering of every SLIR
//! section, used for conformance goldens and debugging. Floats format
//! round-half-even to 3 decimals; `-0` prints as `0`.

use crate::{Slir, attrs, aval, cond, flags, kind};
use std::fmt::Write as _;

/// Round-half-even to 3 decimals, `-0 -> 0`, trailing zeros trimmed.
pub fn fmt_f(x: f64) -> String {
    if !x.is_finite() {
        return if x.is_nan() {
            "nan".into()
        } else if x > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    let scaled = x * 1000.0;
    if scaled.abs() >= 9.0e15 {
        return format!("{x}");
    }
    let m = scaled.round_ties_even() as i64;
    if m == 0 {
        return "0".into();
    }
    let sign = if m < 0 { "-" } else { "" };
    let m = m.unsigned_abs();
    let int = m / 1000;
    let frac = m % 1000;
    if frac == 0 {
        format!("{sign}{int}")
    } else {
        let s = format!("{frac:03}");
        format!("{sign}{int}.{}", s.trim_end_matches('0'))
    }
}

/// `#rrggbb` / `#rrggbbaa` from an rgba8 word (byte 0 = r .. byte 3 = a).
fn fmt_rgba(v: u32) -> String {
    let [r, g, b, a] = v.to_le_bytes();
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

fn fmt_flags(f: u16) -> String {
    if f == 0 {
        return "-".into();
    }
    let mut names = Vec::new();
    for &(bit, name) in &flags::NAMES {
        if f & bit != 0 {
            names.push(name);
        }
    }
    names.join("+")
}

fn fmt_link(v: u32) -> String {
    if v == crate::NONE {
        "-".into()
    } else {
        v.to_string()
    }
}

fn attr_label(id: u16) -> String {
    attrs::attr_name(id)
        .map(str::to_string)
        .unwrap_or_else(|| format!("attr{id}"))
}

fn fmt_attr_run(pool: &[(u16, u32)]) -> String {
    if pool.is_empty() {
        return "-".into();
    }
    pool.iter()
        .map(|&(a, v)| format!("{}=@{v}", attr_label(a)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn aval_line(s: &Slir, a: &crate::Aval) -> String {
    let name = aval::NAMES
        .get(a.tag as usize)
        .copied()
        .unwrap_or("Unknown");
    match a.tag {
        aval::NUM | aval::PCT | aval::SIZE_FIXED | aval::SIZE_FILL | aval::SIZE_PCT => {
            format!("{name} {}", fmt_f(a.as_f64()))
        }
        aval::STR | aval::ENUM_SYM => format!("{name} {:?}", s.str_at(a.lo())),
        aval::COLOR | aval::PAINT_SOLID => format!("{name} {}", fmt_rgba(a.lo())),
        aval::TUPLE => {
            let off = a.lo() as usize;
            let len = a.hi() as usize;
            let items: Vec<String> = s.f64s[off..off + len].iter().map(|&f| fmt_f(f)).collect();
            format!("{name} [{}]", items.join(", "))
        }
        aval::TUPLE_DYN => {
            let off = a.lo() as usize;
            let len = a.hi() as usize;
            let items: Vec<String> = s.tup_dyn[off..off + len]
                .iter()
                .map(|member| match member {
                    crate::TupDynE::Lit(value) => fmt_f(*value),
                    crate::TupDynE::Param(param) => format!("param:{param}"),
                })
                .collect();
            format!("{name} [{}]", items.join(", "))
        }
        aval::SIZE_HUG | aval::PAINT_NONE => name.to_string(),
        aval::PAINT_GRADIENT | aval::PATH_REF | aval::PARAM_REF | aval::PROP_REF => {
            format!("{name} {}", a.lo())
        }
        aval::SHADOW_LIST | aval::LIST_DEFAULT => format!("{name} @{}+{}", a.lo(), a.hi()),
        _ => format!("{name} payload={:#x}", a.payload),
    }
}

/// Render the canonical dump of a decoded SLIR document.
pub fn dump(s: &Slir) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "slir {}.{}", crate::MAJOR, crate::MINOR);

    let _ = writeln!(o, "STRS {}", s.strs.len());
    for (i, st) in s.strs.iter().enumerate() {
        let _ = writeln!(o, "  {i} {st:?}");
    }

    let _ = writeln!(o, "NODE {}", s.nodes.len());
    for i in 0..s.nodes.len() {
        let n = &s.nodes;
        let _ = writeln!(
            o,
            "  {i} {} flags={} parent={} child={} sib={} key={:?} id={} line={}",
            kind::NAMES
                .get(n.kind[i] as usize)
                .copied()
                .unwrap_or("Unknown"),
            fmt_flags(n.flags[i]),
            fmt_link(n.parent[i]),
            fmt_link(n.first_child[i]),
            fmt_link(n.next_sib[i]),
            s.str_at(n.key[i]),
            if n.id[i] == 0 {
                "-".to_string()
            } else {
                format!("{:?}", s.str_at(n.id[i]))
            },
            n.src_line[i],
        );
    }

    let tup_dyn_note = if s.tup_dyn.is_empty() {
        String::new()
    } else {
        format!(" tup_dyn={}", s.tup_dyn.len())
    };
    let _ = writeln!(
        o,
        "AVAL {} f64s={}{tup_dyn_note}",
        s.avals.len(),
        s.f64s.len()
    );
    for (i, a) in s.avals.iter().enumerate() {
        let _ = writeln!(o, "  {i} {}", aval_line(s, a));
    }

    let _ = writeln!(o, "GRAD {}", s.grads.len());
    for (i, g) in s.grads.iter().enumerate() {
        let stops: Vec<String> = g
            .stops
            .iter()
            .map(|&(p, c)| format!("{}:{}", fmt_f(p), fmt_rgba(c)))
            .collect();
        let _ = writeln!(
            o,
            "  {i} {} angle={} stops [{}]",
            match g.kind {
                0 => "linear",
                2 => "conic",
                _ => "radial",
            },
            fmt_f(g.angle),
            stops.join(" ")
        );
    }

    let _ = writeln!(o, "SHDW {}", s.shadows.len());
    for (i, sh) in s.shadows.iter().enumerate() {
        let _ = writeln!(
            o,
            "  {i} x={} y={} blur={} spread={} color={} inset={}",
            fmt_f(sh.x),
            fmt_f(sh.y),
            fmt_f(sh.blur),
            fmt_f(sh.spread),
            fmt_rgba(sh.rgba),
            sh.inset
        );
    }

    let _ = writeln!(o, "ATTR {}", s.attrs.len());
    for node in 0..s.nodes.len() {
        let run = s.node_attrs(node as u32);
        let _ = writeln!(o, "  {node} {}", fmt_attr_run(run));
    }

    let _ = writeln!(o, "PATH {}", s.paths.len());
    for (i, p) in s.paths.iter().enumerate() {
        let verbs: String = p
            .verbs
            .iter()
            .map(|&v| {
                ["M", "L", "C", "Q", "Z"]
                    .get(v as usize)
                    .copied()
                    .unwrap_or("?")
            })
            .collect();
        let coords: Vec<String> = p.coords.iter().map(|&c| fmt_f(c)).collect();
        let _ = writeln!(o, "  {i} verbs={verbs} coords [{}]", coords.join(", "));
    }

    let _ = writeln!(o, "FONT {}", s.fonts.len());
    for (i, f) in s.fonts.iter().enumerate() {
        let _ = writeln!(
            o,
            "  {i} family={:?} class={} weight={} upem={} ascent={} descent={} line_gap={} default_advance={}",
            s.str_at(f.family),
            if f.class == 0 { "sans" } else { "mono" },
            f.weight,
            f.upem,
            f.ascent,
            f.descent,
            f.line_gap,
            f.default_advance
        );
        let mut line = String::from("    cmap");
        for (j, &(cp, gid)) in f.cmap.iter().enumerate() {
            let adv = f.advances.get(j).copied().unwrap_or(0);
            let _ = write!(line, " {cp}:{gid}:{adv}");
            if (j + 1) % 8 == 0 {
                let _ = writeln!(o, "{line}");
                line = String::from("    cmap");
            }
        }
        if line != "    cmap" {
            let _ = writeln!(o, "{line}");
        }
    }

    let _ = writeln!(
        o,
        "WHEN conds={} patches={} attrs={} children={}",
        s.conds.len(),
        s.patches.len(),
        s.patch_attrs.len(),
        s.patch_children.len()
    );
    for (i, c) in s.conds.iter().enumerate() {
        let kindname = cond::NAMES.get(c.kind as usize).copied().unwrap_or("?");
        let neg = if c.neg != 0 { "!" } else { "" };
        let body = match c.kind {
            cond::WCMP | cond::HCMP => format!(
                "{} {}",
                cond::OPS.get(c.op as usize).copied().unwrap_or("?"),
                fmt_f(c.num)
            ),
            _ => format!("{:?}", s.str_at(c.sym)),
        };
        let _ = writeln!(o, "  cond {i} {neg}{kindname} {body}");
    }
    for (i, p) in s.patches.iter().enumerate() {
        let attrs_run = &s.patch_attrs[p.attr_off as usize..(p.attr_off + p.attr_len) as usize];
        let children: Vec<String> = s.patch_children
            [p.child_off as usize..(p.child_off + p.child_len) as usize]
            .iter()
            .map(u32::to_string)
            .collect();
        let _ = writeln!(
            o,
            "  patch {i} node={} cond={} attrs [{}] children [{}]",
            p.node,
            p.cond,
            fmt_attr_run(attrs_run),
            children.join(" ")
        );
    }

    let _ = writeln!(
        o,
        "ANIM anims={} attrs={} binds={} trans={}",
        s.anims.len(),
        s.anim_attrs.len(),
        s.bindings.len(),
        s.transitions.len()
    );
    for (i, a) in s.anims.iter().enumerate() {
        let stops: Vec<String> = a
            .stops
            .iter()
            .map(|&(pos, off, len)| {
                let run = &s.anim_attrs[off as usize..(off + len) as usize];
                format!("{}:[{}]", fmt_f(pos), fmt_attr_run(run))
            })
            .collect();
        let _ = writeln!(
            o,
            "  anim {i} name={:?} stops {}",
            s.str_at(a.name),
            stops.join(" ")
        );
    }
    const MODES: [&str; 3] = ["loop", "once", "alternate"];
    const EASINGS: [&str; 4] = ["linear", "ease-in", "ease-out", "ease-in-out"];
    for (i, b) in s.bindings.iter().enumerate() {
        let _ = writeln!(
            o,
            "  bind {i} node={} anim={} dur={} mode={} easing={} delay={}",
            b.node,
            b.anim,
            fmt_f(b.dur),
            MODES.get(b.mode as usize).copied().unwrap_or("?"),
            EASINGS.get(b.easing as usize).copied().unwrap_or("?"),
            fmt_f(b.delay)
        );
    }
    for (i, t) in s.transitions.iter().enumerate() {
        let _ = writeln!(
            o,
            "  trans {i} node={} dur={} easing={} delay={}",
            t.node,
            fmt_f(t.dur),
            EASINGS.get(t.easing as usize).copied().unwrap_or("?"),
            fmt_f(t.delay)
        );
    }

    let _ = writeln!(
        o,
        "PARM {} enums={} sites={}",
        s.params.len(),
        s.param_enum_syms.len(),
        s.param_sites.len()
    );
    for (i, p) in s.params.iter().enumerate() {
        let enums: Vec<String> = s.param_enum_syms
            [p.enum_off as usize..(p.enum_off + p.enum_len) as usize]
            .iter()
            .map(|&e| s.str_at(e).to_string())
            .collect();
        let sites: Vec<String> = s.param_sites
            [p.site_off as usize..(p.site_off + p.site_len) as usize]
            .iter()
            .map(|&(node, attr)| format!("{node}:{}", attr_label(attr)))
            .collect();
        let _ = writeln!(
            o,
            "  {i} name={:?} type={} default=@{} enums [{}] sites [{}]",
            s.str_at(p.name),
            crate::PARAM_TYPE_NAMES
                .get(p.ty as usize)
                .copied()
                .unwrap_or("?"),
            p.default,
            enums.join(" "),
            sites.join(" ")
        );
    }

    let _ = writeln!(
        o,
        "LIST {} fields={} enums={} items={} values={}",
        s.lists.len(),
        s.list_fields.len(),
        s.list_enum_syms.len(),
        s.list_items.len(),
        s.list_item_values.len()
    );
    for (i, list) in s.lists.iter().enumerate() {
        let _ = writeln!(
            o,
            "  {i} param={} fields=@{}+{}",
            list.param, list.field_off, list.field_len
        );
    }
    for (i, field) in s.list_fields.iter().enumerate() {
        let enums: Vec<String> = s.list_enum_syms
            [field.enum_off as usize..(field.enum_off + field.enum_len) as usize]
            .iter()
            .map(|&sym| format!("{:?}", s.str_at(sym)))
            .collect();
        let _ = writeln!(
            o,
            "  field {i} name={:?} type={} default=@{} sub={} enums [{}]",
            s.str_at(field.name),
            crate::PARAM_TYPE_NAMES
                .get(field.ty as usize)
                .copied()
                .unwrap_or("?"),
            field.default,
            field.sub,
            enums.join(" ")
        );
    }
    for (i, item) in s.list_items.iter().enumerate() {
        let fields: Vec<String> = s.list_item_values
            [item.field_off as usize..(item.field_off + item.field_len) as usize]
            .iter()
            .map(|field| format!("{}=@{}", field.field, field.val))
            .collect();
        let _ = writeln!(o, "  item {i} [{}]", fields.join(" "));
    }

    let _ = writeln!(o, "ICON {}", s.icons.len());
    for (i, icon) in s.icons.iter().enumerate() {
        let _ = writeln!(
            o,
            "  {i} name={:?} node={} viewbox={}",
            s.str_at(icon.name),
            icon.node,
            fmt_f(icon.viewbox)
        );
    }

    let _ = writeln!(o, "HOLE {}", s.holes.len());
    for (i, &(name, node)) in s.holes.iter().enumerate() {
        let _ = writeln!(o, "  {i} name={:?} node={node}", s.str_at(name));
    }

    let _ = writeln!(o, "SIGN {}", s.signals.len());
    for (i, &(name, node, trigger)) in s.signals.iter().enumerate() {
        let _ = writeln!(
            o,
            "  {i} name={:?} node={node} trigger={}",
            s.str_at(name),
            match trigger {
                0 => "activate",
                1 => "change",
                2 => "submit",
                3 => "press",
                4 => "context",
                5 => "dblclick",
                6 => "drag-start",
                7 => "drop",
                8 => "resize",
                9 => "pointer-move",
                10 => "pointer-up",
                11 => "drag-update",
                12 => "drag-end",
                13 => "activate",
                14 => "cancel",
                _ => "?",
            }
        );
    }

    let _ = writeln!(o, "THEM {}", s.themes.len());
    for (i, &name) in s.themes.iter().enumerate() {
        let _ = writeln!(o, "  {i} name={:?}", s.str_at(name));
    }

    let _ = writeln!(o, "TOKN {}", s.tokens.len());
    for (index, token) in s.tokens.iter().enumerate() {
        let _ = writeln!(
            o,
            "  {index} name={:?} base=@{} repr={:?}",
            s.str_at(token.name),
            token.base,
            s.str_at(token.base_repr)
        );
        for &(theme, value, repr) in &token.themes {
            let _ = writeln!(
                o,
                "    theme={:?} val=@{} repr={:?}",
                s.str_at(theme),
                value,
                s.str_at(repr)
            );
        }
    }

    let _ = writeln!(o, "IMGS {}", s.images.len());
    for (i, im) in s.images.iter().enumerate() {
        let _ = writeln!(
            o,
            "  {i} src={:?} w={} h={} format={} blob=@{}+{}",
            s.str_at(im.src),
            im.w,
            im.h,
            if im.format == 0 { "png" } else { "?" },
            im.blob_off,
            im.blob_len
        );
    }

    let _ = writeln!(
        o,
        "BLOB image-bytes={} fnv1a64={:016x}",
        s.blob.len(),
        fnv1a64(&s.blob)
    );
    o
}
