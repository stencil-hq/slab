"""Interactive terminal driver for a Slab session.

The driver mirrors `crates/slab-tui/src/interactive.rs`: it puts the terminal
into raw mode on the alternate screen, enables SGR mouse reporting (mode 1006)
and bracketed paste, decodes escape sequences into Slab input, paints the cell
grid the kernel returns, and advances the virtual motion clock at a fixed frame
rate. Ctrl+C quits and is never forwarded to the document.

The kernel owns layout, hit testing, focus, scrolling, and editing. This module
only translates terminal bytes into SDP input and paints the returned cells.

:class:`Decoder` is the escape-sequence decoder on its own, with no terminal
attached, so it can be exercised directly::

    >>> from slab.tui import Decoder, Key
    >>> Decoder().feed(b"\\x1b[1;5A")
    [Key(key='ArrowUp', mods=('ctrl',))]
"""

from __future__ import annotations

import os
import select
import signal as signalmod
import sys
import termios
import time
import tty
from dataclasses import dataclass, replace
from typing import Callable, Sequence

from . import CELL_HEIGHT, CELL_WIDTH, Session, Signal

__all__ = [
    "ClickTracker",
    "Decoder",
    "Key",
    "Paste",
    "Pointer",
    "Quit",
    "Terminal",
    "TerminalEvent",
    "Text",
    "Wheel",
    "WHEEL_STEP",
    "DEFAULT_SIZE",
    "paint",
    "pointer_units",
    "run",
]

#: Layout units one wheel notch scrolls, matching the Rust terminal driver.
WHEEL_STEP = 3.0 * CELL_HEIGHT

#: Default frame rate for clock ticks and repaints.
DEFAULT_FPS = 30.0

#: Grid used when the terminal reports no usable window size.
DEFAULT_SIZE = (80, 24)


@dataclass(frozen=True, slots=True)
class Key:
    """A key press with its held modifiers.

    Attributes:
        key: Key name such as `Enter`, `ArrowLeft`, `F7`, or a literal
            printable character.
        mods: Ordered subset of `shift`, `alt`, `ctrl`, `meta`.
    """

    key: str
    mods: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class Text:
    """Printable text that follows a key press with no ctrl, alt, or meta."""

    text: str


@dataclass(frozen=True, slots=True)
class Paste:
    """A bracketed-paste payload."""

    text: str


@dataclass(frozen=True, slots=True)
class Pointer:
    """A mouse move, press, or release, in terminal cells.

    Attributes:
        kind: `move`, `down`, or `up`.
        col: Zero-based column.
        row: Zero-based row.
        button: `0` left, `1` middle, `2` right.
        mods: Modifiers held during the report.
        clicks: Consecutive click count for a `down`, counted by the decoder
            like the Rust driver; the kernel ignores it on `move` and `up`.
    """

    kind: str
    col: int
    row: int
    button: int = 0
    mods: tuple[str, ...] = ()
    clicks: int = 1


@dataclass(frozen=True, slots=True)
class Wheel:
    """A wheel report; `notches` is positive downward.

    Attributes:
        col: Zero-based column.
        row: Zero-based row.
        notches: Signed notch count, negative for scroll-up.
        mods: Modifiers held during the report.
    """

    col: int
    row: int
    notches: int
    mods: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class Quit:
    """Ctrl+C: the host shuts down and the document never sees the key."""


#: Anything :meth:`Decoder.feed` can return.
TerminalEvent = Key | Text | Paste | Pointer | Wheel | Quit


def pointer_units(col: int, row: int) -> tuple[float, float]:
    """Converts a terminal cell to layout units at the centre of the cell."""
    return (col * CELL_WIDTH + CELL_WIDTH / 2.0, row * CELL_HEIGHT + CELL_HEIGHT / 2.0)


def _xterm_mods(param: int) -> tuple[str, ...]:
    """Decodes an xterm modifier parameter, where `1` means no modifier."""
    bits = max(param - 1, 0)
    out = []
    if bits & 1:
        out.append("shift")
    if bits & 2:
        out.append("alt")
    if bits & 4:
        out.append("ctrl")
    if bits & 8:
        out.append("meta")
    return tuple(out)


