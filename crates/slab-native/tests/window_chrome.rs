//! The `--undecorated` window-chrome contract, kernel-only (no GPU):
//! `when undecorated` gates the document's own titlebar behind the
//! host-set state, chrome buttons fire the reserved `window-*` activation
//! signals, and the viewer's mapping/precedence rules hold (a drag region
//! yields to the control nested inside it).

use slab_kernel::{
	dispatch::{self as kdispatch, Event},
	frame as kframe, scene,
};
use slab_native::view::{WindowCmd, act_signal};

const CHROME_DOC: &str = r#"
col w=fill h=fill bg=#101018 {
  when undecorated {
    row #bar h=32 w=fill act=window-drag align=center pad=0,8 gap=8 {
      text "app" size=12 color=#EAF2FF
      spacer
      rect #mini  w=12 h=12 act=window-minimize bg=#444455
      rect #maxi  w=12 h=12 act=window-maximize bg=#444455
      rect #close w=12 h=12 act=window-close   bg=#F4644A
    }
  }
  text "body" size=14 color=#EAF2FF
}
"#;

fn compile(src: &str) -> Vec<u8> {
	let (slir, diags) = slab_compile::compile(src, &slab_compile::Options::default());
	assert!(!diags.has_errors(), "chrome doc failed to compile: {:?}", diags.0);
	slab_slir::write(&slir.expect("no SLIR"))
}

const fn ev(etype: u32, x: f64, y: f64) -> Event {
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
		clauses: Vec::new(),
		mods: 0,
	}
}

#[test]
fn undecorated_state_gates_chrome_and_signals_map() {
	let bytes = compile(CHROME_DOC);

	// decorated: the doc node exists (compile-time tree) but the titlebar
	// never enters the SOLVED scene — `when undecorated` children detach
	// while the state is off
	let (mut plain, _) = slab_slir::instance(&bytes).expect("SLIR decode failed");
	assert!(plain.ok, "SLIR decode failed: {:?}", plain.doc().errs);
	kframe::inst_set_env(&mut plain, 640.0, 400.0, 1, false, false);
	kframe::inst_frame(&mut plain, 0.0);
	let bar = scene::node_by_key(plain.doc(), &plain.st.lists, "col@0/#bar");
	assert_ne!(bar, slab_kernel::slir::NONE, "titlebar missing from doc tree");
	assert!(
		scene::index_of(&plain.sc, bar) < 0,
		"titlebar solved into the scene without the undecorated state"
	);

	// undecorated: state on -> chrome solves, controls carry window signals
	let (mut inst, _) = slab_slir::instance(&bytes).expect("SLIR decode failed");
	kframe::inst_set_state(&mut inst, "undecorated", true);
	kframe::inst_set_env(&mut inst, 640.0, 400.0, 1, false, false);
	kframe::inst_frame(&mut inst, 0.0);
	let bar = scene::node_by_key(inst.doc(), &inst.st.lists, "col@0/#bar");
	let close = scene::node_by_key(inst.doc(), &inst.st.lists, "col@0/#bar/#close");
	assert!(scene::index_of(&inst.sc, bar) >= 0, "no titlebar in scene");
	assert!(scene::index_of(&inst.sc, close) >= 0, "no close control in scene");

	// reserved-name mapping straight off the act= bindings
	assert_eq!(act_signal(inst.doc(), bar).and_then(WindowCmd::from_signal), Some(WindowCmd::Drag));
	assert_eq!(
		act_signal(inst.doc(), close).and_then(WindowCmd::from_signal),
		Some(WindowCmd::Close)
	);
	assert_eq!(WindowCmd::from_signal("save"), None);

	// a click on the close control emits the reserved signal through the
	// ordinary kernel dispatch path (what ViewApp::dispatch intercepts)
	let ix = scene::index_of(&inst.sc, close);
	assert!(ix >= 0);
	let entry = &inst.sc.entries[ix as usize];
	let (cx, cy) = (entry.x + entry.w / 2.0, entry.y + entry.h / 2.0);
	kframe::inst_dispatch(&mut inst, &ev(kdispatch::E_POINTER_DOWN, cx, cy));
	let eff = kframe::inst_dispatch(&mut inst, &ev(kdispatch::E_POINTER_UP, cx, cy));
	let names: Vec<&str> = eff
		.sig_name
		.iter()
		.map(|&r| inst.doc().strs[r as usize].as_str())
		.collect();
	assert_eq!(names, vec!["window-close"], "close click signal");

	// pressing the bar itself: the deepest act binding is the drag region
	let bix = scene::index_of(&inst.sc, bar);
	let bentry = &inst.sc.entries[bix as usize];
	let (bx, by) = (bentry.x + 40.0, bentry.y + bentry.h / 2.0);
	let chain = kframe::inst_hit(&inst, bx, by);
	let nearest = chain
		.iter()
		.rev()
		.find_map(|&n| act_signal(inst.doc(), n))
		.expect("no act binding on the bar press point");
	assert_eq!(WindowCmd::from_signal(nearest), Some(WindowCmd::Drag));

	// ...but a press on the close control resolves to Close, not Drag
	let chain = kframe::inst_hit(&inst, cx, cy);
	let nearest = chain
		.iter()
		.rev()
		.find_map(|&n| act_signal(inst.doc(), n))
		.expect("no act binding on the close press point");
	assert_eq!(WindowCmd::from_signal(nearest), Some(WindowCmd::Close));
}
