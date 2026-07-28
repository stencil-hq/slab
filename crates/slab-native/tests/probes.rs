//! Headless wgpu pixel probes: render conformance cases offscreen at their
//! manifest env and assert known pixels against the frozen frame.json
//! geometry + SLIR paints (tolerance ±3/255 per channel). No window; the
//! adapter is requested surfaceless (Metal supports headless). When no
//! adapter exists the tests skip with a clear message — `--headless-frame`
//! remains the manual verification hook.

use std::path::{Path, PathBuf};

use slab_kernel::{
	flatten::{Frame, FrameOp},
	frame as kframe,
};
use slab_native::renderer::{LayerInput, Renderer};

fn case_path(name: &str) -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
		.join("../../conformance/cases")
		.join(format!("{name}.slab"))
}

/// Compile a conformance case and solve it at its manifest env
/// (vh 0 = unbounded, client svg — the class the goldens were frozen with).
fn solve(name: &str, width: f64) -> (kframe::Instance, Frame, Vec<Vec<u8>>) {
	let path = case_path(name);
	let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
	solve_source(name, &src, path.parent().unwrap(), width)
}

fn solve_source(
	name: &str,
	src: &str,
	base_dir: &Path,
	width: f64,
) -> (kframe::Instance, Frame, Vec<Vec<u8>>) {
	let opts = slab_compile::Options {
		embed_assets: true,
		base_dir: base_dir.to_path_buf(),
		..slab_compile::Options::default()
	};
	let (slir, diags) = slab_compile::compile(src, &opts);
	let slir = slir.unwrap_or_else(|| panic!("{name} failed to compile: {:?}", diags.0));
	let bytes = slab_slir::write(&slir);
	let (mut inst, imgs) = slab_slir::instance(&bytes)
		.unwrap_or_else(|err| panic!("{name}: kernel decode failed: {err}"));
	assert!(inst.ok, "{name}: kernel decode failed: {:?}", inst.doc().errs);
	kframe::inst_set_env(&mut inst, width, 0.0, 3, false, false);
	let fr = kframe::inst_frame(&mut inst, 0.0);
	(inst, fr, imgs)
}

struct Probe {
	x:    u32,
	y:    u32,
	want: [u8; 3],
	what: &'static str,
}

fn render_and_read(
	renderer: &mut Renderer,
	inst: &kframe::Instance,
	fr: &Frame,
	imgs: &[Vec<u8>],
) -> (u32, u32, Vec<u8>) {
	let doc_id = renderer.register_doc(inst.doc(), imgs, &[]);
	let tw = fr.width.ceil() as u32;
	let th = fr.height.ceil() as u32;
	let layers = [LayerInput { doc_id, inst, frame: fr, ox: 0.0, oy: 0.0, clip: None }];
	let build = renderer.build(&layers, 1.0, tw, th);
	renderer.render(build, None, wgpu::Color::BLACK);
	renderer.read_pixels().expect("readback failed")
}

fn check(px: &[u8], w: u32, probes: &[Probe], case: &str) {
	for p in probes {
		let i = ((p.y * w + p.x) * 4) as usize;
		let got = [px[i], px[i + 1], px[i + 2]];
		let ok = got
			.iter()
			.zip(p.want)
			.all(|(g, want)| g.abs_diff(want) <= 3);
		assert!(
			ok,
			"{case}: probe '{}' at ({},{}) got {:?}, want {:?} (±3)",
			p.what, p.x, p.y, got, p.want
		);
		println!("{case}: {} at ({},{}) = {:?} (want {:?}) ok", p.what, p.x, p.y, got, p.want);
	}
}

fn make_renderer() -> Option<Renderer> {
	let instance = wgpu::Instance::default();
	let (adapter, device, queue) = slab_native::request_device(&instance, None)?;
	println!("adapter: {} ({:?})", adapter.get_info().name, adapter.get_info().backend);
	Some(Renderer::new(device, queue))
}