def _mouse_mods(button_code: int) -> tuple[str, ...]:
    """Decodes the modifier bits carried in an SGR mouse button code."""
    out = []
    if button_code & 4:
        out.append("shift")
    if button_code & 8:
        out.append("alt")
    if button_code & 16:
        out.append("ctrl")
    return tuple(out)


#: Final CSI bytes that name a key on their own.
_CSI_FINAL: dict[str, str] = {
    "A": "ArrowUp",
    "B": "ArrowDown",
    "C": "ArrowRight",
    "D": "ArrowLeft",
    "E": "Clear",
    "F": "End",
    "H": "Home",
    "P": "F1",
    "Q": "F2",
    "R": "F3",
    "S": "F4",
}

#: `CSI <n> ~` keypad and function codes.
_CSI_TILDE: dict[int, str] = {
    1: "Home",
    2: "Insert",
    3: "Delete",
    4: "End",
    5: "PageUp",
    6: "PageDown",
    7: "Home",
    8: "End",
    11: "F1",
    12: "F2",
    13: "F3",
    14: "F4",
    15: "F5",
    17: "F6",
    18: "F7",
    19: "F8",
    20: "F9",
    21: "F10",
    23: "F11",
    24: "F12",
    25: "F13",
    26: "F14",
    28: "F15",
    29: "F16",
    31: "F17",
    32: "F18",
    33: "F19",
    34: "F20",
}

#: Control bytes that map straight to a key name.
_CONTROL_KEYS: dict[int, str] = {
    0x08: "Backspace",
    0x09: "Tab",
    0x0A: "Enter",
    0x0D: "Enter",
    0x7F: "Backspace",
}

_PASTE_START = b"\x1b[200~"
_PASTE_END = b"\x1b[201~"


class ClickTracker:
    """Stateful consecutive-click counter, matching the Rust terminal driver.

    A press repeats the previous click when it uses the same button, lands
    within :data:`CLICK_RADIUS` layout units of it, and arrives within
    :data:`CLICK_INTERVAL` seconds; anything else starts a fresh single click.
    """

    #: Seconds within which a press can extend the previous click.
    CLICK_INTERVAL = 0.5

    #: Distance in layout units within which a press extends the click.
    CLICK_RADIUS = 4.0

    __slots__ = ("_last",)

    def __init__(self) -> None:
        """Creates a tracker with no click history."""
        self._last: tuple[float, int, float, float, int] | None = None

    def pointer_down(self, button: int, x: float, y: float, now: float | None = None) -> int:
        """Records a press at layout units `(x, y)` and returns its click count.

        Args:
            button: `0` left, `1` middle, `2` right.
            x: Horizontal position in layout units.
            y: Vertical position in layout units.
            now: Monotonic timestamp in seconds; defaults to the current one.
        """
        stamp = time.monotonic() if now is None else now
        clicks = 1
        if self._last is not None:
            last_stamp, last_button, last_x, last_y, count = self._last
            dx = x - last_x
            dy = y - last_y
            if (
                button == last_button
                and stamp - last_stamp <= self.CLICK_INTERVAL
                and dx * dx + dy * dy <= self.CLICK_RADIUS * self.CLICK_RADIUS
            ):
                clicks = count + 1
        self._last = (stamp, button, x, y, clicks)
        return clicks



