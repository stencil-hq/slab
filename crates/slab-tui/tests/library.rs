//! Public terminal host API contracts.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use slab_kernel::cells::CellGrid;
use slab_tui::{Painter, Translated, Translator, translate};

fn default_ink_grid() -> CellGrid {
	CellGrid {
		cols:      1,
		rows:      1,
		ch:        vec![u32::from('A')],
		cl:        vec![String::new()],
		fg:        vec![0],
		bg:        vec![0],
		flags:     vec![0],
		diag_code: Vec::new(),
		diag_msg:  Vec::new(),
		clip_x0:   vec![0],
		clip_y0:   vec![0],
		clip_x1:   vec![1],
		clip_y1:   vec![1],
	}
}

#[test]
fn painter_preserves_terminal_default_foreground() {
	let mut painter = Painter::with_truecolor(true);
	painter.paint(&default_ink_grid(), 1, 1, true);

	assert!(painter.buffer().contains('A'));
	assert!(!painter.buffer().contains(";38;"));
}

#[test]
fn public_translation_emits_key_then_text() {
	let mut translator = Translator::new();
	let translated = translate(
		&mut translator,
		Event::Key(KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT)),
	);

	let Translated::Events(pair) = translated else {
		panic!("printable key must emit key and text events");
	};
	let (key, Some(text)) = *pair else {
		panic!("printable key must pair key with text");
	};
	assert_eq!(key.etype, 4);
	assert_eq!(key.key, "A");
	assert_eq!(key.mods, 1);
	assert_eq!(text.etype, 5);
	assert_eq!(text.text, "A");
}

#[test]
fn public_translation_counts_clicks_and_tracks_move_delta() {
	let mut translator = Translator::new();
	let down = |column| {
		Event::Mouse(MouseEvent {
			kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
			column,
			row: 2,
			modifiers: KeyModifiers::NONE,
		})
	};

	let Translated::Events(first) = translate(&mut translator, down(1)) else {
		panic!("mouse down must emit one pointer event");
	};
	let (first, None) = *first else {
		panic!("mouse down emits one event")
	};
	let Translated::Events(second) = translate(&mut translator, down(1)) else {
		panic!("second mouse down must emit one pointer event");
	};
	let (second, None) = *second else {
		panic!("second mouse down emits one event")
	};
	assert_eq!((first.clicks, second.clicks), (1, 2));

	let moved = Event::Mouse(MouseEvent {
		kind:      MouseEventKind::Moved,
		column:    3,
		row:       4,
		modifiers: KeyModifiers::NONE,
	});
	let Translated::Events(pointer) = translate(&mut translator, moved) else {
		panic!("mouse move must emit one pointer event");
	};
	let (pointer, None) = *pointer else {
		panic!("mouse move emits one event")
	};
	assert_eq!((pointer.dx, pointer.dy), (16.0, 32.0));
}

#[test]
fn public_translation_reports_resize() {
	let mut translator = Translator::new();
	assert!(matches!(
		translate(&mut translator, Event::Resize(120, 40)),
		Translated::Resize(120, 40)
	));
}
