//! The generated player module's contract. Kernel-only (no GPU): a click at
//! the play circle's center decodes to `Signal::Toggle`, the `playing` param
//! swaps the |>/|| glyph opacities, and `progress` moves the playhead knob
//! proportionally across the wave. GPU: headless pixel probes on the mint
//! play circle and the dark card bg (skipped when no adapter exists).

use slab_kernel::{
	dispatch::{self as kdispatch, Event},
	flatten::{Frame, FrameOp},
	scene,
};
use slab_native::{
	gen_player::{Doc, Signal},
	player::PlayerCore,
	renderer::{LayerInput, Renderer},
};

fn ev(etype: u32, x: f64, y: f64) -> Event {
	Event {
		etype,
		x,
		y,
		dx: 0.0,
		dy: 0.0,
		button: 0,
		clicks: 0,
		key: String::new(),
		text: String::new(),
		mods: 0,
	}
}

fn solved_doc() -> Doc {
	let mut doc = Doc::new();
	assert!(doc.ok(), "embedded SLIR failed to decode");
	doc.set_env(360.0, 640.0, false, false);
	doc
}

/// The 44x44 radius-999 transport button (#play) is the only 44x44 rect.
fn play_circle(fr: &Frame) -> (f64, f64, f64, f64) {
	fr.ops
		.iter()
		.find_map(|op| match op {
			FrameOp::Rect(r) if r.w == 44.0 && r.h == 44.0 => Some((r.x, r.y, r.w, r.h)),
			_ => None,
		})
		.expect("no 44x44 play circle rect in frame ops")
}

#[test]
fn toggle_click_at_play_circle_center() {
	let mut doc = solved_doc();
	let fr = doc.frame(0.0);
	let (px, py, pw, ph) = play_circle(&fr);
	let (cx, cy) = (px + pw / 2.0, py + ph / 2.0);

	let (_, sigs) = doc.dispatch(&ev(kdispatch::E_POINTER_DOWN, cx, cy));
	assert!(sigs.is_empty(), "no signal before release, got {sigs:?}");
	let (_, sigs) = doc.dispatch(&ev(kdispatch::E_POINTER_UP, cx, cy));
	assert_eq!(sigs.len(), 1);
	assert!(matches!(&sigs[0], Signal::Toggle { item, .. } if item.is_empty()));
}

/// Effective opacity of the transport glyph text op (`|>` or `||`): the op's
/// own opacity times every enclosing GroupPush (the kernel lowers a `when`
/// opacity patch to a group around the node).
fn glyph_opacity(fr: &Frame, glyph: &str) -> f64 {
	let mut stack: Vec<f64> = Vec::new();
	for op in &fr.ops {
		match op {
			FrameOp::GroupPush(g) => stack.push(g.opacity),
			FrameOp::GroupPop => {
				stack.pop();
			},
			FrameOp::Text(t) if fr.strings[t.str_ref as usize] == glyph => {
				return t.opacity * stack.iter().product::<f64>();
			},
			_ => {},
		}
	}
	panic!("no '{glyph}' text op in frame")
}

#[test]
fn playing_param_swaps_transport_glyph() {
	let mut doc = solved_doc();

	// default: playing=true -> || visible, |> hidden
	let fr = doc.frame(0.0);
	assert!(glyph_opacity(&fr, "|>") < 0.01, "|> should be hidden while playing");
	assert!(glyph_opacity(&fr, "||") > 0.99, "|| should be visible while playing");

	assert!(doc.set_playing(false));
	let fr = doc.frame(1.0);
	assert!(glyph_opacity(&fr, "|>") > 0.99, "|> should be visible while paused");
	assert!(glyph_opacity(&fr, "||") < 0.01, "|| should be hidden while paused");
}

/// The playhead knob: the only 8x8 rect (radius 999, color.ink).
fn knob_x(fr: &Frame) -> f64 {
	fr.ops
		.iter()
		.find_map(|op| match op {
			FrameOp::Rect(r) if r.w == 8.0 && r.h == 8.0 => Some(r.x),
			_ => None,
		})
		.expect("no 8x8 playhead knob rect in frame ops")
}

#[test]
fn progress_param_moves_playhead_knob() {
	let mut doc = solved_doc();

	assert!(doc.set_progress(20.0));
	let x20 = knob_x(&doc.frame(0.0));
	assert!(doc.set_progress(80.0));
	let x80 = knob_x(&doc.frame(1.0));

	// wave canvas is w=fill inside pad 22/22 of the 360u card -> 316u; the
	// knob row's width is param.progress of that, knob packed at its end.
	let wave_w = 360.0 - 22.0 - 22.0;
	let want = 0.6 * wave_w;
	let got = x80 - x20;
	assert!((got - want).abs() <= 2.0, "knob moved {got}u for a 60% progress step, want ~{want}u");
}

