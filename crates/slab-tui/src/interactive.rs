//! Interactive terminal loop: crossterm raw mode + alt screen, terminal
//! keys/mouse → kernel Events, kernel cell grid → minimal ANSI diffs.
//! The kernel owns layout, focus, editing, scroll, and motion; this loop
//! only paces frames (--fps, only while dirty/animating), diffs cell
//! grids, surfaces signals on the optional --debug footer row, and — in
//! `--examples` gallery mode — swaps documents on Ctrl-N/Ctrl-P.

use crate::app;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::{cursor, execute, terminal};
use slab_kernel::{cells, dispatch, frame as kframe};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn ioerr(e: std::io::Error) -> String {
    format!("terminal: {e}")
}

/// Converts terminal dimensions to slab layout units.
pub fn terminal_env(cols: u16, doc_rows: u16) -> (f64, f64) {
    (f64::from(cols) * cells::CW, f64::from(doc_rows) * cells::CH)
}

/// Applies terminal dimensions and returns the document row count.
pub fn resize(
    inst: &mut kframe::Instance,
    cols: u16,
    rows: u16,
    reserved_rows: u16,
    dark: bool,
    coarse: bool,
) -> u16 {
    let doc_rows = rows.saturating_sub(reserved_rows).max(1);
    let (vw, vh) = terminal_env(cols, doc_rows);
    kframe::inst_set_env(inst, vw, vh, 2, dark, coarse);
    doc_rows
}

/// Raw-mode alternate-screen terminal lifecycle.
///
/// Create this value with [`Terminal::new`]. Dropping it restores the terminal.
pub struct Terminal {
    kitty: bool,
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let mut out = std::io::stdout();
        if self.kitty {
            let _ = execute!(out, event::PopKeyboardEnhancementFlags);
        }
        let _ = execute!(
            out,
            event::DisableBracketedPaste,
            event::DisableMouseCapture,
            terminal::LeaveAlternateScreen,
            cursor::Show
        );
        let _ = terminal::disable_raw_mode();
    }
}

/// Effective (fg, bg) of a cell; cells::NO_COLOR = terminal default.
fn cell_colors(g: &cells::CellGrid, ix: usize) -> (u32, u32) {
    let fg = if g.flags[ix] & cells::CF_FG != 0 {
        g.fg[ix]
    } else {
        cells::NO_COLOR
    };
    let bg = if g.flags[ix] & cells::CF_BG != 0 {
        g.bg[ix]
    } else {
        cells::NO_COLOR
    };
    (fg, bg)
}

fn cell_changed(old: &cells::CellGrid, new: &cells::CellGrid, ix: usize) -> bool {
    let (old_fg, old_bg) = cell_colors(old, ix);
    let (new_fg, new_bg) = cell_colors(new, ix);
    old.ch[ix] != new.ch[ix]
        || old.cl[ix] != new.cl[ix]
        || old_fg != new_fg
        || old_bg != new_bg
        || old.flags[ix] & cells::CF_STRIKE != new.flags[ix] & cells::CF_STRIKE
}

/// Nearest xterm-256 index for 0xRRGGBB (6×6×6 cube + gray ramp).
fn x256(rgb: u32) -> u8 {
    let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
    if r == g && g == b {
        return if r < 8 {
            16
        } else if r > 248 {
            231
        } else {
            (232 + (r - 8) / 10).min(255) as u8
        };
    }
    let q = |c: u32| -> u32 {
        if c < 48 {
            0
        } else if c < 115 {
            1
        } else {
            ((c - 35) / 40).min(5)
        }
    };
    (16 + 36 * q(r) + 6 * q(g) + q(b)) as u8
}

/// Stateful ANSI diff painter for kernel cell grids.
pub struct Painter {
    truecolor: bool,
    prev: Option<cells::CellGrid>,
    buf: String,
    cur_fg: u32,
    cur_bg: u32,
    cur_pos: Option<(i32, i32)>, // (row, col) the terminal cursor sits at
    cur_strike: bool,
}

impl Painter {
    /// Detects terminal truecolor support and creates an empty painter.
    pub fn new() -> Self {
        let truecolor = std::env::var("COLORTERM")
            .map(|v| v.contains("truecolor") || v.contains("24bit"))
            .unwrap_or(false);
        Self::with_truecolor(truecolor)
    }

