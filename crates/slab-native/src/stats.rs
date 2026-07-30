//! Opt-in native frame timing and glyph-pipeline statistics.

use std::{
	fmt::Write as _,
	fs::File,
	io::{BufWriter, Write as _},
	path::{Path, PathBuf},
	time::{Duration, Instant},
};

use crate::atlas::AtlasCounters;

#[derive(Clone, Copy, Debug)]
pub enum InputKind {
	Key,
	Wheel,
}

#[derive(Clone, Copy, Debug, Default)]
struct FrameRecord {
	t_ms:           f64,
	kernel_ns:      u64,
	build_ns:       u64,
	render_ns:      u64,
	present_ns:     u64,
	total_ns:       u64,
	rasterized:     u32,
	upload_bytes:   u64,
	input_key_ns:   Option<u64>,
	input_wheel_ns: Option<u64>,
}

/// Timing state for one successfully presented frame.
pub struct FrameMeasurement {
	t_ms:        f64,
	total_start: Instant,
	stage_start: Instant,
	kernel:      Duration,
	build:       Duration,
	render:      Duration,
	present:     Duration,
}

impl FrameMeasurement {
	pub(crate) fn start() -> Self {
		let now = Instant::now();
		Self {
			t_ms:        0.0,
			total_start: now,
			stage_start: now,
			kernel:      Duration::ZERO,
			build:       Duration::ZERO,
			render:      Duration::ZERO,
			present:     Duration::ZERO,
		}
	}

	pub(crate) const fn set_t_ms(&mut self, t_ms: f64) {
		self.t_ms = t_ms;
	}

	pub(crate) fn begin_kernel(&mut self) {
		self.stage_start = Instant::now();
	}

	pub(crate) fn end_kernel(&mut self) {
		self.kernel = self.stage_start.elapsed();
	}

	pub(crate) fn begin_build(&mut self) {
		self.stage_start = Instant::now();
	}

	pub(crate) fn end_build(&mut self) {
		self.build = self.stage_start.elapsed();
	}

	pub(crate) fn begin_render(&mut self) {
		self.stage_start = Instant::now();
	}

	pub(crate) fn end_render(&mut self) {
		self.render = self.stage_start.elapsed();
	}

	pub(crate) fn begin_present(&mut self) {
		self.stage_start = Instant::now();
	}

	pub(crate) fn finish(mut self, stats: &mut FrameStats, counters: AtlasCounters) {
		let presented = Instant::now();
		self.present = presented.duration_since(self.stage_start);
		stats.frames.push(FrameRecord {
			t_ms:           self.t_ms,
			kernel_ns:      nanos(self.kernel),
			build_ns:       nanos(self.build),
			render_ns:      nanos(self.render),
			present_ns:     nanos(self.present),
			total_ns:       nanos(presented.duration_since(self.total_start)),
			rasterized:     counters.rasterized_glyphs,
			upload_bytes:   counters.upload_bytes,
			input_key_ns:   stats
				.pending_key
				.take()
				.map(|stamp| nanos(presented.duration_since(stamp))),
			input_wheel_ns: stats
				.pending_wheel
				.take()
				.map(|stamp| nanos(presented.duration_since(stamp))),
		});
	}
}

/// Session-bounded collector for frame timings and input-to-present latency.
pub struct FrameStats {
	frames:        Vec<FrameRecord>,
	pending_key:   Option<Instant>,
	pending_wheel: Option<Instant>,
	csv_path:      Option<PathBuf>,
}

impl FrameStats {
	pub(crate) const fn new(csv_path: Option<PathBuf>) -> Self {
		Self { frames: Vec::new(), pending_key: None, pending_wheel: None, csv_path }
	}

	pub(crate) fn input(&mut self, kind: InputKind) {
		let pending = match kind {
			InputKind::Key => &mut self.pending_key,
			InputKind::Wheel => &mut self.pending_wheel,
		};
		// Keep the first event in a burst: all events are resolved by the same
		// next present, and this measures the full wait for that batch.
		if pending.is_none() {
			*pending = Some(Instant::now());
		}
	}

	pub(crate) fn finish(self) -> Result<String, String> {
		if let Some(path) = &self.csv_path {
			self.write_csv(path)?;
		}
		Ok(self.summary())
	}

