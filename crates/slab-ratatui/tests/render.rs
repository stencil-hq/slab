//! Renders the embed fixture into a TestBackend and clicks its button.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use slab_ratatui::{SlabState, SlabWidget};
use slab_tui::crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::path::Path;

const COLS: u16 = 40;
const ROWS: u16 = 12;

fn state() -> SlabState {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/embed.slab");
    SlabState::from_file(&fixture).expect("fixture compiles")
}

fn draw(terminal: &mut Terminal<TestBackend>, state: &mut SlabState) -> String {
    let mut text = String::new();
    terminal
        .draw(|frame| {
            frame.render_stateful_widget(SlabWidget, frame.area(), state);
            let buffer = frame.buffer_mut();
            for row in 0..buffer.area.height {
                for col in 0..buffer.area.width {
                    text.push_str(buffer[(col, row)].symbol());
                }
                text.push('\n');
            }
        })
        .expect("draw succeeds");
    text
}

/// Returns the (col, row) of the first cell starting `needle` in the buffer.
fn find(text: &str, needle: &str) -> (u16, u16) {
    for (row, line) in text.lines().enumerate() {
        if let Some(byte) = line.find(needle) {
            let col = line[..byte].chars().count();
            return (col as u16, row as u16);
        }
    }
    panic!("{needle:?} not on screen:\n{text}");
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn renders_document_text() {
    let mut state = state();
    let mut terminal = Terminal::new(TestBackend::new(COLS, ROWS)).expect("backend");
    let text = draw(&mut terminal, &mut state);
    assert!(text.contains("Embedded Slab"), "heading missing:\n{text}");
    assert!(text.contains("Ping"), "button label missing:\n{text}");
    assert!(text.contains("ready"), "status param missing:\n{text}");
}

#[test]
fn click_emits_signal() {
    let mut state = state();
    let mut terminal = Terminal::new(TestBackend::new(COLS, ROWS)).expect("backend");
    let text = draw(&mut terminal, &mut state);
    let (col, row) = find(&text, "Ping");
    let area = Rect::new(0, 0, COLS, ROWS);
    assert!(state.handle_event(
        &mouse(MouseEventKind::Down(MouseButton::Left), col, row),
        area
    ));
    assert!(state.handle_event(
        &mouse(MouseEventKind::Up(MouseButton::Left), col, row),
        area
    ));
    draw(&mut terminal, &mut state);
    let signals = state.drain_signals();
    assert!(
        signals.iter().any(|s| s.name == "ping"),
        "expected a ping signal, got {:?}",
        signals.iter().map(|s| s.name.clone()).collect::<Vec<_>>()
    );
    assert!(
        state.drain_signals().is_empty(),
        "drain must empty the queue"
    );
}

#[test]
fn mouse_outside_area_is_dropped() {
    let mut state = state();
    let mut terminal = Terminal::new(TestBackend::new(COLS, ROWS)).expect("backend");
    let text = draw(&mut terminal, &mut state);
    let (col, row) = find(&text, "Ping");
    // Widget offset by 5 columns: a click at the unshifted position misses.
    let area = Rect::new(5, 0, COLS - 5, ROWS);
    let hit = state.handle_event(
        &mouse(MouseEventKind::Down(MouseButton::Left), col, row),
        area,
    );
    let _ = hit; // translated relative to the shifted origin; must not panic
    assert!(!state.handle_event(
        &mouse(MouseEventKind::Down(MouseButton::Left), 2, row),
        area
    ));
}
