//! Minimal ratatui app embedding a Slab document beside a plain Paragraph.
//! Run with `cargo run -p slab-ratatui --example embed`; press `q` to quit.

use std::{
	path::Path,
	time::{Duration, Instant},
};

use ratatui::{
	layout::{Constraint, Layout, Rect},
	widgets::{Block, Paragraph},
};
use slab_ratatui::{SlabState, SlabWidget};
use slab_tui::crossterm::{
	event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
	execute,
};

fn main() -> Result<(), String> {
	let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/embed.slab");
	let mut state = SlabState::from_file(&fixture)?;

	let mut terminal = ratatui::init();
	execute!(std::io::stdout(), EnableMouseCapture).map_err(|e| format!("terminal: {e}"))?;
	let result = run(&mut terminal, &mut state);
	let _ = execute!(std::io::stdout(), DisableMouseCapture);
	ratatui::restore();
	result
}

fn run(terminal: &mut ratatui::DefaultTerminal, state: &mut SlabState) -> Result<(), String> {
	let mut slab_area = Rect::default();
	let mut last_signal = String::from("—");
	let mut tick = Instant::now();
	loop {
		state.tick(tick.elapsed().as_secs_f64() * 1000.0);
		tick = Instant::now();
		terminal
			.draw(|frame| {
				let [left, right] =
					Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
						.areas(frame.area());
				let text = format!("Plain ratatui pane.\n\nlast signal: {last_signal}\n\nq quits.");
				frame.render_widget(Paragraph::new(text).block(Block::bordered().title("host")), left);
				slab_area = right;
				frame.render_stateful_widget(SlabWidget, right, state);
			})
			.map_err(|e| format!("terminal: {e}"))?;
		if event::poll(Duration::from_millis(50)).map_err(|e| format!("terminal: {e}"))? {
			let ev = event::read().map_err(|e| format!("terminal: {e}"))?;
			if let Event::Key(key) = &ev
				&& key.kind != KeyEventKind::Release
				&& key.code == KeyCode::Char('q')
			{
				return Ok(());
			}
			state.handle_event(&ev, slab_area);
		}
		for signal in state.drain_signals() {
			last_signal = signal.name;
		}
	}
}
