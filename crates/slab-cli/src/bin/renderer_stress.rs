use std::{
	env,
	hint::black_box,
	process::ExitCode,
	time::{Duration, Instant},
};

use slab_kernel::{
	flatten::{Frame, frame_new},
	frame::{
		Instance, ParamValue, inst_frame_update, inst_set_env, inst_set_list_field,
		inst_set_list_key, inst_set_list_len, inst_set_param, inst_set_scroll,
	},
	slir::{PARAM_BOOL, PARAM_NUM, PARAM_TEXT},
};

const ROW_COUNT: i32 = 20_000;
const WARMUP_FRAMES: usize = 48;
const MEASURED_FRAMES: usize = 480;
const CLEAN_FRAMES: usize = 100_000;
const ROW_HEIGHT: f64 = 26.0;

const fn value(kind: u32, num: f64, text: String) -> ParamValue {
	ParamValue { kind, num, s: text, rgba: 0, sym: String::new() }
}

fn param(instance: &Instance, name: &str) -> Result<u32, String> {
	instance
		.doc()
		.parm_name
		.iter()
		.position(|&name_ref| {
			let index = usize::try_from(name_ref).expect("string index exceeds usize");
			instance.doc().strs[index] == name
		})
		.and_then(|index| u32::try_from(index).ok())
		.ok_or_else(|| format!("stress fixture has no '{name}' parameter"))
}

fn set_field(
	instance: &mut Instance,
	list: u32,
	index: i32,
	field: &str,
	field_value: &ParamValue,
) -> Result<(), String> {
	if inst_set_list_field(instance, list, "", index, field, field_value) {
		Ok(())
	} else {
		Err(format!("cannot set graph.rows[{index}].{field}"))
	}
}

fn populate_graph(instance: &mut Instance, rows: u32) -> Result<(), String> {
	if !inst_set_list_len(instance, rows, "", ROW_COUNT) {
		return Err("cannot size graph.rows".to_owned());
	}

	let lane = value(PARAM_TEXT, 0.0, "M 0 13 L 18 13 C 22 13 22 5 28 5 L 94 5".to_owned());
	let selected = value(PARAM_BOOL, 1.0, String::new());
	let node_x = value(PARAM_NUM, 42.0, String::new());

	for index in 0..ROW_COUNT {
		let key = format!("commit-{index:05}");
		if !inst_set_list_key(instance, rows, "", index, &key) {
			return Err(format!("cannot set graph row key {index}"));
		}
		set_field(
			instance,
			rows,
			index,
			"subject",
			&value(PARAM_TEXT, 0.0, format!("Retained renderer change {index:05}")),
		)?;
		set_field(
			instance,
			rows,
			index,
			"description",
			&value(PARAM_TEXT, 0.0, "Reuse layout state and cached paint records".to_owned()),
		)?;
		set_field(
			instance,
			rows,
			index,
			"date",
			&value(PARAM_TEXT, 0.0, "2026-07-28 12:00".to_owned()),
		)?;
		set_field(instance, rows, index, "sha", &value(PARAM_TEXT, 0.0, format!("{index:08x}")))?;
		set_field(instance, rows, index, "lane_c0", &lane)?;
		if index % 97 == 0 {
			set_field(instance, rows, index, "selected", &selected)?;
		}
		if index % 11 == 0 {
			set_field(instance, rows, index, "node_x", &node_x)?;
		}
	}
	Ok(())
}