    /// Creates an empty painter with explicit color capability.
    pub fn with_truecolor(truecolor: bool) -> Self {
        Painter {
            truecolor,
            prev: None,
            buf: String::new(),
            cur_fg: cells::NO_COLOR,
            cur_bg: cells::NO_COLOR,
            cur_pos: None,
            cur_strike: false,
        }
    }

    fn sgr(&mut self, fg: u32, bg: u32, strike: bool) {
        if fg == self.cur_fg && bg == self.cur_bg && strike == self.cur_strike {
            return;
        }
        self.buf.push_str("\x1b[0");
        if strike {
            self.buf.push_str(";9");
        }
        for (base, c) in [(38u8, fg), (48u8, bg)] {
            if c == cells::NO_COLOR {
                continue;
            }
            if self.truecolor {
                self.buf.push_str(&format!(
                    ";{base};2;{};{};{}",
                    (c >> 16) & 0xFF,
                    (c >> 8) & 0xFF,
                    c & 0xFF
                ));
            } else {
                self.buf.push_str(&format!(";{base};5;{}", x256(c)));
            }
        }
        self.buf.push('m');
        self.cur_fg = fg;
        self.cur_bg = bg;
        self.cur_strike = strike;
    }

    /// Paints `grid` into the buffered terminal diff.
    pub fn paint(&mut self, grid: &cells::CellGrid, cols: u16, rows: u16, full: bool) {
        let g = grid;
        let full = full
            || match &self.prev {
                Some(p) => p.cols != g.cols || p.rows != g.rows,
                None => true,
            };
        self.buf.clear();
        if full {
            self.buf.push_str("\x1b[0m\x1b[2J");
            self.cur_fg = cells::NO_COLOR;
            self.cur_bg = cells::NO_COLOR;
            self.cur_strike = false;
        }
        self.cur_pos = None;
        let vis_rows = g.rows.min(i32::from(rows));
        let vis_cols = g.cols.min(i32::from(cols));
        for r in 0..vis_rows {
            for c in 0..vis_cols {
                let ix = (r * g.cols + c) as usize;
                let (fg, bg) = cell_colors(g, ix);
                let changed = full
                    || match &self.prev {
                        Some(p) => {
                            let here = cell_changed(p, g, ix);
                            let right = c + 1 < vis_cols
                                && (p.ch[ix + 1] == cells::CONT || g.ch[ix + 1] == cells::CONT)
                                && cell_changed(p, g, ix + 1);
                            here || right
                        }
                        None => true,
                    };
                if !changed {
                    continue;
                }
                if g.ch[ix] == cells::CONT {
                    continue;
                }
                if self.cur_pos != Some((r, c)) {
                    self.buf.push_str(&format!("\x1b[{};{}H", r + 1, c + 1));
                }
                self.sgr(fg, bg, g.flags[ix] & cells::CF_STRIKE != 0);
                if g.cl[ix].is_empty() {
                    self.buf.push(char::from_u32(g.ch[ix]).unwrap_or(' '));
                } else {
                    self.buf.push_str(&g.cl[ix]);
                }
                let advance = if c + 1 < vis_cols && g.ch[ix + 1] == cells::CONT {
                    2
                } else {
                    1
                };
                self.cur_pos = Some((r, c + advance));
            }
        }
        self.prev = Some(g.clone());
    }

    /// Appends one clipped and padded status line to the buffered diff.
    pub fn footer(&mut self, row: u16, cols: u16, text: &str) {
        let mut line: String = text.chars().take(usize::from(cols)).collect();
        let pad = usize::from(cols).saturating_sub(line.chars().count());
        line.extend(std::iter::repeat_n(' ', pad));
        self.buf
            .push_str(&format!("\x1b[{row};1H\x1b[0;2m{line}\x1b[0m"));
        self.cur_fg = cells::NO_COLOR;
        self.cur_bg = cells::NO_COLOR;
        self.cur_pos = None;
    }

    /// Returns the complete buffered terminal write.
    pub fn buffer(&self) -> &str {
        &self.buf
    }

