//! Canonical native-kernel output used by the Node WASM conformance runner.

use serde::Serialize;
use slab_kernel::{
    capability, caps, cells,
    frame::{self, Instance},
};

pub(crate) fn cells_text(instance: &mut Instance, time_ms: f64) -> String {
    let solved = frame::inst_frame(instance, time_ms);
    let grid = cells::cells_from_frame(&instance.doc, &solved, solved.width, solved.height);
    cells::cells_to_text(&grid, true)
}

pub(crate) fn cells_attrs(instance: &mut Instance, time_ms: f64) -> String {
    let solved = frame::inst_frame(instance, time_ms);
    let grid = cells::cells_from_frame(&instance.doc, &solved, solved.width, solved.height);
    cells::cells_attrs_text(&grid)
}

pub(crate) fn caps_report(
    instance: &mut Instance,
    time_ms: f64,
    client: u32,
) -> Result<String, String> {
    let client_index =
        usize::try_from(client).map_err(|_| format!("invalid client index {client}"))?;
    if client_index >= caps::CLIENTS.len() {
        return Err(format!("invalid client index {client}"));
    }

    let solved = frame::inst_frame(instance, time_ms);
    let mut lines = Vec::new();
    if client == 2 {
        let grid = cells::cells_from_frame(&instance.doc, &solved, solved.width, solved.height);
        for (index, code) in grid.diag_code.iter().enumerate() {
            lines.push(format!("grid {code}: {}", grid.diag_msg[index]));
        }
    }
    lines.extend(capability::chart_lines(
        &instance.doc,
        &solved,
        client_index,
    ));
    let mut report = lines.join("\n");
    report.push('\n');
    Ok(report)
}

#[derive(Serialize)]
struct SelftestCounts {
    nodes: usize,
    strs: usize,
    values: usize,
    fonts: usize,
    ops: usize,
}

pub(crate) fn selftest_counts_json(instance: &mut Instance, time_ms: f64) -> String {
    let solved = frame::inst_frame(instance, time_ms);
    serde_json::to_string(&SelftestCounts {
        nodes: instance.doc.node_kind.len(),
        strs: instance.doc.strs.len(),
        values: instance.doc.aval_tag.len(),
        fonts: instance.doc.font_family.len(),
        ops: solved.ops.len(),
    })
    .expect("selftest counts are serializable")
}
