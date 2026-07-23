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
use slab_kernel::{cells, frame as kframe};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn ioerr(e: std::io::Error) -> String {
    format!("terminal: {e}")
}

/// One terminal cell covers CW×CH slab units (`slab_kernel::cells` quantization).
fn env_for(cols: u16, doc_rows: u16) -> (f64, f64) {
    (f64::from(cols) * cells::CW, f64::from(doc_rows) * cells::CH)
}

/// Live terminal session: raw mode + alt screen, restored on scope exit.
/// Built by [`Term::new`], drives documents through [`Term::run`]; `kitty`
/// mirrors whether enhancement flags were pushed (pop must precede
/// LeaveAlternateScreen).
pub struct Term {
    kitty: bool,
}

impl Drop for Term {
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
    old.ch[ix] != new.ch[ix] || old.cl[ix] != new.cl[ix] || old_fg != new_fg || old_bg != new_bg
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

/// Diff-paints cell grids: cursor moves + SGR runs, full clear+repaint on
/// resize or first frame. Truecolor unless COLORTERM lacks 24-bit.
struct Painter {
    truecolor: bool,
    prev: Option<cells::CellGrid>,
    buf: String,
    cur_fg: u32,
    cur_bg: u32,
    cur_pos: Option<(i32, i32)>, // (row, col) the terminal cursor sits at
}

impl Painter {
    fn new() -> Self {
        let truecolor = std::env::var("COLORTERM")
            .map(|v| v.contains("truecolor") || v.contains("24bit"))
            .unwrap_or(false);
        Painter {
            truecolor,
            prev: None,
            buf: String::new(),
            cur_fg: cells::NO_COLOR,
            cur_bg: cells::NO_COLOR,
            cur_pos: None,
        }
    }

    fn sgr(&mut self, fg: u32, bg: u32) {
        if fg == self.cur_fg && bg == self.cur_bg {
            return;
        }
        self.buf.push_str("\x1b[0");
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
    }