    /// Marks the terminal cursor position as unknown after an overlay.
    pub fn invalidate_cursor(&mut self) {
        self.cur_pos = None;
    }
}

impl Default for Painter {
    fn default() -> Self {
        Self::new()
    }
}

fn mods_of(m: KeyModifiers) -> u32 {
    let mut out = 0;
    if m.contains(KeyModifiers::SHIFT) {
        out |= app::M_SHIFT;
    }
    if m.contains(KeyModifiers::ALT) {
        out |= app::M_ALT;
    }
    if m.contains(KeyModifiers::CONTROL) {
        out |= app::M_CTRL;
    }
    if m.intersects(KeyModifiers::SUPER | KeyModifiers::META) {
        out |= app::M_META;
    }
    out
}

fn accepts_printable_text(mods: u32) -> bool {
    mods & (app::M_CTRL | app::M_ALT | app::M_META) == 0
}

/// Mouse cell → slab units at the cell center.
fn mouse_xy(col: u16, row: u16) -> (f64, f64) {
    (
        f64::from(col) * cells::CW + cells::CW / 2.0,
        f64::from(row) * cells::CH + cells::CH / 2.0,
    )
}

/// Stateful terminal click counter and pointer delta tracker.
pub struct ClickTracker {
    last: Option<(Instant, u32, f64, f64, u32)>,
    cursor: Option<(f64, f64)>,
}

impl ClickTracker {
    /// Creates an empty click and pointer history.
    pub fn new() -> Self {
        Self {
            last: None,
            cursor: None,
        }
    }

    /// Records a press and returns its consecutive click count.
    pub fn pointer_down(&mut self, button: u32, x: f64, y: f64) -> u32 {
        let now = Instant::now();
        let clicks = self
            .last
            .map_or(1, |(last, last_button, last_x, last_y, count)| {
                let dx = x - last_x;
                let dy = y - last_y;
                if button == last_button
                    && now.duration_since(last) <= Duration::from_millis(500)
                    && dx * dx + dy * dy <= 16.0
                {
                    count.saturating_add(1)
                } else {
                    1
                }
            });
        self.last = Some((now, button, x, y, clicks));
        clicks
    }

    /// Records a pointer position and returns its event-local delta.
    pub fn move_to(&mut self, x: f64, y: f64) -> (f64, f64) {
        let delta = self
            .cursor
            .map_or((0.0, 0.0), |(last_x, last_y)| (x - last_x, y - last_y));
        self.cursor = Some((x, y));
        delta
    }
}

impl Default for ClickTracker {
    fn default() -> Self {
        Self::new()
    }
}

