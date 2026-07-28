//! Hand-rolled kernel frame benchmark: `cargo bench -p slab-cli`.
//!
//! Compiles the checked-in examples, then times the hot kernel paths a real
//! host exercises per frame: dirty solves, animated re-solves, clean retained
//! updates, and pointer dispatch. Prints one line per (example, phase) with
//! median and mean nanoseconds so before/after runs diff cleanly.

use std::{hint::black_box, path::Path, time::Instant};

use slab_kernel::{
	dispatch::{E_POINTER_MOVE, Event},
	frame::{self, Instance},
};

fn compile(path: &Path) -> Vec<u8> {
	let src = std::fs::read_to_string(path).expect("read example");
	let opts = slab_compile::Options {
		embed_assets: true,
		base_dir:     path
			.parent()
			.map_or_else(|| Path::new(".").to_path_buf(), Path::to_path_buf),
		assets:       None,
		sources:      None,
		fonts:        std::collections::HashMap::new(),
	};
	let (slir, diags) = slab_compile::compile(&src, &opts);
	assert!(!diags.has_errors(), "{}: compile errors", path.display());
	slab_slir::write(&slir.expect("slir"))
}

fn instance(bytes: &[u8]) -> Instance {
	let (mut inst, _) = slab_slir::instance(bytes).expect("decode");
	frame::inst_set_env(&mut inst, 1280.0, 800.0, 0, false, false);
	inst
}

/// Times `iters` runs of `f`, returning (median, mean) nanoseconds per run.
fn time<F: FnMut(u64)>(iters: u64, mut f: F) -> (u64, u64) {
	let mut samples = Vec::with_capacity(iters as usize);
	for i in 0..iters {
		let start = Instant::now();
		f(i);
		samples.push(start.elapsed().as_nanos() as u64);
	}
	samples.sort_unstable();
	let median = samples[samples.len() / 2];
	let mean = samples.iter().sum::<u64>() / samples.len().max(1) as u64;
	(median, mean)
}

fn report(example: &str, phase: &str, (median, mean): (u64, u64)) {
	println!("{example:<24} {phase:<16} median {median:>10} ns   mean {mean:>10} ns");
}

fn main() {
	let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
	let examples = [
		"examples/00-player.slab",
		"examples/02-ops.slab",
		"examples/07-monitor.slab",
		"examples/10-settings.slab",
		"examples/12-tracklist.slab",
		"examples/13-modern.slab",
	];

	for rel in examples {
		let name = Path::new(rel).file_stem().unwrap().to_str().unwrap();
		let bytes = compile(&root.join(rel));

		// Cold: decode + first solve.
		let cold = time(30, |_| {
			let mut inst = instance(&bytes);
			black_box(frame::inst_frame(&mut inst, 0.0));
		});
		report(name, "cold", cold);

		// Dirty re-solve: full solve + flatten each call (env stays, param dirt).
		let mut inst = instance(&bytes);
		frame::inst_frame(&mut inst, 0.0);
		let dirty = time(200, |i| {
			inst.dirty = true;
			black_box(frame::inst_frame(&mut inst, i as f64));
		});
		report(name, "dirty-solve", dirty);

		// Animated: advancing clock, reusing a retained frame buffer.
		let mut inst = instance(&bytes);
		let mut fr = frame::inst_frame(&mut inst, 0.0);
		let animated = time(200, |i| {
			inst.dirty = true;
			black_box(frame::inst_frame_update(&mut inst, i as f64, &mut fr));
		});
		report(name, "retained-solve", animated);

		// Clean retained update: nothing changed, should be near-zero.
		let mut inst = instance(&bytes);
		let mut fr = frame::inst_frame(&mut inst, 0.0);
		frame::inst_frame_update(&mut inst, 1.0, &mut fr);
		let t_settled = inst.last_t;
		let clean = time(200, |_| {
			black_box(frame::inst_frame_update(&mut inst, t_settled, &mut fr));
		});
		report(name, "clean-update", clean);

		// Pointer dispatch: sweep moves across the document.
		let mut inst = instance(&bytes);
		let mut fr = frame::inst_frame(&mut inst, 0.0);
		let dispatch = time(200, |i| {
			let x = ((i % 60) as f64).mul_add(20.0, 20.0);
			let y = ((i % 37) as f64).mul_add(20.0, 20.0);
			let ev = Event {
				etype: E_POINTER_MOVE,
				x,
				y,
				dx: 0.0,
				dy: 0.0,
				button: 0,
				clicks: 0,
				key: String::new(),
				text: String::new(),
				clauses: Vec::new(),
				mods: 0,
			};
			black_box(frame::inst_dispatch(&mut inst, &ev));
			black_box(frame::inst_frame_update(&mut inst, 1000.0 + i as f64, &mut fr));
		});
		report(name, "dispatch+frame", dispatch);
	}
}