#[test]
fn pixel_probes_play_circle_and_card_bg() {
	let instance = wgpu::Instance::default();
	let Some((adapter, device, queue)) = slab_native::request_device(&instance, None) else {
		println!("SKIP: no wgpu adapter available");
		return;
	};
	println!("adapter: {} ({:?})", adapter.get_info().name, adapter.get_info().backend);
	let mut renderer = Renderer::new(device, queue);

	let mut doc = solved_doc();
	let fr = doc.frame(0.0);
	let doc_id = renderer.register_doc(&doc.inst.doc, &doc.imgs, &[]);
	let tw = fr.width.ceil() as u32;
	let th = fr.height.ceil() as u32;
	let layers = [LayerInput { doc_id, inst: &doc.inst, frame: &fr, ox: 0.0, oy: 0.0, clip: None }];
	let build = renderer.build(&layers, 1.0, tw, th);
	renderer.render(build, None, wgpu::Color::BLACK);
	let (w, h, px) = renderer.read_pixels().expect("readback failed");

	let probe = |x: u32, y: u32| -> [u8; 3] {
		let i = ((y.min(h - 1) * w + x.min(w - 1)) * 4) as usize;
		[px[i], px[i + 1], px[i + 2]]
	};

	// card bg: color.night #0A120D on the left edge below the artwork
	let night = probe(5, 200);
	assert!(
		night
			.iter()
			.zip([0x0a, 0x12, 0x0d])
			.all(|(g, want)| g.abs_diff(want) <= 3),
		"night bg probe at (5,200) got {night:?}, want [0A,12,0D] (±3)"
	);

	// play circle: mint gradient #B9F5CE(0%)..#7FE0A8(100%); probe above the
	// transport glyph, inside the 44u circle -> each channel must sit inside
	// the gradient's endpoint band (±6).
	let (cx, cy, pw, _) = play_circle(&fr);
	let mint = probe((cx + pw / 2.0) as u32, (cy + 8.0) as u32);
	let band = |v: u8, lo: u8, hi: u8| (lo.saturating_sub(6)..=hi.saturating_add(6)).contains(&v);
	assert!(
		band(mint[0], 0x7f, 0xb9) && band(mint[1], 0xe0, 0xf5) && band(mint[2], 0xa8, 0xce),
		"play-circle probe got {mint:?}, outside the mint band B9F5CE..7FE0A8 (±6)"
	);
	println!("probes ok: night {night:?}, mint {mint:?}");
}

fn key(name: &str) -> Event {
	let mut e = ev(kdispatch::E_KEY_DOWN, 0.0, 0.0);
	e.key = name.to_string();
	e
}

/// The frame paints `s` somewhere.
fn text_present(fr: &Frame, s: &str) -> bool {
	fr.ops.iter().any(|op| match op {
		FrameOp::Text(t) => fr.strings[t.str_ref as usize] == s,
		_ => false,
	})
}

/// The bg paint of node's rect op: (bg_kind, bg). None = no rect painted.
fn rect_bg(fr: &Frame, node: u32) -> Option<(u32, u32)> {
	fr.ops.iter().find_map(|op| match op {
		FrameOp::Rect(r) if r.node == node => Some((r.bg_kind, r.bg)),
		_ => None,
	})
}

/// The MOSS hover bg in the kernel's 0xAABBGGRR packing (#1B2E22 opaque).
const MOSS: u32 = 0xff222e1b;

/// The hover-ease proof: three bg samples along a 140ms ease-out
/// transition (flip t0, t0+70, t0+500) — all different, the middle
/// strictly between the endpoints on every changing channel. Hover-enter
/// from no base bg fades through the target color at alpha 0 (CSS
/// `transparent` semantics), so the alpha channel carries the ramp;
/// hover->pressed is a full OKLab color lerp.
fn assert_eases(c0: u32, c1: u32, c2: u32) {
	assert!(
		c0 != c1 && c1 != c2 && c0 != c2,
		"want three different bg values, got {c0:08X} {c1:08X} {c2:08X}"
	);
	for shift in [24, 16, 8, 0] {
		let (v0, v1, v2) = ((c0 >> shift) & 0xff, (c1 >> shift) & 0xff, (c2 >> shift) & 0xff);
		let (lo, hi) = (v0.min(v2), v0.max(v2));
		if v0 != v2 {
			assert!(
				lo < v1 && v1 < hi,
				"channel <<{shift} not strictly between: {v0:02X} {v1:02X} {v2:02X}"
			);
		}
	}
}