fn mouse_button_code(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

/// Result of translating one crossterm input event.
pub enum Translated {
    /// No kernel input corresponds to this event.
    Ignored,
    /// Dispatches one event and an optional following text event.
    Events(dispatch::Event, Option<dispatch::Event>),
    /// Resizes the terminal viewport in cells.
    Resize(u16, u16),
    /// Requests a clean host shutdown.
    Quit,
}

/// Stateful crossterm-to-kernel event translator.
#[derive(Default)]
pub struct Translator {
    clicks: ClickTracker,
}

impl Translator {
    /// Creates a translator with empty click and pointer history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Translates one crossterm event without dispatching it.
    pub fn translate(&mut self, event: Event) -> Translated {
        match event {
            Event::Key(KeyEvent {
                code,
                modifiers,
                kind,
                ..
            }) => {
                if kind == KeyEventKind::Release {
                    return Translated::Ignored;
                }
                let mods = mods_of(modifiers);
                let key = match code {
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Translated::Quit;
                    }
                    KeyCode::Tab => app::key_event("Tab", mods),
                    KeyCode::BackTab => app::key_event("Tab", mods | app::M_SHIFT),
                    KeyCode::Enter => app::key_event("Enter", mods),
                    KeyCode::Backspace => app::key_event("Backspace", mods),
                    KeyCode::Delete => app::key_event("Delete", mods),
                    KeyCode::Esc => app::key_event("Escape", mods),
                    KeyCode::Insert => app::key_event("Insert", mods),
                    KeyCode::Home => app::key_event("Home", mods),
                    KeyCode::End => app::key_event("End", mods),
                    KeyCode::PageUp => app::key_event("PageUp", mods),
                    KeyCode::PageDown => app::key_event("PageDown", mods),
                    KeyCode::Left => app::key_event("ArrowLeft", mods),
                    KeyCode::Right => app::key_event("ArrowRight", mods),
                    KeyCode::Up => app::key_event("ArrowUp", mods),
                    KeyCode::Down => app::key_event("ArrowDown", mods),
                    KeyCode::F(number) if number <= 24 => {
                        app::key_event(&format!("F{number}"), mods)
                    }
                    KeyCode::Char(ch) => {
                        let text =
                            accepts_printable_text(mods).then(|| app::text_event(&ch.to_string()));
                        return Translated::Events(app::key_event(&ch.to_string(), mods), text);
                    }
                    _ => return Translated::Ignored,
                };
                Translated::Events(key, None)
            }
            Event::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers,
            }) => {
                let (x, y) = mouse_xy(column, row);
                let (pointer_dx, pointer_dy) = self.clicks.move_to(x, y);
                let mut event = match kind {
                    MouseEventKind::Down(button) => {
                        let button = mouse_button_code(button);
                        let count = self.clicks.pointer_down(button, x, y);
                        app::pointer_button_event(app::E_POINTER_DOWN, x, y, button, count)
                    }
                    MouseEventKind::Up(button) => app::pointer_button_event(
                        app::E_POINTER_UP,
                        x,
                        y,
                        mouse_button_code(button),
                        0,
                    ),
                    MouseEventKind::Drag(button) => app::pointer_button_event(
                        app::E_POINTER_MOVE,
                        x,
                        y,
                        mouse_button_code(button),
                        0,
                    ),
                    MouseEventKind::Moved => app::pointer_event(app::E_POINTER_MOVE, x, y),
                    MouseEventKind::ScrollDown => app::wheel_event(x, y, 3.0 * cells::CH),
                    MouseEventKind::ScrollUp => app::wheel_event(x, y, -3.0 * cells::CH),
                    _ => return Translated::Ignored,
                };
                event.mods = mods_of(modifiers);
                if matches!(event.etype, app::E_POINTER_MOVE | app::E_POINTER_UP) {
                    event.dx = pointer_dx;
                    event.dy = pointer_dy;
                }
                Translated::Events(event, None)
            }
            Event::Paste(text) => Translated::Events(app::paste_event(&text), None),
            Event::Resize(cols, rows) => Translated::Resize(cols, rows),
            _ => Translated::Ignored,
        }
    }
}

/// Translates one crossterm event with caller-owned input history.
pub fn translate(translator: &mut Translator, event: Event) -> Translated {
    translator.translate(event)
}

/// `--examples` gallery: the document list plus the entry on screen.
/// Ctrl-N/Ctrl-P step through it; the bottom row shows the position.
pub struct Gallery<'a> {
    /// Ordered document paths.
    pub files: &'a [PathBuf],
    /// Current document index.
    pub index: usize,
}

impl Gallery<'_> {
    /// Wrapping neighbours in load order.
    fn next(&self) -> usize {
        (self.index + 1) % self.files.len()
    }

    fn prev(&self) -> usize {
        (self.index + self.files.len() - 1) % self.files.len()
    }

    /// Bottom-row hint: position, document name, bindings.
    fn footer(&self) -> String {
        let name = self.files[self.index]
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        format!(
            "[{}/{}] {name}   ^N next   ^P prev   ^C quit",
            self.index + 1,
            self.files.len()
        )
    }
}

/// Why the document loop stopped.
pub enum Exit {
    /// Ends the terminal session.
    Quit,
    /// Load the gallery entry at this index instead.
    Switch(usize),
}

enum Handled {
    Quit,
    Switch(usize),
    Resized(u16, u16),
    Continue,
}