class Decoder:
    """Incremental decoder from terminal bytes to :data:`TerminalEvent` values.

    Feed whatever a read returns; the decoder keeps a partial escape sequence
    or a partial UTF-8 character in its buffer until the rest arrives. A lone
    `ESC` is ambiguous until the next byte, so it stays buffered; call
    :meth:`flush` after an input timeout to turn it into an `Escape` key.
    """

    __slots__ = ("_buffer", "_clicks", "_paste")

    def __init__(self) -> None:
        """Creates a decoder with an empty buffer and click history."""
        self._buffer = bytearray()
        self._paste: bytearray | None = None
        self._clicks = ClickTracker()

    def feed(self, data: bytes) -> list[TerminalEvent]:
        """Appends `data` and returns every event that is now complete."""
        self._buffer.extend(data)
        events: list[TerminalEvent] = []
        while self._buffer:
            consumed, produced = self._step()
            if consumed == 0:
                break
            del self._buffer[:consumed]
            events.extend(produced)
        return events

    def flush(self) -> list[TerminalEvent]:
        """Resolves a buffered lone `ESC` into an `Escape` key press.

        Call this when a read timed out, which proves no escape sequence is
        still on its way. Any other buffered bytes are left alone.
        """
        if self._paste is None and len(self._buffer) == 1 and self._buffer[0] == 0x1B:
            self._buffer.clear()
            return [Key("Escape")]
        return []

    @property
    def pending(self) -> bytes:
        """Bytes held back because they do not yet form a complete event."""
        return bytes(self._buffer)

    def _step(self) -> tuple[int, list[TerminalEvent]]:
        """Decodes one event; returns bytes consumed and events produced."""
        if self._paste is not None:
            return self._step_paste()
        first = self._buffer[0]
        if first == 0x1B:
            return self._step_escape()
        if first == 0x03:
            return (1, [Quit()])
        name = _CONTROL_KEYS.get(first)
        if name is not None:
            return (1, [Key(name)])
        if first < 0x20:
            return (1, [Key(chr(first + 0x60), ("ctrl",))])
        return self._step_utf8()

    def _step_paste(self) -> tuple[int, list[TerminalEvent]]:
        """Accumulates bracketed-paste bytes until the terminating sequence."""
        assert self._paste is not None
        end = bytes(self._buffer).find(_PASTE_END)
        if end < 0:
            keep = max(len(self._buffer) - len(_PASTE_END) + 1, 0)
            self._paste.extend(self._buffer[:keep])
            return (keep, []) if keep else (0, [])
        self._paste.extend(self._buffer[:end])
        text = self._paste.decode("utf-8", "replace")
        self._paste = None
        return (end + len(_PASTE_END), [Paste(text)])

    def _step_escape(self) -> tuple[int, list[TerminalEvent]]:
        """Decodes a sequence that starts with `ESC`."""
        buffer = bytes(self._buffer)
        if len(buffer) == 1:
            return (0, [])
        second = buffer[1]
        if second == 0x1B:
            return (1, [Key("Escape")])
        if second == ord("["):
            return self._step_csi(buffer)
        if second == ord("O"):
            if len(buffer) < 3:
                return (0, [])
            name = _CSI_FINAL.get(chr(buffer[2]))
            return (3, [Key(name)]) if name else (3, [])
        if second < 0x20:
            if second == 0x03:
                return (2, [Quit()])
            name = _CONTROL_KEYS.get(second)
            if name is not None:
                return (2, [Key(name, ("alt",))])
            return (2, [Key(chr(second + 0x60), ("alt", "ctrl"))])
        consumed, events = self._decode_utf8(buffer[1:])
        if consumed == 0:
            return (0, [])
        alt = [Key(event.key, ("alt",)) for event in events if isinstance(event, Key)]
        return (1 + consumed, alt)

    def _step_csi(self, buffer: bytes) -> tuple[int, list[TerminalEvent]]:
        """Decodes a `CSI` sequence, including SGR mouse and paste markers."""
        if buffer.startswith(_PASTE_START):
            self._paste = bytearray()
            return (len(_PASTE_START), [])
        index = 2
        while index < len(buffer) and (0x20 <= buffer[index] < 0x40):
            index += 1
        if index >= len(buffer):
            return (0, [])
        final = chr(buffer[index])
        body = buffer[2:index].decode("ascii", "replace")
        total = index + 1
        if body.startswith("<"):
            return (total, [self._count(event) for event in _sgr_mouse(body[1:], final)])
        params = [int(part) if part.isdigit() else 0 for part in body.split(";")] if body else []
        mods = _xterm_mods(params[1]) if len(params) > 1 else ()
        if final == "Z":
            return (total, [Key("Tab", ("shift",) + tuple(m for m in mods if m != "shift"))])
        if final == "~":
            name = _CSI_TILDE.get(params[0] if params else 0)
            return (total, [Key(name, mods)]) if name else (total, [])
        name = _CSI_FINAL.get(final)
        return (total, [Key(name, mods)]) if name else (total, [])

    def _count(self, event: TerminalEvent) -> TerminalEvent:
        """Stamps a pointer press with its consecutive click count."""
        if isinstance(event, Pointer) and event.kind == "down":
            x, y = pointer_units(event.col, event.row)
            return replace(event, clicks=self._clicks.pointer_down(event.button, x, y))
        return event

    def _step_utf8(self) -> tuple[int, list[TerminalEvent]]:
        """Decodes one printable character into a key plus its text."""
        return self._decode_utf8(bytes(self._buffer))

    @staticmethod
    def _decode_utf8(buffer: bytes) -> tuple[int, list[TerminalEvent]]:
        """Decodes the first character of `buffer` as UTF-8."""
        lead = buffer[0]
        if lead < 0x80:
            width = 1
        elif lead >= 0xF0:
            width = 4
        elif lead >= 0xE0:
            width = 3
        elif lead >= 0xC0:
            width = 2
        else:
            return (1, [])
        if len(buffer) < width:
            return (0, [])
        try:
            char = buffer[:width].decode("utf-8")
        except UnicodeDecodeError:
            return (1, [])
        return (width, [Key(char), Text(char)])


