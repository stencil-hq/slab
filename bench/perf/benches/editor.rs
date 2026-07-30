use std::time::{Duration, Instant};

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use slab_kernel::frame::{Instance, inst_frame};
use slab_perf::{
	blocks_doc, editor_field_doc, editor_field_doc_cold, key_down, rich_field_doc, scroll, type_char,
};

/// Criterion sizes. 100k-line scenarios run through the `oneshot` binary
/// (`cargo run -p slab-perf --release --bin oneshot`) instead: at tens of
/// seconds per operation they starve Criterion's sampling model.
const LINE_SIZES: [usize; 2] = [1_000, 10_000];
const BLOCK_SIZES: [usize; 2] = [1_000, 10_000];

/// Times `op` per iteration and runs `restore` outside the clock so the
/// instance returns to its pre-iteration state (stationary sampling).
fn iter_restored(
	b: &mut criterion::Bencher<'_>,
	instance: &mut Instance,
	time: &mut f64,
	op: fn(&mut Instance, &mut f64),
	restore: fn(&mut Instance, &mut f64),
) {
	b.iter_custom(|iters| {
		let mut total = Duration::ZERO;
		for _ in 0..iters {
			let start = Instant::now();
			op(instance, time);
			total += start.elapsed();
			restore(instance, time);
		}
		total
	});
}

fn insert_x(instance: &mut Instance, time: &mut f64) {
	type_char(instance, "x");
	*time += 1.0;
	inst_frame(instance, *time);
}

fn delete_back(instance: &mut Instance, time: &mut f64) {
	key_down(instance, "Backspace");
	*time += 1.0;
	inst_frame(instance, *time);
}

fn arrow_down(instance: &mut Instance, time: &mut f64) {
	key_down(instance, "ArrowDown");
	*time += 1.0;
	inst_frame(instance, *time);
}

fn arrow_up(instance: &mut Instance, time: &mut f64) {
	key_down(instance, "ArrowUp");
	*time += 1.0;
	inst_frame(instance, *time);
}

fn cold_layout(c: &mut Criterion) {
	let mut group = c.benchmark_group("cold_layout");
	for lines in LINE_SIZES {
		group.bench_with_input(BenchmarkId::from_parameter(lines), &lines, |b, &lines| {
			b.iter_batched(
				|| editor_field_doc_cold(lines),
				|mut instance| inst_frame(&mut instance, 1.0),
				BatchSize::LargeInput,
			);
		});
	}
	group.finish();
}

/// One insert keystroke + frame; buffer restored by an untimed backspace.
fn keystroke(c: &mut Criterion) {
	let mut group = c.benchmark_group("keystroke");
	for lines in LINE_SIZES {
		let mut instance = editor_field_doc(lines);
		let mut time = 1.0;
		group.bench_with_input(BenchmarkId::from_parameter(lines), &lines, |b, _| {
			iter_restored(b, &mut instance, &mut time, insert_x, delete_back);
		});
	}
	group.finish();
}

/// One insert keystroke + frame in a rich field, restored by an untimed
/// backspace.
fn rich_keystroke(c: &mut Criterion) {
	let mut group = c.benchmark_group("rich_keystroke");
	for lines in LINE_SIZES {
		let mut instance = rich_field_doc(lines);
		let mut time = 1.0;
		group.bench_with_input(BenchmarkId::from_parameter(lines), &lines, |b, _| {
			iter_restored(b, &mut instance, &mut time, insert_x, delete_back);
		});
	}
	group.finish();
}

/// `ArrowDown` + frame then insert + frame, timed together: caret motion must
/// not break splice lineage, so the insert still re-measures only its hard
/// line. Restored by an untimed backspace + `ArrowUp`.
fn nav_then_type(c: &mut Criterion) {
	fn op(instance: &mut Instance, time: &mut f64) {
		key_down(instance, "ArrowDown");
		*time += 1.0;
		inst_frame(instance, *time);
		type_char(instance, "x");
		*time += 1.0;
		inst_frame(instance, *time);
	}
	fn restore(instance: &mut Instance, time: &mut f64) {
		key_down(instance, "Backspace");
		*time += 1.0;
		inst_frame(instance, *time);
		key_down(instance, "ArrowUp");
		*time += 1.0;
		inst_frame(instance, *time);
	}
	let mut group = c.benchmark_group("nav_then_type");
	for lines in LINE_SIZES {
		let mut instance = editor_field_doc(lines);
		let mut time = 1.0;
		group.bench_with_input(BenchmarkId::from_parameter(lines), &lines, |b, _| {
			iter_restored(b, &mut instance, &mut time, op, restore);
		});
	}
	group.finish();
}

/// One backspace + frame; buffer restored by an untimed insert.
fn backspace(c: &mut Criterion) {
	let mut group = c.benchmark_group("backspace");
	for lines in LINE_SIZES {
		let mut instance = editor_field_doc(lines);
		let mut time = 1.0;
		group.bench_with_input(BenchmarkId::from_parameter(lines), &lines, |b, _| {
			iter_restored(b, &mut instance, &mut time, delete_back, insert_x);
		});
	}
	group.finish();
}

fn scroll_benchmark(c: &mut Criterion) {
	let mut group = c.benchmark_group("scroll");
	for lines in LINE_SIZES {
		let mut instance = editor_field_doc(lines);
		let mut time = 1.0;
		let mut delta = 120.0;
		group.bench_with_input(BenchmarkId::from_parameter(lines), &lines, |b, _| {
			b.iter(|| {
				scroll(&mut instance, delta);
				delta = -delta;
				time += 1.0;
				inst_frame(&mut instance, time)
			});
		});
	}
	group.finish();
}

/// `ArrowDown` + frame; caret restored by an untimed `ArrowUp`.
fn caret_nav(c: &mut Criterion) {
	let mut group = c.benchmark_group("caret_nav");
	for lines in LINE_SIZES {
		let mut instance = editor_field_doc(lines);
		let mut time = 1.0;
		group.bench_with_input(BenchmarkId::from_parameter(lines), &lines, |b, _| {
			iter_restored(b, &mut instance, &mut time, arrow_down, arrow_up);
		});
	}
	group.finish();
}

/// Insert + frame in one block of a slate-like block list, restored untimed.
fn blocks_keystroke(c: &mut Criterion) {
	let mut group = c.benchmark_group("blocks_keystroke");
	for blocks in BLOCK_SIZES {
		let mut instance = blocks_doc(blocks);
		let mut time = 1.0;
		group.bench_with_input(BenchmarkId::from_parameter(blocks), &blocks, |b, _| {
			iter_restored(b, &mut instance, &mut time, insert_x, delete_back);
		});
	}
	group.finish();
}

criterion_group! {
	 name = editor_benches;
	 config = Criterion::default()
		  .sample_size(10)
		  .warm_up_time(Duration::from_secs(1))
		  .measurement_time(Duration::from_secs(3));
	 targets = cold_layout, keystroke, rich_keystroke, nav_then_type, backspace, scroll_benchmark, caret_nav, blocks_keystroke
}
criterion_main!(editor_benches);
