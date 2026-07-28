use slab_native::{
	NativeDocument,
	view::{NativeShell, ShellEvent, ShellHost, ShellOptions},
};
use winit::{
	event_loop::{ActiveEventLoop, EventLoop},
	window::Window,
};

struct AppEvent;
struct SlateHost;

impl ShellHost<AppEvent> for SlateHost {
	fn effects(&mut self, _doc: &mut NativeDocument, _effects: &slab_kernel::dispatch::Effects) {}

	fn user_event(
		&mut self,
		_doc: &mut NativeDocument,
		_window: &Window,
		_el: &ActiveEventLoop,
		_event: AppEvent,
	) -> bool {
		false
	}
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let src = include_str!("../doc/editor.slab");
	let (slir, _) = slab_compile::compile(src, &Default::default());
	let slir = slir.unwrap();
	let bytes = slab_slir::write(&slir);
	let mut doc = NativeDocument::decode(&bytes)?;

	// Push initial params/lists on doc.inst
	// Just minimal wiring as requested

	let event_loop = EventLoop::<ShellEvent<AppEvent>>::with_user_event().build()?;
	// install_sigterm(event_loop.create_proxy());
	let options =
		ShellOptions { title: "Slate".into(), width: 900.0, height: 640.0, ..Default::default() };
	let mut app = NativeShell::new(doc, options, event_loop.create_proxy(), SlateHost);
	event_loop.run_app(&mut app)?;

	Ok(())
}