/// Clicking queue row 2 (child-frame geometry + hole rect offset) makes it
/// the current track: title param flips, the frame paints the new title,
/// and the freshly mounted queue rotation carries the playing marker
/// (mint gradient) on that row.
#[test]
fn queue_click_selects_track() {
	let mut core = PlayerCore::new().expect("core");
	core.set_env(360.0, 640.0, false, false);
	let _ = core.frame(0.0);
	let _ = core.queue_frame(0.0);
	assert_eq!(core.track_index(), 0);

	let hr = core.hole_rects.first().cloned().expect("hole rect");
	let (_, (rx, ry, rw, rh)) = core.queue_row(1).expect("queue row 2 rect");
	let (x, y) = (hr.x + rx + rw / 2.0, hr.y + ry + rh / 2.0);
	core.dispatch(&ev(kdispatch::E_POINTER_DOWN, x, y));
	let out = core.dispatch(&ev(kdispatch::E_POINTER_UP, x, y));

	assert_eq!(out.picked, Some(1), "pick signal should select row 2");
	assert_eq!(core.track_index(), 1);
	assert_eq!(core.title(), "This Year");
	let fr = core.frame(16.0);
	assert!(text_present(&fr, "This Year"), "main frame should title the picked track");

	// playing marker moved: in the mounted rotation, row 2 paints the mint
	// gradient and row 1 does not.
	let cf = core.queue_frame(16.0);
	let (n1, _) = core.queue_row(1).expect("row 2 in new rotation");
	let (n0, _) = core.queue_row(0).expect("row 1 in new rotation");
	assert_eq!(
		rect_bg(&cf, n1).map(|(k, _)| k),
		Some(2),
		"picked row should carry the playing gradient"
	);
	assert_ne!(
		rect_bg(&cf, n0).map(|(k, _)| k),
		Some(2),
		"row 1 should have dropped the playing gradient"
	);
}

/// Focus inside the mounted queue survives selecting a different queue
/// instance by stable scene key, and subsequent keyboard activation continues
/// to dispatch to that queue rather than the main player document.
#[test]
fn queue_selection_preserves_keyed_focus_and_keyboard_owner() {
	let mut core = PlayerCore::new().expect("core");
	core.set_env(360.0, 640.0, false, false);
	let _ = core.frame(0.0);
	let _ = core.queue_frame(0.0);

	let hr = core.hole_rects.first().cloned().expect("hole rect");
	let (_, (rx, ry, rw, rh)) = core.queue_row(1).expect("queue row 2 rect");
	let (x, y) = (hr.x + rx + rw / 2.0, hr.y + ry + rh / 2.0);
	core.dispatch(&ev(kdispatch::E_POINTER_DOWN, x, y));
	let expected_key = {
		let instance = &core.queue().inst;
		scene::key_of(&instance.doc, &instance.st.lists, instance.ds.fs.focus)
	};
	assert!(!expected_key.is_empty());

	let selected = core.dispatch(&ev(kdispatch::E_POINTER_UP, x, y));
	assert_eq!(selected.picked, Some(1));
	let _ = core.frame(16.0);
	let _ = core.queue_frame(16.0);
	let actual_key = {
		let instance = &core.queue().inst;
		scene::key_of(&instance.doc, &instance.st.lists, instance.ds.fs.focus)
	};
	assert_eq!(actual_key, expected_key);

	let activated = core.dispatch(&key("Enter"));
	assert_eq!(activated.picked, Some(1), "Enter must remain routed to the focused queue row");
}