def _sgr_mouse(body: str, final: str) -> list[TerminalEvent]:
    """Decodes one SGR (mode 1006) mouse report body."""
    parts = body.split(";")
    if len(parts) != 3:
        return []
    try:
        code, column, row = (int(part) for part in parts)
    except ValueError:
        return []
    col = max(column - 1, 0)
    line = max(row - 1, 0)
    mods = _mouse_mods(code)
    if code & 64:
        return [Wheel(col, line, 1 if code & 3 else -1, mods)]
    button = code & 3
    if button == 3:
        return [Pointer("move", col, line, 0, mods)]
    if code & 32:
        return [Pointer("move", col, line, button, mods)]
    return [Pointer("down" if final == "M" else "up", col, line, button, mods)]


class Terminal:
    """Raw-mode alternate-screen terminal lifecycle.

    Entering the context switches the terminal into raw mode with the alternate
    screen, a hidden cursor, SGR mouse reporting, and bracketed paste. Leaving
    it restores every one of those settings, including after an exception.
    """

    #: Sequences written on entry, in order.
    ENTER = "\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h\x1b[?2004h\x1b[2J"

    #: Sequences written on exit, in reverse order of :data:`ENTER`.
    LEAVE = "\x1b[?2004l\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?25h\x1b[?1049l"

    __slots__ = ("_fd", "_saved", "_out")

    def __init__(self, fd: int | None = None, out: object | None = None) -> None:
        """Binds the lifecycle to a terminal file descriptor.

        Args:
            fd: Input descriptor; defaults to standard input.
            out: Text stream for output; defaults to standard output.
        """
        self._fd = sys.stdin.fileno() if fd is None else fd
        self._out = sys.stdout if out is None else out
        self._saved: list | None = None

    @property
    def fd(self) -> int:
        """Input file descriptor the terminal reads from."""
        return self._fd

    def size(self) -> tuple[int, int]:
        """Returns the terminal size as `(cols, rows)`, with a safe fallback.

        A terminal that reports no window size — an unsized pty, or a stream
        the kernel cannot measure — yields the conventional 80x24 grid instead
        of a degenerate one that paints nothing.
        """
        try:
            size = os.get_terminal_size(self._fd)
        except OSError:
            return DEFAULT_SIZE
        if size.columns <= 0 or size.lines <= 0:
            return DEFAULT_SIZE
        return (size.columns, size.lines)

    def write(self, text: str) -> None:
        """Writes `text` to the terminal and flushes it."""
        self._out.write(text)  # type: ignore[attr-defined]
        self._out.flush()  # type: ignore[attr-defined]

    def __enter__(self) -> Terminal:
        """Enters raw mode and the alternate screen."""
        self._saved = termios.tcgetattr(self._fd)
        tty.setraw(self._fd)
        self.write(self.ENTER)
        return self

    def __exit__(self, exc_type: object, exc: object, tb: object) -> None:
        """Restores the terminal, even when the body raised."""
        self.write(self.LEAVE)
        if self._saved is not None:
            termios.tcsetattr(self._fd, termios.TCSADRAIN, self._saved)
            self._saved = None


def paint(terminal: Terminal, text: str) -> None:
    """Repaints the whole grid from the top-left corner.

    This is the driver's own repaint: home the cursor, write each row followed
    by an erase-to-end, and erase everything below the last row. A host that
    writes its own event loop from :class:`Terminal` and :class:`Decoder` can
    call it directly with :meth:`Session.render_cells` text.
    """
    lines = text.split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    out = ["\x1b[H"]
    for index, line in enumerate(lines):
        out.append(line)
        out.append("\x1b[K")
        if index + 1 < len(lines):
            out.append("\r\n")
    out.append("\x1b[J")
    terminal.write("".join(out))