	fn summary(&self) -> String {
		let mut out = format!("slab-native: frame stats ({} frames)\n", self.frames.len());
		out.push_str("  metric                 mean      p50      p95      p99      max  (us)\n");
		for (name, values) in [
			("kernel", self.frames.iter().map(|f| f.kernel_ns).collect()),
			("build", self.frames.iter().map(|f| f.build_ns).collect()),
			("render", self.frames.iter().map(|f| f.render_ns).collect()),
			("present", self.frames.iter().map(|f| f.present_ns).collect()),
			("total", self.frames.iter().map(|f| f.total_ns).collect()),
			("input key", self.frames.iter().filter_map(|f| f.input_key_ns).collect()),
			(
				"input wheel",
				self
					.frames
					.iter()
					.filter_map(|f| f.input_wheel_ns)
					.collect(),
			),
		] {
			write_summary_row(&mut out, name, values);
		}
		let glyphs: u64 = self.frames.iter().map(|f| u64::from(f.rasterized)).sum();
		let uploads: u64 = self.frames.iter().map(|f| f.upload_bytes).sum();
		let _ = writeln!(out, "  rasterized glyphs: {glyphs}");
		let _ = write!(out, "  atlas upload bytes: {uploads}");
		out
	}

	fn write_csv(&self, path: &Path) -> Result<(), String> {
		let file = File::create(path)
			.map_err(|e| format!("cannot create stats CSV {}: {e}", path.display()))?;
		let mut out = BufWriter::new(file);
		writeln!(
			out,
			"t_ms,kernel_us,build_us,render_us,present_us,total_us,rasterized_glyphs,\
			 atlas_upload_bytes,input_key_us,input_wheel_us"
		)
		.map_err(|e| format!("cannot write stats CSV {}: {e}", path.display()))?;
		for frame in &self.frames {
			writeln!(
				out,
				"{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{},{},{},{}",
				frame.t_ms,
				micros(frame.kernel_ns),
				micros(frame.build_ns),
				micros(frame.render_ns),
				micros(frame.present_ns),
				micros(frame.total_ns),
				frame.rasterized,
				frame.upload_bytes,
				optional_micros(frame.input_key_ns),
				optional_micros(frame.input_wheel_ns),
			)
			.map_err(|e| format!("cannot write stats CSV {}: {e}", path.display()))?;
		}
		out.flush()
			.map_err(|e| format!("cannot write stats CSV {}: {e}", path.display()))
	}
}

fn nanos(duration: Duration) -> u64 {
	u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn micros(ns: u64) -> f64 {
	ns as f64 / 1_000.0
}

fn optional_micros(ns: Option<u64>) -> String {
	ns.map(|value| format!("{:.3}", micros(value)))
		.unwrap_or_default()
}

fn write_summary_row(out: &mut String, name: &str, mut values: Vec<u64>) {
	if values.is_empty() {
		let _ = writeln!(out, "  {name:<20}        -        -        -        -        -");
		return;
	}
	values.sort_unstable();
	let mean = values.iter().map(|&v| v as f64).sum::<f64>() / values.len() as f64;
	let _ = writeln!(
		out,
		"  {name:<20} {:8.1} {:8.1} {:8.1} {:8.1} {:8.1}",
		mean / 1_000.0,
		micros(percentile(&values, 50)),
		micros(percentile(&values, 95)),
		micros(percentile(&values, 99)),
		micros(*values.last().expect("non-empty values")),
	);
}

const fn percentile(sorted: &[u64], percent: usize) -> u64 {
	let rank = (percent * sorted.len()).div_ceil(100);
	sorted[rank.saturating_sub(1)]
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn percentile_uses_nearest_rank() {
		let values = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
		assert_eq!(percentile(&values, 50), 5);
		assert_eq!(percentile(&values, 95), 10);
		assert_eq!(percentile(&values, 99), 10);
	}

	#[test]
	fn summary_aggregates_frames_and_counters() {
		let stats = FrameStats {
			frames:        vec![
				FrameRecord { kernel_ns: 1_000, rasterized: 2, upload_bytes: 10, ..Default::default() },
				FrameRecord { kernel_ns: 3_000, rasterized: 3, upload_bytes: 20, ..Default::default() },
			],
			pending_key:   None,
			pending_wheel: None,
			csv_path:      None,
		};
		let summary = stats.summary();
		assert!(summary.contains("frame stats (2 frames)"));
		assert!(summary.contains("rasterized glyphs: 5"));
		assert!(summary.contains("atlas upload bytes: 30"));
	}
}