    /// Paint `g` clipped to a cols×rows terminal window.
    fn paint(&mut self, g: &cells::CellGrid, cols: u16, rows: u16, full: bool) {
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
                self.sgr(fg, bg);
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

    /// Paint one dim status line across 1-based `row`, clipped and padded
    /// to `cols`. Buffered like a frame; the caller flushes.
    fn footer(&mut self, row: u16, cols: u16, text: &str) {
        let mut line: String = text.chars().take(usize::from(cols)).collect();
        let pad = usize::from(cols).saturating_sub(line.chars().count());
        line.extend(std::iter::repeat_n(' ', pad));
        self.buf
            .push_str(&format!("\x1b[{row};1H\x1b[0;2m{line}\x1b[0m"));
        self.cur_fg = cells::NO_COLOR;
        self.cur_bg = cells::NO_COLOR;
        self.cur_pos = None;
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

struct ClickTracker {
    last: Option<(Instant, u32, f64, f64, u32)>,
    cursor: Option<(f64, f64)>,
}

impl ClickTracker {
    fn new() -> Self {
        Self {
            last: None,
            cursor: None,
        }
    }

    fn pointer_down(&mut self, button: u32, x: f64, y: f64) -> u32 {
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

    fn move_to(&mut self, x: f64, y: f64) -> (f64, f64) {
        let delta = self
            .cursor
            .map_or((0.0, 0.0), |(last_x, last_y)| (x - last_x, y - last_y));
        self.cursor = Some((x, y));
        delta
    }
}

fn mouse_button_code(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

/// `--examples` gallery: the document list plus the entry on screen.
/// Ctrl-N/Ctrl-P step through it; the bottom row shows the position.
pub struct Gallery<'a> {
    pub files: &'a [PathBuf],
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
    ev: Event,
    signals: &mut Vec<app::Signal>,
    clicks: &mut ClickTracker,
    gallery: Option<&Gallery>,
) -> Handled {
    match ev {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind,
            ..
        }) => {
            if kind == KeyEventKind::Release {
                return Handled::Continue;
            }
            let mods = mods_of(modifiers);
            let kev = |k: &str| Some(app::key_event(k, mods));
            let dispatch = match code {
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                    app::close_instance(inst, signals);
                    return Handled::Quit;
                }
                // Gallery navigation: Ctrl-N/Ctrl-P are unbound in the
                // kernel, so intercepting them costs the document nothing.
                KeyCode::Char(c @ ('n' | 'p')) if modifiers.contains(KeyModifiers::CONTROL) => {
                    let Some(gallery) = gallery else {
                        return Handled::Continue;
                    };
                    let next = if c == 'n' {
                        gallery.next()
                    } else {
                        gallery.prev()
                    };
                    app::close_instance(inst, signals);
                    return Handled::Switch(next);
                }
                KeyCode::Tab => kev("Tab"),
                KeyCode::BackTab => Some(app::key_event("Tab", mods | app::M_SHIFT)),
                KeyCode::Enter => kev("Enter"),
                KeyCode::Backspace => kev("Backspace"),
                KeyCode::Delete => kev("Delete"),
                KeyCode::Esc => kev("Escape"),
                KeyCode::Insert => kev("Insert"),
                KeyCode::Home => kev("Home"),
                KeyCode::End => kev("End"),
                KeyCode::PageUp => kev("PageUp"),
                KeyCode::PageDown => kev("PageDown"),
                KeyCode::Left => kev("ArrowLeft"),
                KeyCode::Right => kev("ArrowRight"),
                KeyCode::Up => kev("ArrowUp"),
                KeyCode::Down => kev("ArrowDown"),
                KeyCode::F(number) if number <= 24 => {
                    Some(app::key_event(&format!("F{number}"), mods))
                }
                KeyCode::Char(ch) => {
                    // key-down first (Space/Enter button semantics, ctrl-A
                    // select-all), then the text insert for plain chars.
                    let s = ch.to_string();
                    let eff = kframe::inst_dispatch(inst, &app::key_event(&s, mods));
                    app::collect_signals(inst, &eff, signals);
                    if accepts_printable_text(mods) {
                        Some(app::text_event(&s))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(ev) = dispatch {
                let eff = kframe::inst_dispatch(inst, &ev);
                app::collect_signals(inst, &eff, signals);
            }
            Handled::Continue
        }
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers,
        }) => {
            let (x, y) = mouse_xy(column, row);
            let (pointer_dx, pointer_dy) = clicks.move_to(x, y);
            let mods = mods_of(modifiers);
            let ev = match kind {
                MouseEventKind::Down(button) => {
                    let button = mouse_button_code(button);
                    let count = clicks.pointer_down(button, x, y);
                    Some(app::pointer_button_event(
                        app::E_POINTER_DOWN,
                        x,
                        y,
                        button,
                        count,
                    ))
                }
                MouseEventKind::Up(button) => Some(app::pointer_button_event(
                    app::E_POINTER_UP,
                    x,
                    y,
                    mouse_button_code(button),
                    0,
                )),
                MouseEventKind::Drag(button) => Some(app::pointer_button_event(
                    app::E_POINTER_MOVE,
                    x,
                    y,
                    mouse_button_code(button),
                    0,
                )),
                MouseEventKind::Moved => Some(app::pointer_event(app::E_POINTER_MOVE, x, y)),
                MouseEventKind::ScrollDown => Some(app::wheel_event(x, y, 3.0 * cells::CH)),
                MouseEventKind::ScrollUp => Some(app::wheel_event(x, y, -3.0 * cells::CH)),
                _ => None,
            };
            if let Some(mut ev) = ev {
                ev.mods = mods;
                if matches!(ev.etype, app::E_POINTER_MOVE | app::E_POINTER_UP) {
                    ev.dx = pointer_dx;
                    ev.dy = pointer_dy;
                }
                let eff = kframe::inst_dispatch(inst, &ev);
                app::collect_signals(inst, &eff, signals);
            }
            Handled::Continue
        }
        Event::Paste(s) => {
            let eff = kframe::inst_dispatch(inst, &app::paste_event(&s));
            app::collect_signals(inst, &eff, signals);
            Handled::Continue
        }
        Event::Resize(c, r) => Handled::Resized(c, r),
        _ => Handled::Continue,
    }
}

fn forward_app_signals(
    inst: &mut kframe::Instance,
    papp: &mut Option<&mut crate::player::PlayerApp>,
    signals: &[app::Signal],
    seen_signals: &mut usize,
) -> Result<(), String> {
    if let Some(player) = papp.as_deref_mut() {
        while *seen_signals < signals.len() {
            player.on_signal(inst, &signals[*seen_signals].name)?;
            *seen_signals += 1;
        }
    }
    Ok(())
}

/// Session knobs shared by every document a `Term` drives.
pub struct Ui<'a> {
    pub fps: f64,
    pub debug: bool,
    pub dark: bool,
    pub coarse: bool,
    /// `Some` in `--examples` mode: enables the gallery row and Ctrl-N/P.
    pub gallery: Option<Gallery<'a>>,
}

impl Term {
    /// Enter raw mode + alt screen with mouse capture, bracketed paste and —
    /// where the terminal speaks it — the kitty keyboard protocol, which
    /// disambiguates Shift+Enter (multiline newline) from Enter. The probe is
    /// skipped under tmux, where it is known to misreport.
    pub fn new() -> Result<Term, String> {
        terminal::enable_raw_mode().map_err(ioerr)?;
        let kitty = std::env::var_os("TMUX").is_none()
            && terminal::supports_keyboard_enhancement().unwrap_or(false);
        // Built before the fallible setup below so Drop still restores.
        let term = Term { kitty };
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

    /// Drive one document until the user quits or picks another gallery entry.
    pub fn run(
        &self,
        inst: &mut kframe::Instance,
        mut papp: Option<&mut crate::player::PlayerApp>,
        mut images: crate::images::Images,
        ui: &Ui,
    ) -> Result<Exit, String> {
        let mut out = std::io::stdout();
        let (mut cols, mut rows) = terminal::size().map_err(ioerr)?;
        // Bottom rows are host chrome: gallery hint last, signals above it.
        let gallery_rows = u16::from(ui.gallery.is_some());
        let footer_rows = u16::from(ui.debug) + gallery_rows;
        let mut doc_rows = rows.saturating_sub(footer_rows).max(1);
        let (vw, vh) = env_for(cols, doc_rows);
        kframe::inst_set_env(inst, vw, vh, 2, ui.dark, ui.coarse);

        let start = Instant::now();
        let frame_dt = Duration::from_secs_f64(1.0 / ui.fps.max(1.0));
        let mut painter = Painter::new();
        let mut full = true;
        let mut last_frame = Instant::now();
        let mut signals: Vec<app::Signal> = Vec::new();
        let mut footer_shown = String::new();
        let mut app_tick = Instant::now();
        let mut seen_signals = 0usize;
        let mut clicks = ClickTracker::new();

        loop {
            // App layer first: forward freshly emitted signals, then advance
            // the play clock; param writes mark the instance dirty, so the
            // ordinary dirty/animating check below repaints through the
            // kernel (times, progress knob, |>/|| swap, auto-advance).
            forward_app_signals(inst, &mut papp, &signals, &mut seen_signals)?;
            if let Some(player) = papp.as_deref_mut() {
                player.advance(inst, app_tick.elapsed().as_secs_f64() * 1000.0)?;
            }
            app_tick = Instant::now();
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
                    let badges = papp.as_deref().map(|p| p.badges()).unwrap_or_default();
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
                    painter.cur_pos = None;
                }
                out.write_all(painter.buf.as_bytes()).map_err(ioerr)?;
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
                        &mut clicks,
                        ui.gallery.as_ref(),
                    ) {
                        Handled::Quit => {
                            forward_app_signals(inst, &mut papp, &signals, &mut seen_signals)?;
                            return Ok(Exit::Quit);
                        }
                        Handled::Switch(next) => {
                            forward_app_signals(inst, &mut papp, &signals, &mut seen_signals)?;
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
                            doc_rows = rows.saturating_sub(footer_rows).max(1);
                            let (vw, vh) = env_for(cols, doc_rows);
                            kframe::inst_set_env(inst, vw, vh, 2, ui.dark, ui.coarse);
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
