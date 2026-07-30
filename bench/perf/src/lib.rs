//! Reusable editor-scale kernel scenarios for performance measurements.

use slab_kernel::{
    dispatch::{E_KEY_DOWN, E_TEXT, E_WHEEL, Event},
    frame::{
        Instance, ParamValue, inst_dispatch, inst_frame, inst_focus_note, inst_set_caret,
        inst_set_env, inst_set_field_text, inst_set_focus, inst_set_list_field, inst_set_list_key,
        inst_set_list_len, inst_set_scroll,
    },
};

const WIDTH: f64 = 900.0;
const HEIGHT: f64 = 700.0;
const CLIENT_GPU: u32 = 1;
const EDITOR_FIELD: &str = "editor";
const BLOCKS_PARAM: u32 = 0;

const EDITOR_DOC: &str = r#"
col#editor-scroll w=fill h=fill scroll pad=24 {
  text#editor "" w=fill field=editor field-sync=host multiline size=14 leading=20
}
"#;

const BLOCKS_DOC: &str = r#"
params {
  blocks list(Block) = []
}
def Block(text="") export {
  row#row w=fill key=key pad=4,0 {
    text#field text w=fill field=key field-sync=host multiline size=14 leading=20
  }
}
col#blocks-scroll w=fill h=fill scroll pad=24 {
  each param.blocks #blocks-list
}
"#;

/// Produces deterministic pseudo-code with varied line lengths and Unicode.
///
/// The result contains exactly `n` logical lines (and no trailing newline),
/// spanning empty, short, normal, wide, and occasional 200+ column cases.
pub fn gen_lines(n: usize) -> String {
    let mut state = 0x9e37_79b9_u32;
    let mut out = String::with_capacity(n.saturating_mul(58));
    for line in 0..n {
        if line != 0 {
            out.push('\n');
        }
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let width = match line % 20 {
            0 => 0,
            1 | 2 => 5 + (state as usize % 10),
            19 => 210 + (state as usize % 70),
            3..=12 => 34 + (state as usize % 14),
            _ => 88 + (state as usize % 28),
        };
        if width == 0 {
            continue;
        }
        let prefix = if line % 37 == 0 { "fn λ" } else { "let v" };
        out.push_str(prefix);
        out.push_str(&(state % 10_000).to_string());
        out.push_str(" = ");
        while out.rsplit_once('\n').map_or(out.len(), |(_, tail)| tail.len()) < width {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            out.push((b'a' + (state % 26) as u8) as char);
        }
    }
    out
}

/// Builds and settles one scrollable multiline editor field.
pub fn editor_field_doc(lines: usize) -> Instance {
    editor_field_doc_inner(lines, true)
}

/// Builds an editor whose large text assignment has not yet been laid out.
///
/// A tiny bootstrap frame is required because the public focus API only accepts
/// nodes from the current painted scene. The returned dirty frame is therefore
/// the first layout of the generated editor text.
pub fn editor_field_doc_cold(lines: usize) -> Instance {
    editor_field_doc_inner(lines, false)
}

fn editor_field_doc_inner(lines: usize, warm: bool) -> Instance {
    let mut instance = compile_instance(EDITOR_DOC, "editor field");
    inst_set_env(&mut instance, WIDTH, HEIGHT, CLIENT_GPU, false, false);
    assert!(
        inst_set_scroll(&mut instance, "editor-scroll", 0, 0.0),
        "editor scroll container initialization failed"
    );
    let _ = inst_frame(&mut instance, 0.0);

    let text = gen_lines(lines);
    assert!(
        inst_set_field_text(&mut instance, EDITOR_FIELD, &text),
        "editor field text assignment failed"
    );
    assert!(
        inst_set_focus(&mut instance, EDITOR_FIELD, false),
        "editor field focus failed: {}",
        inst_focus_note(&instance)
    );
    let middle = i32::try_from(text.chars().count() / 2).unwrap_or(i32::MAX);
    assert!(
        inst_set_caret(&mut instance, EDITOR_FIELD, middle, middle),
        "editor field caret assignment failed"
    );
    if warm {
        let _ = inst_frame(&mut instance, 0.0);
    }
    instance
}