fn update_frame(
	instance: &mut Instance,
	frame: &mut Frame,
	rows: u32,
	message_width: u32,
	step: usize,
) -> Result<(), String> {
	let row_count = usize::try_from(ROW_COUNT).expect("positive row count");
	let row = (step.wrapping_mul(1_943).wrapping_add(step / 7 * 31)) % (row_count - 64);
	let row = i32::try_from(row).expect("row index exceeds i32");
	let scroll = f64::from(row) * ROW_HEIGHT;
	if !inst_set_scroll(instance, "graph-scroll", 0, scroll) {
		return Err("cannot scroll #graph-scroll".to_owned());
	}

	let width = if step & 1 == 0 { 326.0 } else { 334.0 };
	if !inst_set_param(instance, message_width, &value(PARAM_NUM, width, String::new())) {
		return Err("cannot set message_width".to_owned());
	}

	let subject = if step & 1 == 0 {
		"Retained renderer update A"
	} else {
		"Retained renderer update B"
	};
	set_field(instance, rows, row, "subject", &value(PARAM_TEXT, 0.0, subject.to_owned()))?;

	if step.is_multiple_of(40) {
		let width = if step.is_multiple_of(80) {
			1_920.0
		} else {
			1_440.0
		};
		inst_set_env(instance, width, 1_080.0, 1, true, false);
	}

	if !inst_frame_update(instance, f64::from(u32::try_from(step).expect("step exceeds u32")), frame)
	{
		return Err("dirty stress frame was not rendered".to_owned());
	}
	black_box((frame.ops.len(), frame.strings.len(), instance.sc.entries.len()));
	Ok(())
}

fn nanos(duration: Duration) -> u64 {
	u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn trimmed_mean(samples: &[u64]) -> u64 {
	let trim = samples.len() / 10;
	let middle = &samples[trim..samples.len() - trim];
	let sum = middle
		.iter()
		.fold(0_u128, |total, &sample| total.saturating_add(u128::from(sample)));
	u64::try_from(sum / u128::try_from(middle.len()).expect("sample count exceeds u128"))
		.unwrap_or(u64::MAX)
}

fn benchmark(path: &str) -> Result<(), String> {
	let bytes = std::fs::read(path).map_err(|error| format!("cannot read {path}: {error}"))?;
	let (mut instance, _) = slab_slir::instance(&bytes)?;
	inst_set_env(&mut instance, 1_920.0, 1_080.0, 1, true, false);

	let rows = param(&instance, "graph.rows")?;
	let message_width = param(&instance, "message_width")?;
	populate_graph(&mut instance, rows)?;

	let mut frame = frame_new();
	for step in 0..WARMUP_FRAMES {
		update_frame(&mut instance, &mut frame, rows, message_width, step)?;
	}

	let mut samples = Vec::with_capacity(MEASURED_FRAMES);
	for step in WARMUP_FRAMES..WARMUP_FRAMES + MEASURED_FRAMES {
		let start = Instant::now();
		update_frame(&mut instance, &mut frame, rows, message_width, step)?;
		samples.push(nanos(start.elapsed()));
	}
	samples.sort_unstable();

	let clean_start = Instant::now();
	for step in 0..CLEAN_FRAMES {
		black_box(inst_frame_update(
			black_box(&mut instance),
			black_box(f64::from(u32::try_from(step).expect("step exceeds u32"))),
			black_box(&mut frame),
		));
	}
	let clean_frame_ns = nanos(clean_start.elapsed())
		/ u64::try_from(CLEAN_FRAMES).expect("clean frame count exceeds u64");

	let frame_time_ns = trimmed_mean(&samples);
	let p50 = samples[samples.len() / 2];
	let p95 = samples[samples.len() * 95 / 100];
	let frame_time_for_fps = u32::try_from(frame_time_ns).unwrap_or(u32::MAX);
	let fps = 1_000_000_000.0 / f64::from(frame_time_for_fps);

	println!("METRIC frame_time_ns={frame_time_ns}");
	println!("METRIC frame_p50_ns={p50}");
	println!("METRIC frame_p95_ns={p95}");
	println!("METRIC frames_per_second={fps:.3}");
	println!("METRIC clean_frame_ns={clean_frame_ns}");
	println!("METRIC frame_ops={}", frame.ops.len());
	println!("METRIC scene_nodes={}", instance.sc.entries.len());
	Ok(())
}

fn main() -> ExitCode {
	let Some(path) = env::args().nth(1) else {
		eprintln!("usage: renderer_stress <document.slir>");
		return ExitCode::FAILURE;
	};
	match benchmark(&path) {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("renderer stress failed: {error}");
			ExitCode::FAILURE
		},
	}
}
