use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use slab_kernel::frame::inst_frame;
use slab_perf::{blocks_doc, editor_field_doc, editor_field_doc_cold, key_down, scroll, type_char};

const LINE_SIZES: [usize; 3] = [1_000, 10_000, 100_000];
const BLOCK_SIZES: [usize; 2] = [1_000, 10_000];

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

fn keystroke(c: &mut Criterion) {
    let mut group = c.benchmark_group("keystroke");
    for lines in LINE_SIZES {
        let mut instance = editor_field_doc(lines);
        let mut time = 1.0;
        group.bench_with_input(BenchmarkId::from_parameter(lines), &lines, |b, _| {
            b.iter(|| {
                type_char(&mut instance, "x");
                time += 1.0;
                inst_frame(&mut instance, time)
            });
        });
    }
    group.finish();
}

fn backspace(c: &mut Criterion) {
    let mut group = c.benchmark_group("backspace");
    for lines in LINE_SIZES {
        let mut instance = editor_field_doc(lines);
        let mut time = 1.0;
        group.bench_with_input(BenchmarkId::from_parameter(lines), &lines, |b, _| {
            b.iter(|| {
                key_down(&mut instance, "Backspace");
                time += 1.0;
                inst_frame(&mut instance, time)
            });
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

fn caret_nav(c: &mut Criterion) {
    let mut group = c.benchmark_group("caret_nav");
    for lines in LINE_SIZES {
        let mut instance = editor_field_doc(lines);
        let mut time = 1.0;
        group.bench_with_input(BenchmarkId::from_parameter(lines), &lines, |b, _| {
            b.iter(|| {
                key_down(&mut instance, "ArrowDown");
                time += 1.0;
                inst_frame(&mut instance, time)
            });
        });
    }
    group.finish();
}

fn blocks_keystroke(c: &mut Criterion) {
    let mut group = c.benchmark_group("blocks_keystroke");
    for blocks in BLOCK_SIZES {
        let mut instance = blocks_doc(blocks);
        let mut time = 1.0;
        group.bench_with_input(BenchmarkId::from_parameter(blocks), &blocks, |b, _| {
            b.iter(|| {
                type_char(&mut instance, "x");
                time += 1.0;
                inst_frame(&mut instance, time)
            });
        });
    }
    group.finish();
}

criterion_group! {
    name = editor_benches;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3));
    targets = cold_layout, keystroke, backspace, scroll_benchmark, caret_nav, blocks_keystroke
}
criterion_main!(editor_benches);
