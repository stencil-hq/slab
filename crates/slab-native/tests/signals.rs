//! The generated module's contract, kernel-only (no GPU): the embedded SLIR
//! decodes, a click on the Save button decodes to `Signal::Save`, and typing
//! into the focused field emits `Signal::Draft(text)`.

use slab_kernel::dispatch::{self as kdispatch, Event};
use slab_kernel::flatten::FrameOp;
use slab_native::gen_settings::{Doc, Signal};

fn ev(etype: u32, x: f64, y: f64) -> Event {
    Event {
        etype,
        x,
        y,
        dx: 0.0,
        dy: 0.0,
        button: 0,
        clicks: 0,
        key: String::new(),
        text: String::new(),
        mods: 0,
    }
}

#[test]
fn save_click_and_draft_change() {
    let mut doc = Doc::new();
    assert!(doc.ok(), "embedded SLIR failed to decode");
    doc.set_env(900.0, 640.0, false, false);
    let fr = doc.frame(0.0);

    // locate the Save button by its label
    let save = fr
        .ops
        .iter()
        .find_map(|op| match op {
            FrameOp::Text(t) if fr.strings[t.str_ref as usize] == "Save" => Some(t.clone()),
            _ => None,
        })
        .expect("no 'Save' text op");
    let (x, y) = (save.x + 2.0, save.y_baseline - 4.0);
    let (_, sigs) = doc.dispatch(&ev(kdispatch::E_POINTER_DOWN, x, y));
    assert!(sigs.is_empty(), "no signal before release, got {sigs:?}");
    let (_, sigs) = doc.dispatch(&ev(kdispatch::E_POINTER_UP, x, y));
    assert_eq!(sigs.len(), 1);
    assert!(matches!(&sigs[0], Signal::Save { item, .. } if item.is_empty()));

    // focus the field (its Text op carries field=draft) and type
    let field = fr
        .ops
        .iter()
        .find_map(|op| match op {
            FrameOp::Rect(r) if r.bg_kind == 1 && r.bg == 0xFF18_100C => Some(r.clone()),
            _ => None,
        })
        .expect("no field rect (#0C1018)");
    let (fx, fy) = (field.x + 10.0, field.y + field.h / 2.0);
    doc.dispatch(&ev(kdispatch::E_POINTER_DOWN, fx, fy));
    doc.dispatch(&ev(kdispatch::E_POINTER_UP, fx, fy));
    let mut text_ev = ev(kdispatch::E_TEXT, fx, fy);
    text_ev.text = "hi".into();
    let (_, sigs) = doc.dispatch(&text_ev);
    assert_eq!(sigs.len(), 1);
    assert!(
        matches!(&sigs[0], Signal::Draft { text, item, .. } if text == "hi" && item.is_empty())
    );

    // typed setter round-trips through inst_set_param
    let mut doc2 = Doc::new();
    doc2.set_env(900.0, 640.0, false, false);
    assert!(doc2.set_title("Prefs"));
    assert!(doc2.set_compact(true));
    let fr2 = doc2.frame(1.0);
    assert!(
        fr2.strings.iter().any(|s| s == "Prefs"),
        "param.title override did not reach the frame"
    );
}