#[test]
fn probes_l1_box_basics() {
	let Some(mut renderer) = make_renderer() else {
		eprintln!("SKIP: no wgpu adapter available; verify via `slab-native --headless-frame`");
		return;
	};
	// frozen geometry (conformance/expected/l1-box-basics.frame.json):
	// root 400x179 #101418; node2 red @(16,24,24,24); node3 green
	// @(188,44,24,12); node4 blue @(360,24,24,24); node7 #444 @(95,64,225,30)
	let (inst, fr, bytes) = solve("l1-box-basics", 800.0);
	assert_eq!(fr.width, 400.0, "frozen frame width moved");
	assert_eq!(fr.height, 179.0, "frozen frame height moved");
	let (w, _h, px) = render_and_read(&mut renderer, &inst, &fr, &bytes);
	check(
		&px,
		w,
		&[
			Probe { x: 2, y: 2, want: [0x10, 0x14, 0x18], what: "root bg tl" },
			Probe { x: 390, y: 170, want: [0x10, 0x14, 0x18], what: "root bg br" },
			Probe { x: 28, y: 36, want: [0xff, 0x00, 0x00], what: "red box" },
			Probe { x: 200, y: 50, want: [0x00, 0xff, 0x00], what: "green box" },
			Probe { x: 372, y: 36, want: [0x00, 0x00, 0xff], what: "blue box" },
			Probe { x: 207, y: 79, want: [0x44, 0x44, 0x44], what: "fill row mid" },
		],
		"l1-box-basics",
	);
}

#[test]
fn probes_l3_paint() {
	let Some(mut renderer) = make_renderer() else {
		eprintln!("SKIP: no wgpu adapter available; verify via `slab-native --headless-frame`");
		return;
	};
	// frozen geometry (conformance/expected/l3-paint.frame.json):
	// grad0 linear(135, #241A4E, #E8865E) @(10,10,240,30)
	// grad1 radial(#FFE0B0, #FFB37C00)    @(10,48,240,30)
	// grad2 linear(90, #49A9FF, #10141B)  @(10,86,240,30)
	// solids: #ffffff88 @(10,124) #4080ff @(10,152) #49a9ff @(10,180)
	// white+black stroke @(10,208,100,20); no root bg (clear = black)
	let (inst, fr, bytes) = solve("l3-paint", 800.0);
	assert_eq!(fr.width, 280.0, "frozen frame width moved");
	assert_eq!(fr.height, 238.0, "frozen frame height moved");
	let (w, _h, px) = render_and_read(&mut renderer, &inst, &fr, &bytes);
	check(
		&px,
		w,
		&[
			// linear 135°: midpoint color at the rect center
			Probe { x: 130, y: 25, want: [0x86, 0x50, 0x56], what: "grad0 mid" },
			// radial: first stop at the center
			Probe { x: 130, y: 63, want: [0xff, 0xe0, 0xb0], what: "grad1 center" },
			// linear 90° (left->right): stops at both ends
			Probe { x: 12, y: 101, want: [0x49, 0xa9, 0xff], what: "grad2 left" },
			Probe { x: 248, y: 101, want: [0x10, 0x14, 0x1b], what: "grad2 right" },
			Probe { x: 130, y: 101, want: [0x2c, 0x5e, 0x8d], what: "grad2 mid" },
			// #fff8 over the black clear: 0x88-weighted white
			Probe { x: 60, y: 134, want: [0x88, 0x88, 0x88], what: "8-digit hex alpha" },
			Probe { x: 60, y: 162, want: [0x40, 0x80, 0xff], what: "rgb()" },
			Probe { x: 60, y: 190, want: [0x49, 0xa9, 0xff], what: "oklch()" },
			Probe { x: 60, y: 218, want: [0xff, 0xff, 0xff], what: "white fill" },
		],
		"l3-paint",
	);
}

#[test]
fn probes_gpu_dashes_and_inset_shadow() {
	let Some(mut renderer) = make_renderer() else {
		eprintln!("SKIP: no wgpu adapter available; verify via `slab-native --headless-frame`");
		return;
	};

	let (inst, fr, bytes) = solve("l2-style", 800.0);
	let (w, _h, px) = render_and_read(&mut renderer, &inst, &fr, &bytes);
	check(
		&px,
		w,
		&[
			Probe { x: 16, y: 65, want: [0x8a, 0x97, 0xa8], what: "rect dash on" },
			Probe { x: 22, y: 65, want: [0x0e, 0x11, 0x16], what: "rect dash gap" },
			Probe { x: 20, y: 109, want: [70, 75, 82], what: "inset shadow rim" },
			Probe { x: 20, y: 120, want: [0x20, 0x26, 0x2e], what: "inset shadow center" },
		],
		"l2-style gpu effects",
	);

	let src = "stack w=100 h=30 bg=#101418 {\npath \"M10 15 L90 15\" stroke=#ffffff stroke-w=2 \
	           stroke-dash=6,4\n}\n";
	let base = Path::new(env!("CARGO_MANIFEST_DIR"));
	let (inst, fr, bytes) = solve_source("path-dash", src, base, 100.0);
	let (w, _h, px) = render_and_read(&mut renderer, &inst, &fr, &bytes);
	check(
		&px,
		w,
		&[Probe { x: 12, y: 15, want: [0xff, 0xff, 0xff], what: "path dash on" }, Probe {
			x:    18,
			y:    15,
			want: [0x10, 0x14, 0x18],
			what: "path dash gap",
		}],
		"path dash gpu effect",
	);
}