def _dispatch(
    session: Session, event: TerminalEvent, on_signal: Callable[[Signal], None] | None
) -> bool:
    """Sends one decoded event to the session; returns `False` to stop.

    A printable key emits `input.key` and then `input.text`, but only when no
    ctrl, alt, or meta modifier is held, matching the Rust driver.
    """
    if isinstance(event, Quit):
        return False
    signals: Sequence[Signal] = ()
    if isinstance(event, Key):
        signals = session.key(event.key, event.mods).signals
    elif isinstance(event, Text):
        signals = session.text(event.text).signals
    elif isinstance(event, Paste):
        signals = session.paste(event.text).signals
    elif isinstance(event, Pointer):
        x, y = pointer_units(event.col, event.row)
        signals = session.pointer(
            event.kind, x, y, button=event.button, clicks=event.clicks, mods=event.mods
        ).signals
    elif isinstance(event, Wheel):
        x, y = pointer_units(event.col, event.row)
        signals = session.wheel(x, y, event.notches * WHEEL_STEP, mods=event.mods).signals
    if on_signal is not None:
        for item in signals:
            on_signal(item)
    return True


def run(
    session: Session,
    *,
    dark: bool = False,
    fps: float = DEFAULT_FPS,
    on_signal: Callable[[Signal], None] | None = None,
    on_tick: Callable[[Session], None] | None = None,
    tick_interval: float = 1.0,
    fd: int | None = None,
) -> None:
    """Drives `session` interactively until Ctrl+C or `protocol.quit`.

    Args:
        session: A session with a document already loaded.
        dark: Whether to request the dark environment.
        fps: Frame rate for clock ticks and repaints; must be positive.
        on_signal: Called once per emitted signal, in emission order.
        on_tick: Called with the session once immediately after the first
            environment write and then every `tick_interval` seconds, so a
            host can write live params (a clock, timers) without owning the
            loop. Repaints triggered by its writes happen on the same frame.
        tick_interval: Seconds between `on_tick` calls; must be positive.
        fd: Input descriptor; defaults to standard input.

    Raises:
        ValueError: `fps` or `tick_interval` is not positive.
        OSError: The descriptor is not a terminal.
    """
    if fps <= 0:
        raise ValueError("fps must be positive")
    if tick_interval <= 0:
        raise ValueError("tick_interval must be positive")
    frame = 1.0 / fps
    decoder = Decoder()
    resized = [True]

    def on_winch(_number: int, _frame: object) -> None:
        resized[0] = True

    with Terminal(fd) as terminal:
        previous: object = None
        try:
            previous = signalmod.signal(signalmod.SIGWINCH, on_winch)
        except ValueError:
            previous = None
        try:
            cols, rows = terminal.size()
            painted = ""
            clock = time.monotonic()
            ticked: float | None = None
            running = True
            while running:
                if resized[0]:
                    resized[0] = False
                    cols, rows = terminal.size()
                    session.set_env_cells(cols, rows, dark=dark)
                    painted = ""
                ready, _, _ = select.select([terminal.fd], [], [], frame)
                if ready:
                    try:
                        data = os.read(terminal.fd, 4096)
                    except OSError:
                        data = b""
                    if not data:
                        break
                    for event in decoder.feed(data):
                        if not _dispatch(session, event, on_signal):
                            running = False
                            break
                else:
                    for event in decoder.flush():
                        if not _dispatch(session, event, on_signal):
                            running = False
                            break
                if not running:
                    break
                now = time.monotonic()
                elapsed = (now - clock) * 1000.0
                clock = now
                if elapsed > 0:
                    session.advance(elapsed)
                if on_tick is not None and (ticked is None or now - ticked >= tick_interval):
                    ticked = now
                    on_tick(session)
                if terminal.size() != (cols, rows):
                    resized[0] = True
                    continue
                text = session.render_cells(plain=False, caret=True).text
                if text != painted:
                    painted = text
                    paint(terminal, text)
                if session.has_quit:
                    running = False
        finally:
            if previous is not None:
                signalmod.signal(signalmod.SIGWINCH, previous)
