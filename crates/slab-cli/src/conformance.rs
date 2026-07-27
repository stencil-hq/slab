//! `slab conformance [--update] [--emit-slir DIR]` compiles every case in
//! `conformance/cases/`, drives `slab-kernel` natively, emits canonical
//! `dumpjson` frame snapshots, and byte-compares them with
//! `conformance/expected/<name>.frame.json`. Each compiled `.slir` is also
//! written to the emit directory (default `target/conformance-slir/`) so the
//! WebAssembly conformance runner can drive the same Rust kernel over the same
//! bytes and goldens.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub(crate) fn client_code(name: &str) -> Option<u32> {
    match name {
        "web" => Some(0),
        "gpu" => Some(1),
        "tui" => Some(2),
        "svg" => Some(3),
        "png" => Some(4),
        _ => None,
    }
}

/// Locate the repo root: the nearest ancestor holding conformance/manifest.json.
fn repo_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("conformance/manifest.json").is_file() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

pub(crate) fn compile_case(path: &Path) -> Result<Vec<u8>, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let opts = slab_compile::Options {
        embed_assets: true,
        base_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
        assets: None,
        fonts: std::collections::HashMap::new(),
    };
    let (slir, diags) = slab_compile::compile(&src, &opts);
    if diags.has_errors() {
        let msgs: Vec<String> = diags
            .0
            .iter()
            .map(|d| d.format(&path.display().to_string()))
            .collect();
        return Err(msgs.join("\n"));
    }
    let slir = slir.ok_or_else(|| format!("{}: no SLIR produced", path.display()))?;
    Ok(slab_slir::write(&slir))
}

/// Build the instance for a manifest case and return it plus the final
/// frame clock. Cases with `states_prev`/`state_age` drive the P5
/// transition sequence: solve once under states_prev at t=0, flip to
/// `states` (the kernel stamps the flip at the next solve's t=0), and
/// sample the tween at t=state_age — the research build(states,
/// states_prev, state_age) contract realized on kernel-tracked clocks.
fn setup_case(
    bytes: &[u8],
    case: &serde_json::Value,
) -> Result<(slab_kernel::frame::Instance, f64), String> {
    let (mut inst, _) = slab_slir::instance(bytes)?;
    let width = case["width"].as_f64().unwrap_or(800.0);
    let height = case["height"].as_f64().unwrap_or(0.0); // 0 = unbounded
    let client = case["client"].as_str().unwrap_or("svg");
    let client = client_code(client).ok_or_else(|| format!("unknown client '{client}'"))?;
    slab_kernel::frame::inst_set_env(&mut inst, width, height, client, false, false);
    let theme = match case.get("theme") {
        None | Some(serde_json::Value::Null) => "",
        Some(value) => value
            .as_str()
            .ok_or_else(|| "manifest field 'theme' must be a string".to_string())?,
    };
    if !slab_kernel::frame::inst_set_theme(&mut inst, theme) {
        return Err(format!("unknown theme '{theme}'"));
    }
    let sets =
        slab_compile::input::sets_from_json(case.get("set").unwrap_or(&serde_json::Value::Null))
            .map_err(|e| format!("manifest field 'set': {e}"))?;
    slab_compile::input::apply_sets(&mut inst, &sets)?;
    let set_states = |inst: &mut slab_kernel::frame::Instance, list: &serde_json::Value| {
        if let Some(states) = list.as_array() {
            for s in states {
                if let Some(name) = s.as_str() {
                    slab_kernel::frame::inst_set_state(inst, name, true);
                }
            }
        }
    };
    if let Some(age) = case["state_age"].as_f64() {
        set_states(&mut inst, &case["states_prev"]);
        slab_kernel::frame::inst_frame(&mut inst, 0.0);
        // drop prev-only states, then apply the current set
        if let Some(prev) = case["states_prev"].as_array() {
            let cur = case["states"].as_array().cloned().unwrap_or_default();
            for s in prev {
                if let Some(name) = s.as_str()
                    && !cur.iter().any(|c| c.as_str() == Some(name))
                {
                    slab_kernel::frame::inst_set_state(&mut inst, name, false);
                }
            }
        }
        set_states(&mut inst, &case["states"]);
        slab_kernel::frame::inst_frame(&mut inst, 0.0); // flip stamped at t=0
        return Ok((inst, age));
    }
    set_states(&mut inst, &case["states"]);
    let t = case["t"].as_f64().unwrap_or(0.0);
    Ok((inst, t))
}

