use slab_compile::{Options, compile};
use slab_kernel::{flatten::FrameOp, frame, scene, slir::Doc};

fn compile_instance(source: &str, width: f64, height: f64) -> frame::Instance {
	let (slir, diagnostics) = compile(source, &Options::default());
	assert!(!diagnostics.has_errors(), "{:#?}", diagnostics.0);
	let bytes = slab_slir::write(&slir.expect("valid scroll fixture"));
	let (mut instance, _) = slab_slir::instance(&bytes).expect("decode scroll fixture");
	frame::inst_set_env(&mut instance, width, height, 0, false, false);
	instance
}

fn node(doc: &Doc, instance: &frame::Instance, key: &str) -> u32 {
	scene::node_by_key(doc, &instance.st.lists, key)
}

#[test]
fn cross_scroll_tracks_exact_content_and_translates_both_axes() {
	let mut instance = compile_instance(
		r"
row#scroller w=100 h=60 scroll=both scrollbar=always scrollbar-w=4 {
  rect#content w=220 h=140 bg=#345678
}
",
		100.0,
		60.0,
	);
	let first = frame::inst_frame(&mut instance, 0.0);
	let scroller = node(&instance.doc, &instance, "#scroller");
	let content = node(&instance.doc, &instance, "#scroller/#content");
	let scroller_scene = scene::index_of(&instance.sc, scroller);
	let scroller_scene = usize::try_from(scroller_scene).expect("scroller retained");
	assert_eq!(instance.sc.content_main[scroller_scene], 220.0);
	assert_eq!(instance.sc.content_cross[scroller_scene], 140.0);
	assert_eq!(first.width, 100.0);
	assert_eq!(first.height, 60.0);

	assert!(frame::inst_set_scroll(&mut instance, "#scroller", 0, 30.0));
	assert!(frame::inst_set_scroll(&mut instance, "#scroller", 1, 40.0));
	let moved = frame::inst_frame(&mut instance, 1.0);
	let content_scene = scene::index_of(&instance.sc, content);
	let content_scene = usize::try_from(content_scene).expect("content retained");
	assert_eq!(instance.sc.x[content_scene], -30.0);
	assert_eq!(instance.sc.y[content_scene], -40.0);

	let scrollbar_rects = moved
		.ops
		.iter()
		.filter_map(|op| match op {
			FrameOp::Rect(rect) if rect.node == scroller => Some(rect),
			_ => None,
		})
		.collect::<Vec<_>>();
	assert_eq!(scrollbar_rects.len(), 4, "track and thumb for each axis");
	assert!(
		scrollbar_rects
			.iter()
			.any(|rect| { (rect.x, rect.y, rect.w, rect.h) == (0.0, 54.0, 100.0, 4.0) })
	);
	assert!(
		scrollbar_rects
			.iter()
			.any(|rect| { (rect.x, rect.y, rect.w, rect.h) == (94.0, 0.0, 4.0, 60.0) })
	);
}

#[test]
fn sticky_headers_push_off_paint_above_siblings_and_hit_at_painted_rects() {
	let mut instance = compile_instance(
		r"
col#feed w=100 h=60 scroll {
  rect#h1 w=100 h=20 sticky bg=#ff0000
  rect#b1 w=100 h=60 bg=#111111
  rect#h2 w=100 h=20 sticky bg=#00ff00
  rect#b2 w=100 h=60 bg=#222222
}
",
		100.0,
		60.0,
	);
	let _ = frame::inst_frame(&mut instance, 0.0);
	assert!(frame::inst_set_scroll(&mut instance, "#feed", 0, 70.0));
	let painted = frame::inst_frame(&mut instance, 1.0);

	let h1 = node(&instance.doc, &instance, "#feed/#h1");
	let h2 = node(&instance.doc, &instance, "#feed/#h2");
	let b2 = node(&instance.doc, &instance, "#feed/#b2");
	let h1_scene = usize::try_from(scene::index_of(&instance.sc, h1)).expect("h1 retained");
	let h2_scene = usize::try_from(scene::index_of(&instance.sc, h2)).expect("h2 retained");
	assert_eq!(instance.sc.y[h1_scene], -10.0, "next header pushes the first away");
	assert_eq!(instance.sc.y[h2_scene], 10.0, "next header approaches the start edge");

	let rect_nodes = painted
		.ops
		.iter()
		.filter_map(|op| match op {
			FrameOp::Rect(rect) => Some(rect.node),
			_ => None,
		})
		.collect::<Vec<_>>();
	let b2_paint = rect_nodes
		.iter()
		.position(|candidate| *candidate == b2)
		.unwrap();
	let h1_paint = rect_nodes
		.iter()
		.position(|candidate| *candidate == h1)
		.unwrap();
	let h2_paint = rect_nodes
		.iter()
		.position(|candidate| *candidate == h2)
		.unwrap();
	assert!(b2_paint < h1_paint && h1_paint < h2_paint);

	assert_eq!(frame::inst_hit(&instance, 5.0, 5.0).last(), Some(&h1));
	assert_eq!(frame::inst_hit(&instance, 5.0, 15.0).last(), Some(&h2));
}

#[test]
fn sticky_rejects_roots_and_cross_only_parents() {
	for source in ["col sticky { }\n", "col scroll=cross { rect sticky }\n"] {
		let (slir, diagnostics) = compile(source, &Options::default());
		assert!(slir.is_none());
		assert!(diagnostics.0.iter().any(|diag| diag.code == "sticky-ctx"), "{:#?}", diagnostics.0);
	}

	let (slir, diagnostics) = compile("col scroll=both { rect sticky }\n", &Options::default());
	assert!(slir.is_some(), "{:#?}", diagnostics.0);
	assert!(!diagnostics.has_errors());
	let root_flags = slir.unwrap().nodes.flags[0];
	assert_eq!(
		root_flags & (slab_slir::flags::SCROLL | slab_slir::flags::SCROLL_CROSS),
		slab_slir::flags::SCROLL | slab_slir::flags::SCROLL_CROSS
	);

	let (slir, diagnostics) = compile("col scroll=diagonal { rect }\n", &Options::default());
	assert!(slir.is_none());
	assert!(diagnostics.0.iter().any(|diag| diag.code == "ref"));
}