/// Builds and settles a scrollable slate-like list of editable blocks.
pub fn blocks_doc(blocks: usize) -> Instance {
    let mut instance = compile_instance(BLOCKS_DOC, "blocks");
    inst_set_env(&mut instance, WIDTH, HEIGHT, CLIENT_GPU, false, false);
    assert!(blocks > 0, "blocks benchmark requires at least one block");
    let count = i32::try_from(blocks).expect("block count exceeds kernel list capacity");
    assert!(
        inst_set_list_len(&mut instance, BLOCKS_PARAM, "", count),
        "blocks list length assignment failed"
    );

    for index in 0..blocks {
        let key = block_key(index);
        let index_i32 = i32::try_from(index).expect("block index exceeds kernel list capacity");
        assert!(
            inst_set_list_key(&mut instance, BLOCKS_PARAM, "", index_i32, &key),
            "stable key assignment failed for block {index}"
        );
        let text = format!("block {index}: deterministic editable text");
        assert!(
            inst_set_list_field(
                &mut instance,
                BLOCKS_PARAM,
                "",
                index_i32,
                "text",
                &ParamValue { kind: 0, num: 0.0, s: text.clone(), rgba: 0, sym: String::new() },
            ),
            "text property assignment failed for block {index}"
        );
    }
    let _ = inst_frame(&mut instance, 0.0);
    for index in 0..blocks {
        let key = block_key(index);
        let text = format!("block {index}: deterministic editable text");
        let locator = block_locator(&key);
        let resolution = slab_kernel::scene::resolve_key(instance.doc(), &instance.st.lists, &locator);
        assert!(
            inst_set_field_text(&mut instance, &locator, &text),
            "editable field assignment failed for block {index}: locator {locator}, resolution {resolution:?}"
        );
    }

    let middle = blocks / 2;
    assert!(
        inst_set_scroll(&mut instance, "blocks-scroll", 0, middle as f64 * 28.0),
        "blocks scroll offset assignment failed"
    );
    let _ = inst_frame(&mut instance, 0.0);

    let locator = block_locator(&block_key(middle));
    assert!(
        inst_set_focus(&mut instance, &locator, false),
        "middle block focus failed: {}",
        inst_focus_note(&instance)
    );
    assert!(
        inst_set_caret(&mut instance, &locator, 8, 8),
        "middle block caret assignment failed"
    );
    let _ = inst_frame(&mut instance, 0.0);
    instance
}

/// Dispatches committed text through the same event path as a native host.
pub fn type_char(instance: &mut Instance, ch: &str) {
    let mut event = event(E_TEXT);
    event.text = ch.to_owned();
    let _ = inst_dispatch(instance, &event);
}

/// Dispatches one named key-down event through the host input path.
pub fn key_down(instance: &mut Instance, key: &str) {
    let mut event = event(E_KEY_DOWN);
    event.key = key.to_owned();
    let _ = inst_dispatch(instance, &event);
}

/// Dispatches a native-style wheel event at the center of the scroll viewport.
pub fn scroll(instance: &mut Instance, delta_y: f64) {
    let mut event = event(E_WHEEL);
    event.x = WIDTH / 2.0;
    event.y = HEIGHT / 2.0;
    event.dy = delta_y;
    let _ = inst_dispatch(instance, &event);
}

fn compile_instance(source: &str, scenario: &str) -> Instance {
    let (document, diagnostics) = slab_compile::compile(source, &slab_compile::Options::default());
    if diagnostics.has_errors() {
        let messages = diagnostics
            .0
            .iter()
            .map(|diagnostic| diagnostic.format("<benchmark>"))
            .collect::<Vec<_>>()
            .join("\n");
        panic!("{scenario} benchmark document failed to compile:\n{messages}");
    }
    let document = document.unwrap_or_else(|| panic!("{scenario} benchmark produced no SLIR"));
    let bytes = slab_slir::write(&document);
    slab_slir::instance(&bytes)
        .unwrap_or_else(|error| panic!("{scenario} benchmark SLIR failed to instantiate: {error}"))
        .0
}

fn block_key(index: usize) -> String {
    format!("block-{index}")
}
fn block_locator(key: &str) -> String {
    format!(
        "#blocks-scroll/#blocks-list~{}/key/#field",
        slab_kernel::scene::escape_segment(key)
    )
}

const fn event(etype: u32) -> Event {
    Event {
        etype,
        x: 0.0,
        y: 0.0,
        dx: 0.0,
        dy: 0.0,
        button: 0,
        clicks: 0,
        key: String::new(),
        text: String::new(),
        clauses: Vec::new(),
        mods: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_builders_settle() {
        let _ = editor_field_doc(8);
        let _ = blocks_doc(3);
    }
}
