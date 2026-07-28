//! Profiling loop: `cargo bench -p slab-cli --bench profile_loop` re-solves
//! one example for ~20s so an external sampler (`sample <pid>`) can attach.

use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rel = std::env::var("SLAB_PROFILE_DOC")
        .unwrap_or_else(|_| "examples/00-player.slab".to_owned());
    let path = root.join(rel);
    let src = std::fs::read_to_string(&path).expect("read example");
    let opts = slab_compile::Options {
        embed_assets: true,
        base_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
        assets: None,
        sources: None,
        fonts: std::collections::HashMap::new(),
    };
    let (slir, diags) = slab_compile::compile(&src, &opts);
    assert!(!diags.has_errors(), "compile errors");
    let bytes = slab_slir::write(&slir.expect("slir"));
    let (mut inst, _) = slab_slir::instance(&bytes).expect("decode");
    slab_kernel::frame::inst_set_env(&mut inst, 1280.0, 800.0, 0, false, false);
    let mut fr = slab_kernel::frame::inst_frame(&mut inst, 0.0);

    println!("pid {}", std::process::id());
    let start = Instant::now();
    let mut n = 0u64;
    while start.elapsed().as_secs_f64() < 20.0 {
        inst.dirty = true;
        black_box(slab_kernel::frame::inst_frame_update(
            &mut inst,
            n as f64,
            &mut fr,
        ));
        n += 1;
    }
    println!("{n} solves in 20s: {} ns/solve", 20_000_000_000 / n.max(1));
}