#[test]
fn probes_text_coverage() {
	let Some(mut renderer) = make_renderer() else {
		eprintln!("SKIP: no wgpu adapter available; verify via `slab-native --headless-frame`");
		return;
	};
	// l1-text op0: "The quick brown fox jumps over the lazy" @ x=12,
	// baseline 26.893, measured_w 269.609, size 14, color #e6edf3 — assert
	// glyph coverage (bright pixels) inside the line box on the black clear.
	let (inst, fr, bytes) = solve("l1-text", 800.0);
	let Some(FrameOp::Text(t)) = fr
		.ops
		.iter()
		.find(|op| matches!(op, FrameOp::Text(_)))
		.cloned()
	else {
		panic!("l1-text has no Text op");
	};
	let (w, h, px) = render_and_read(&mut renderer, &inst, &fr, &bytes);
	let x0 = t.x as u32;
	let x1 = ((t.x + t.measured_w).ceil() as u32).min(w);
	let y0 = ((t.y_baseline - t.size).floor().max(0.0)) as u32;
	let y1 = (t.size.mul_add(0.3, t.y_baseline).ceil() as u32).min(h);
	let mut lit = 0usize;
	for y in y0..y1 {
		for x in x0..x1 {
			let i = ((y * w + x) * 4) as usize;
			if px[i] > 100 {
				lit += 1;
			}
		}
	}
	println!("l1-text: {lit} lit pixels in [{x0},{x1})x[{y0},{y1}) (box {}x{})", x1 - x0, y1 - y0);
	assert!(lit > 200, "expected >200 glyph pixels inside the first Text op box, got {lit}");
}

/// Exercise the layer machinery (GroupPush/Pop opacity+blur, Backdrop,
/// rotation, clips, lyon paths) on the heavier examples: must not panic and
/// must produce non-background pixels.
#[test]
fn renders_layered_examples() {
	let Some(mut renderer) = make_renderer() else {
		eprintln!("SKIP: no wgpu adapter available; verify via `slab-native --headless-frame`");
		return;
	};
	let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
	for name in ["08-glass", "04-poster", "09-widget"] {
		let path = base.join(format!("{name}.slab"));
		let src =
			std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
		let opts = slab_compile::Options {
			embed_assets: true,
			base_dir: base.clone(),
			..slab_compile::Options::default()
		};
		let (slir, diags) = slab_compile::compile(&src, &opts);
		let slir = slir.unwrap_or_else(|| panic!("{name} failed to compile: {:?}", diags.0));
		let bytes = slab_slir::write(&slir);
		let (mut inst, imgs) =
			slab_slir::instance(&bytes).unwrap_or_else(|err| panic!("{name}: decode failed: {err}"));
		assert!(inst.ok, "{name}: decode failed: {:?}", inst.doc().errs);
		kframe::inst_set_env(&mut inst, 800.0, 0.0, 1, false, false);
		let fr = kframe::inst_frame(&mut inst, 250.0);
		let (w, h, px) = render_and_read(&mut renderer, &inst, &fr, &imgs);
		let lit = px
			.as_chunks::<4>()
			.0
			.iter()
			.filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
			.count();
		println!("{name}: {w}x{h}, {lit} non-background pixels");
		if let Ok(dir) = std::env::var("PROBE_DUMP") {
			let out = PathBuf::from(dir).join(format!("{name}.png"));
			slab_native::demo::write_png(&out, w, h, &px).unwrap();
			println!("dumped {}", out.display());
		}
		assert!(
			lit > (w * h / 20) as usize,
			"{name}: suspiciously empty render ({lit} lit of {})",
			w * h
		);
	}
}
