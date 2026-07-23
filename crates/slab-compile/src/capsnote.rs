//! Renderer capability notes for `slab render`.
//!
//! Feature detection lives in [`slab_kernel::capability`] so native and
//! WebAssembly conformance, plus render-time notes, share one scan over the
//! decoded document and solved frame.

use slab_kernel::{capability, caps, flatten::Frame, slir::Doc};

/// One-time capability notes for `slab render`: unsupported features plus
/// degraded text keyframes. `skip` holds codes a driver already surfaced, so
/// nothing reports twice. The CLI prints these strings; wasm returns them.
pub fn render_notes(doc: &Doc, fr: &Frame, client: u32, skip: &[String]) -> Vec<String> {
    let client_index = usize::try_from(client).expect("client index exceeds usize");
    let mut out = Vec::new();
    for (f, &name) in caps::FEATURES.iter().enumerate() {
        let code = caps::NOTES[f][client_index];
        if caps::LEVELS[f][client_index] == caps::NONE
            && capability::uses(doc, fr, name)
            && !skip.iter().any(|s| s == code)
        {
            out.push(format!(
                "note {code}: '{name}' is not supported by the {} renderer",
                caps::CLIENTS[client_index]
            ));
        } else if name == "text-keyframes"
            && caps::LEVELS[f][client_index] == caps::DEGRADED
            && capability::uses(doc, fr, name)
            && let Some(start) = code.find("(cap-")
            && let Some(end) = code[start + 1..].find(')')
        {
            let cap = &code[start + 1..start + 1 + end];
            if !skip.iter().any(|s| s == cap) {
                out.push(format!(
                    "note {cap}: 'text-keyframes' is degraded by the {} renderer",
                    caps::CLIENTS[client_index]
                ));
            }
        }
    }
    out
}