fn handle(
    inst: &mut kframe::Instance,
    event: Event,
    signals: &mut Vec<app::Signal>,
    translator: &mut Translator,
    gallery: Option<&Gallery>,
) -> Handled {
    if let Event::Key(KeyEvent {
        code: KeyCode::Char(c @ ('n' | 'p')),
        modifiers,
        kind,
        ..
    }) = &event
        && *kind != KeyEventKind::Release
        && modifiers.contains(KeyModifiers::CONTROL)
        && let Some(gallery) = gallery
    {
        let next = if *c == 'n' {
            gallery.next()
        } else {
            gallery.prev()
        };
        app::close_instance(inst, signals);
        return Handled::Switch(next);
    }

    match translate(translator, event) {
        Translated::Ignored => Handled::Continue,
        Translated::Quit => {
            app::close_instance(inst, signals);
            Handled::Quit
        }
        Translated::Resize(cols, rows) => Handled::Resized(cols, rows),
        Translated::Events(first, second) => {
            for event in std::iter::once(first).chain(second) {
                let effects = kframe::inst_dispatch(inst, &event);
                app::collect_signals(inst, &effects, signals);
            }
            Handled::Continue
        }
    }
}

fn forward_host_signals(
    inst: &mut kframe::Instance,
    host: &mut dyn app::Host,
    signals: &[app::Signal],
    seen_signals: &mut usize,
) -> Result<(), String> {
    while *seen_signals < signals.len() {
        host.on_signal(inst, &signals[*seen_signals])?;
        *seen_signals += 1;
    }
    Ok(())
}

/// Session settings for a document run.
pub struct Ui<'a> {
    /// Maximum repaint frequency.
    pub fps: f64,
    /// Shows the latest signal in a reserved footer row.
    pub debug: bool,
    /// Enables the dark environment condition.
    pub dark: bool,
    /// Enables the coarse-pointer environment condition.
    pub coarse: bool,
    /// Enables gallery navigation and its reserved footer row.
    pub gallery: Option<Gallery<'a>>,
}

impl Terminal {
    /// Enter raw mode + alt screen with mouse capture, bracketed paste and —
    /// where the terminal speaks it — the kitty keyboard protocol, which
    /// disambiguates Shift+Enter (multiline newline) from Enter. The probe is
    /// skipped under tmux, where it is known to misreport.
    pub fn new() -> Result<Terminal, String> {
        terminal::enable_raw_mode().map_err(ioerr)?;
        let kitty = std::env::var_os("TMUX").is_none()
            && terminal::supports_keyboard_enhancement().unwrap_or(false);
        // Built before the fallible setup below so Drop still restores.
        let term = Terminal { kitty };
        let mut out = std::io::stdout();
        execute!(
            out,
            terminal::EnterAlternateScreen,
            cursor::Hide,
            event::EnableMouseCapture,
            event::EnableBracketedPaste
        )
        .map_err(ioerr)?;
        if kitty {
            execute!(
                out,
                event::PushKeyboardEnhancementFlags(
                    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                )
            )
            .map_err(ioerr)?;
        }
        Ok(term)
    }