/// Runs one case through the native Rust kernel and returns one JSON frame.
fn run_case(bytes: &[u8], case: &serde_json::Value) -> Result<String, String> {
    let (mut inst, t) = setup_case(bytes, case)?;
    let fr = slab_kernel::frame::inst_frame(&mut inst, t);
    Ok(slab_kernel::dumpjson::dump(&inst.doc, &inst.st, &fr))
}

/// Runs one TUI case through `cells` and returns the plain cell grid used by
/// the conformance golden format (no ANSI).
fn run_cells(bytes: &[u8], case: &serde_json::Value) -> Result<String, String> {
    let (mut inst, t) = setup_case(bytes, case)?;
    let fr = slab_kernel::frame::inst_frame(&mut inst, t);
    let grid = slab_kernel::cells::cells_from_frame(&inst.doc, &fr, fr.width, fr.height);
    Ok(slab_kernel::cells::cells_to_text(&grid, true))
}
/// Runs one capability case: TUI grid diagnostics first, then the shared
/// support-chart lines for each used feature the selected client degrades or
/// omits. Native and WebAssembly runners both use `slab_kernel::capability`.
fn run_caps(bytes: &[u8], case: &serde_json::Value) -> Result<String, String> {
    let (mut inst, t) = setup_case(bytes, case)?;
    let fr = slab_kernel::frame::inst_frame(&mut inst, t);
    let client = case["client"].as_str().unwrap_or("svg");
    let client = client_code(client).ok_or_else(|| format!("unknown client '{client}'"))?;
    let mut lines = Vec::new();
    if client == 2 {
        let grid = slab_kernel::cells::cells_from_frame(&inst.doc, &fr, fr.width, fr.height);
        for k in 0..grid.diag_code.len() {
            lines.push(format!("grid {}: {}", grid.diag_code[k], grid.diag_msg[k]));
        }
    }
    let client_index = usize::try_from(client).expect("client index exceeds usize");
    lines.extend(slab_kernel::capability::chart_lines(
        &inst.doc,
        &fr,
        client_index,
    ));
    let mut out = lines.join("\n");
    out.push('\n');
    Ok(out)
}

/// First-difference window for two single-line JSON payloads.
pub(crate) fn diff_window(got: &str, want: &str) -> String {
    let gb = got.as_bytes();
    let wb = want.as_bytes();
    let n = gb.len().min(wb.len());
    let mut at = n;
    for i in 0..n {
        if gb[i] != wb[i] {
            at = i;
            break;
        }
    }
    if at == n && gb.len() == wb.len() {
        return "identical?".into();
    }
    let lo = at.saturating_sub(60);
    let g_hi = (at + 60).min(gb.len());
    let w_hi = (at + 60).min(wb.len());
    format!(
        "first diff at byte {at}\n  expected: …{}…\n  got:      …{}…",
        String::from_utf8_lossy(&wb[lo..w_hi]),
        String::from_utf8_lossy(&gb[lo..g_hi]),
    )
}

