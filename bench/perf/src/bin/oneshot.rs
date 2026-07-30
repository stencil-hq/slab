//! One-shot large-buffer probe for scenarios too slow for Criterion sampling.
//!
//! Usage: `cargo run -p slab-perf --release --bin oneshot [-- LINES ...
//! [cold|warm|rich|sweep]]` Defaults to 100000 lines. `cold` and `warm`
//! restrict the ordinary probe phases; `rich` times a rich-field keystroke;
//! `sweep` scrolls page-by-page through the whole document. Sizes below 50k
//! lines take 5 samples per ordinary operation (mutations restored outside the
//! clock); larger sizes take exactly one sample per operation.

use std::time::{Duration, Instant};

use slab_kernel::frame::{Instance, inst_frame, inst_get_scroll, inst_set_caret, inst_set_scroll};
use slab_perf::{
	editor_field_doc, editor_field_doc_cold, key_down, rich_field_doc, scroll, type_char,
};

fn timed(mut op: impl FnMut()) -> Duration {
	let start = Instant::now();
	op();
	start.elapsed()
}

fn stats(label: &str, lines: usize, mut samples: Vec<Duration>) {
	samples.sort_unstable();
	let median = samples[samples.len() / 2];
	println!(
		"{label:<12} {lines:>7} lines  n={:<2} min {:>12.3?}  median {:>12.3?}  max {:>12.3?}",
		samples.len(),
		samples[0],
		median,
		samples[samples.len() - 1],
	);
}

fn rich_probe(lines: usize) {
	let mut instance = rich_field_doc(lines);
	let elapsed = timed(|| {
		type_char(&mut instance, "x");
		inst_frame(&mut instance, 1.0);
	});
	stats("rich", lines, vec![elapsed]);
}

fn sweep_probe(lines: usize) {
	let mut instance = editor_field_doc(lines);
	assert!(inst_set_caret(&mut instance, "editor", 0, 0), "failed to reset editor caret");
	assert!(
		inst_set_scroll(&mut instance, "editor-scroll", 0, 0.0),
		"failed to reset editor scroll"
	);
	inst_frame(&mut instance, 1.0);
	let mut time = 1.0;
	let mut previous = inst_get_scroll(&instance, "editor-scroll", 0);
	assert_eq!(previous, 0.0, "sweep must begin at the document top");
	let mut steps = 0_usize;
	let start = Instant::now();
	loop {
		scroll(&mut instance, 700.0);
		time += 1.0;
		inst_frame(&mut instance, time);
		steps += 1;
		let current = inst_get_scroll(&instance, "editor-scroll", 0);
		if current <= previous {
			break;
		}
		previous = current;
	}
	println!("sweep       {lines:>7} lines  steps={steps:<7} total {:>12.3?}", start.elapsed());
}

fn probe(lines: usize, phase: Option<&str>) {
	if phase == Some("rich") {
		rich_probe(lines);
		return;
	}
	if phase == Some("sweep") {
		sweep_probe(lines);
		return;
	}
	let reps = if lines >= 50_000 { 1 } else { 5 };
	let frame = |instance: &mut Instance, time: &mut f64| {
		*time += 1.0;
		inst_frame(instance, *time);
	};

	if phase.is_none_or(|phase| phase == "cold") {
		// Cold layout: fresh instance per repetition, first frame timed.
		let cold: Vec<Duration> = (0..reps)
			.map(|_| {
				let mut instance = editor_field_doc_cold(lines);
				timed(|| {
					inst_frame(&mut instance, 1.0);
				})
			})
			.collect();
		stats("cold_layout", lines, cold);
	}
	if phase.is_some_and(|phase| phase == "cold") {
		return;
	}

	let mut instance = editor_field_doc(lines);
	let mut time = 1.0;

	let keystroke: Vec<Duration> = (0..reps)
		.map(|rep| {
			let sample = timed(|| {
				type_char(&mut instance, "x");
				frame(&mut instance, &mut time);
			});
			// Restore outside the clock, unless this was the last repetition;
			// one stray 'x' does not perturb the remaining scenarios.
			if rep + 1 < reps {
				key_down(&mut instance, "Backspace");
				frame(&mut instance, &mut time);
			}
			sample
		})
		.collect();
	stats("keystroke", lines, keystroke);

	let mut delta = 120.0;
	let scroll_samples: Vec<Duration> = (0..reps * 2)
		.map(|_| {
			let sample = timed(|| {
				scroll(&mut instance, delta);
				frame(&mut instance, &mut time);
			});
			delta = -delta;
			sample
		})
		.collect();
	stats("scroll", lines, scroll_samples);

	let caret: Vec<Duration> = (0..reps)
		.map(|rep| {
			let sample = timed(|| {
				key_down(&mut instance, "ArrowDown");
				frame(&mut instance, &mut time);
			});
			if rep + 1 < reps {
				key_down(&mut instance, "ArrowUp");
				frame(&mut instance, &mut time);
			}
			sample
		})
		.collect();
	stats("caret_nav", lines, caret);
}

fn main() {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let phase = args
		.iter()
		.find(|arg| matches!(arg.as_str(), "cold" | "warm" | "rich" | "sweep"))
		.map(String::as_str);
	let sizes: Vec<usize> = args
		.iter()
		.filter(|arg| !matches!(arg.as_str(), "cold" | "warm" | "rich" | "sweep"))
		.map(|arg| arg.parse().expect("line count must be an integer"))
		.collect();
	let sizes = if sizes.is_empty() {
		vec![100_000]
	} else {
		sizes
	};
	for lines in sizes {
		probe(lines, phase);
	}
}