    /// Drives one document and invokes `host` between kernel frames.
    pub fn run(
        &self,
        inst: &mut kframe::Instance,
        host: &mut dyn app::Host,
        mut images: crate::images::Images,
        ui: &Ui,
    ) -> Result<Exit, String> {
        let mut out = std::io::stdout();
        let (mut cols, mut rows) = terminal::size().map_err(ioerr)?;
        // Bottom rows are host chrome: gallery hint last, signals above it.
        let gallery_rows = u16::from(ui.gallery.is_some());
        let footer_rows = u16::from(ui.debug) + gallery_rows;
        let mut doc_rows = resize(inst, cols, rows, footer_rows, ui.dark, ui.coarse);

        let start = Instant::now();
        let frame_dt = Duration::from_secs_f64(1.0 / ui.fps.max(1.0));
        let mut painter = Painter::new();
        let mut full = true;
        let mut last_frame = Instant::now();
        let mut signals: Vec<app::Signal> = Vec::new();
        let mut footer_shown = String::new();
        let mut host_tick = Instant::now();
        let mut seen_signals = 0usize;
        let mut translator = Translator::new();

        loop {
            // Forward new signals before the host clock advances. Host writes
            // mark the instance dirty and repaint through the same kernel path.
            forward_host_signals(inst, host, &signals, &mut seen_signals)?;
            host.tick(inst, host_tick.elapsed().as_secs_f64() * 1000.0)?;
            host_tick = Instant::now();
            if inst.dirty || !inst.solved || inst.ms.active || full {
                let t = start.elapsed().as_secs_f64() * 1000.0;
                let fr = kframe::inst_frame(inst, t);
                app::drain_frame_signals(inst, &mut signals);
                let grid = cells::cells_with_caret(inst, &fr);
                painter.paint(&grid, cols, doc_rows, full);
                // Rows are 1-based; a degenerate 0-row report must not
                // address row 0.
                let bottom = rows.max(1);
                if let Some(g) = &ui.gallery
                    && full
                {
                    painter.footer(bottom, cols, &g.footer());
                }
                if ui.debug {
                    let badges = host.badges();
                    let text = match signals.last() {
                        Some(signal) => format!("sig: {}{badges}", app::format_signal(signal)),
                        None => format!("sig: —{badges}"),
                    };
                    if full || text != footer_shown {
                        painter.footer(bottom.saturating_sub(gallery_rows).max(1), cols, &text);
                        footer_shown = text;
                    }
                }
                // Kitty-graphics placements ride the same write: they must land
                // after the cell grid so real images cover the placeholder.
                let mut overlay = String::new();
                if images.paint(&mut overlay, &fr, full) {
                    painter.invalidate_cursor();
                }
                out.write_all(painter.buffer().as_bytes()).map_err(ioerr)?;
                out.write_all(overlay.as_bytes()).map_err(ioerr)?;
                out.flush().map_err(ioerr)?;
                full = false;
                last_frame = Instant::now();
            }
            let animating = inst.ms.active || inst.dirty;
            let timeout = if animating {
                frame_dt.saturating_sub(last_frame.elapsed())
            } else {
                Duration::from_millis(300)
            };
            if event::poll(timeout).map_err(ioerr)? {
                loop {
                    match handle(
                        inst,
                        event::read().map_err(ioerr)?,
                        &mut signals,
                        &mut translator,
                        ui.gallery.as_ref(),
                    ) {
                        Handled::Quit => {
                            forward_host_signals(inst, host, &signals, &mut seen_signals)?;
                            return Ok(Exit::Quit);
                        }
                        Handled::Switch(next) => {
                            forward_host_signals(inst, host, &signals, &mut seen_signals)?;
                            // The next document reuses image ids 1..n for
                            // different pictures: drop this one's placements.
                            let mut buf = String::new();
                            images.clear(&mut buf);
                            out.write_all(buf.as_bytes()).map_err(ioerr)?;
                            out.flush().map_err(ioerr)?;
                            return Ok(Exit::Switch(next));
                        }
                        Handled::Resized(c, r) => {
                            cols = c;
                            rows = r;
                            doc_rows = resize(inst, cols, rows, footer_rows, ui.dark, ui.coarse);
                            full = true;
                        }
                        Handled::Continue => {}
                    }
                    if !event::poll(Duration::ZERO).map_err(ioerr)? {
                        break;
                    }
                }
            }
        }
    }
}

/// Enters a terminal session and drives one document with host callbacks.
pub fn run(
    inst: &mut kframe::Instance,
    host: &mut dyn app::Host,
    images: crate::images::Images,
    ui: &Ui,
) -> Result<Exit, String> {
    Terminal::new()?.run(inst, host, images, ui)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_terminal_modifiers_map_to_kernel_meta() {
        assert_eq!(mods_of(KeyModifiers::SUPER), app::M_META);
        assert_eq!(mods_of(KeyModifiers::META), app::M_META);
        assert_eq!(
            mods_of(
                KeyModifiers::SHIFT
                    | KeyModifiers::ALT
                    | KeyModifiers::CONTROL
                    | KeyModifiers::SUPER
            ),
            app::M_SHIFT | app::M_ALT | app::M_CTRL | app::M_META
        );
    }

    #[test]
    fn command_modifiers_do_not_emit_printable_text() {
        assert!(accepts_printable_text(0));
        assert!(accepts_printable_text(app::M_SHIFT));
        assert!(!accepts_printable_text(app::M_CTRL));
        assert!(!accepts_printable_text(app::M_ALT));
        assert!(!accepts_printable_text(app::M_META));
    }
}