/// Hovering SHUF eases its bg to moss over 140ms (transparent-fade in):
/// three sampled values at flip t0 / +70 / +500, middle strictly between;
/// pressing it then eases moss -> #0B1A11 the same way (OKLab).
#[test]
fn shuffle_hover_and_press_ease_bg() {
	let mut doc = solved_doc();
	let _ = doc.frame(0.0);
	let node = slab_kernel::scene::node_by_key(
		&doc.inst.doc,
		&doc.inst.st.lists,
		"#player/row@0/row@0/#shuffle",
	);
	let ix = slab_kernel::scene::index_of(&doc.inst.sc, node);
	assert!(ix >= 0, "no #shuffle in scene");
	let (x, y) = (
		doc.inst.sc.x[ix as usize] + doc.inst.sc.w[ix as usize] / 2.0,
		doc.inst.sc.y[ix as usize] + doc.inst.sc.h[ix as usize] / 2.0,
	);

	// hover-enter: the first frame after the flip anchors the tween at
	// t0=1000; the bg fades in from moss@alpha0.
	doc.dispatch(&ev(kdispatch::E_POINTER_MOVE, x, y));
	let h0 = rect_bg(&doc.frame(1000.0), node).expect("bg at hover t0").1;
	let h1 = rect_bg(&doc.frame(1070.0), node)
		.expect("bg at hover mid")
		.1;
	let h2 = rect_bg(&doc.frame(1500.0), node)
		.expect("bg at hover end")
		.1;
	assert_eases(h0, h1, h2);
	assert_eq!(h2, MOSS, "hover should settle on moss");

	// press: moss -> #0B1A11 is color->color, a real 140ms OKLab lerp.
	doc.dispatch(&ev(kdispatch::E_POINTER_DOWN, x, y));
	let c0 = rect_bg(&doc.frame(2000.0), node).expect("bg at press t0").1;
	let c1 = rect_bg(&doc.frame(2070.0), node)
		.expect("bg at press mid")
		.1;
	let c2 = rect_bg(&doc.frame(2500.0), node)
		.expect("bg at press end")
		.1;
	assert_eases(c0, c1, c2);
}

/// Hovering a queue row (through the hole, coordinates translated) runs
/// the same 140ms ease in the child instance, then a smooth pressed ease;
/// leaving the hole synthesizes a pointer leave that drops the hover.
#[test]
fn queue_row_hover_and_press_ease_bg() {
	let mut core = PlayerCore::new().expect("core");
	core.set_env(360.0, 640.0, false, false);
	let _ = core.frame(0.0);
	let _ = core.queue_frame(0.0);

	let hr = core.hole_rects.first().cloned().expect("hole rect");
	let (node, (rx, ry, rw, rh)) = core.queue_row(1).expect("queue row 2 rect");
	let (x, y) = (hr.x + rx + rw / 2.0, hr.y + ry + rh / 2.0);
	let out = core.dispatch(&ev(kdispatch::E_POINTER_MOVE, x, y));
	assert!(out.repaint, "hover enter should repaint the child");
	assert_eq!(out.cursor, kdispatch::CUR_POINTER, "queue rows are focusable -> pointer cursor");
	let h0 = rect_bg(&core.queue_frame(1000.0), node)
		.expect("hover t0")
		.1;
	let h1 = rect_bg(&core.queue_frame(1070.0), node)
		.expect("hover mid")
		.1;
	let h2 = rect_bg(&core.queue_frame(1500.0), node)
		.expect("hover end")
		.1;
	assert_eases(h0, h1, h2);
	assert_eq!(h2, MOSS, "row hover should settle on moss");

	core.dispatch(&ev(kdispatch::E_POINTER_DOWN, x, y));
	let c0 = rect_bg(&core.queue_frame(2000.0), node)
		.expect("press t0")
		.1;
	let c1 = rect_bg(&core.queue_frame(2070.0), node)
		.expect("press mid")
		.1;
	let c2 = rect_bg(&core.queue_frame(2500.0), node)
		.expect("press end")
		.1;
	assert_eases(c0, c1, c2);

	// drag out of the hole before releasing (capture routes the move to
	// the child as a leave; releasing off the row fires no pick), then
	// move on main: hover and pressed fade back out to no bg.
	core.dispatch(&ev(kdispatch::E_POINTER_MOVE, 5.0, 5.0));
	core.dispatch(&ev(kdispatch::E_POINTER_UP, 5.0, 5.0));
	core.dispatch(&ev(kdispatch::E_POINTER_MOVE, 5.0, 5.0));
	assert_eq!(core.track_index(), 0, "no pick on release off the row");
	let _ = core.queue_frame(4000.0); // anchors the fade-out tween at t=4000
	assert_eq!(
		rect_bg(&core.queue_frame(5000.0), node),
		None,
		"hover bg should drop after the pointer leaves the hole"
	);
}

/// ArrowRight x3 walks the focus ring (shuffle -> prev -> play) and Enter
/// activates: Signal::Toggle, all through kernel dispatch.
#[test]
fn arrow_ring_enter_toggles() {
	let mut doc = solved_doc();
	let _ = doc.frame(0.0);
	for _ in 0..3 {
		let (_, sigs) = doc.dispatch(&key("ArrowRight"));
		assert!(sigs.is_empty(), "arrows only move focus, got {sigs:?}");
	}
	let (_, sigs) = doc.dispatch(&key("Enter"));
	assert_eq!(sigs.len(), 1);
	assert!(matches!(&sigs[0], Signal::Toggle { item, .. } if item.is_empty()));
}