pub fn cmd_conformance(args: &[String]) -> ExitCode {
    let mut update = false;
    let mut emit_dir: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--update" => update = true,
            "--emit-slir" => match it.next() {
                Some(v) => emit_dir = Some(PathBuf::from(v)),
                None => {
                    eprintln!("error: missing value for --emit-slir");
                    return ExitCode::from(2);
                }
            },
            other => {
                eprintln!("error: unknown argument '{other}'");
                return ExitCode::from(2);
            }
        }
    }
    let Some(root) = repo_root() else {
        eprintln!("error: conformance/manifest.json not found from the current directory");
        return ExitCode::from(2);
    };
    let emit_dir = emit_dir.unwrap_or_else(|| root.join("target/conformance-slir"));
    if let Err(e) = std::fs::create_dir_all(&emit_dir) {
        eprintln!("error: {}: {e}", emit_dir.display());
        return ExitCode::from(2);
    }
    let manifest: serde_json::Value =
        match std::fs::read_to_string(root.join("conformance/manifest.json"))
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        {
            Ok(v) => v,
            Err(e) => {
                eprintln!("error: conformance/manifest.json: {e}");
                return ExitCode::from(2);
            }
        };
    let cases = manifest["cases"].as_array().cloned().unwrap_or_default();
    let mut pass = 0usize;
    let mut fail = 0usize;
    for case in &cases {
        let name = case["name"].as_str().unwrap_or("?");
        let source = case["source"].as_str().unwrap_or(name);
        let src = root
            .join("conformance/cases")
            .join(format!("{source}.slab"));
        let bytes = match compile_case(&src) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("FAIL {name}: compile\n{e}");
                fail += 1;
                continue;
            }
        };
        if let Err(e) = std::fs::write(emit_dir.join(format!("{name}.slir")), &bytes) {
            eprintln!("FAIL {name}: write slir: {e}");
            fail += 1;
            continue;
        }
        // `caps` cases freeze the degradation report instead of frame.json
        if case["kind"].as_str() == Some("caps") {
            let caps_path = root
                .join("conformance/expected")
                .join(format!("{name}.caps.txt"));
            let ok = match run_caps(&bytes, case) {
                Ok(text) => {
                    if update {
                        match std::fs::write(&caps_path, &text) {
                            Ok(()) => {
                                eprintln!("update {name}: wrote {} bytes (caps)", text.len());
                                true
                            }
                            Err(e) => {
                                eprintln!("FAIL {name}: write caps golden: {e}");
                                false
                            }
                        }
                    } else {
                        match std::fs::read_to_string(&caps_path) {
                            Ok(want) if want == text => {
                                eprintln!("ok {name}");
                                true
                            }
                            Ok(want) => {
                                eprintln!("FAIL {name}: caps.txt mismatch");
                                eprintln!("{}", diff_window(&text, &want));
                                false
                            }
                            Err(e) => {
                                eprintln!(
                                    "FAIL {name}: {}: {e} (run with --update)",
                                    caps_path.display()
                                );
                                false
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("FAIL {name}: caps: {e}");
                    false
                }
            };
            if ok {
                pass += 1;
            } else {
                fail += 1;
            }
            continue;
        }
        let json = match run_case(&bytes, case) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("FAIL {name}: {e}");
                fail += 1;
                continue;
            }
        };
        let payload = format!("{json}\n");
        let expected_path = root
            .join("conformance/expected")
            .join(format!("{name}.frame.json"));
        let mut case_ok = true;
        if update {
            if let Err(e) = std::fs::write(&expected_path, &payload) {
                eprintln!("FAIL {name}: write golden: {e}");
                case_ok = false;
            } else {
                eprintln!("update {name}: wrote {} bytes", payload.len());
            }
        } else {
            match std::fs::read_to_string(&expected_path) {
                Ok(want) if want == payload => {}
                Ok(want) => {
                    eprintln!("FAIL {name}: frame.json mismatch");
                    eprintln!("{}", diff_window(&payload, &want));
                    case_ok = false;
                }
                Err(e) => {
                    eprintln!(
                        "FAIL {name}: {}: {e} (run with --update)",
                        expected_path.display()
                    );
                    case_ok = false;
                }
            }
        }
        // TUI cases additionally freeze the plain cell grid.
        if case["kind"].as_str() == Some("tui") {
            let cells_path = root
                .join("conformance/expected")
                .join(format!("{name}.cells.txt"));
            match run_cells(&bytes, case) {
                Ok(text) => {
                    if update {
                        if let Err(e) = std::fs::write(&cells_path, &text) {
                            eprintln!("FAIL {name}: write cells golden: {e}");
                            case_ok = false;
                        } else {
                            eprintln!("update {name}: wrote {} bytes (cells)", text.len());
                        }
                    } else {
                        match std::fs::read_to_string(&cells_path) {
                            Ok(want) if want == text => {}
                            Ok(_) => {
                                eprintln!("FAIL {name}: cells.txt mismatch");
                                case_ok = false;
                            }
                            Err(e) => {
                                eprintln!(
                                    "FAIL {name}: {}: {e} (run with --update)",
                                    cells_path.display()
                                );
                                case_ok = false;
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("FAIL {name}: cells: {e}");
                    case_ok = false;
                }
            }
        }
        if case_ok {
            if !update {
                eprintln!("ok {name}");
            }
            pass += 1;
        } else {
            fail += 1;
        }
    }
    // interaction traces (P5): conformance/cases/traces/*.json
    let (tpass, tfail) = crate::traces::run_traces(&root, &emit_dir, update);
    pass += tpass;
    fail += tfail;
    eprintln!("conformance: {pass}/{} ok", cases.len() + tpass + tfail);
    if fail > 0 {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
