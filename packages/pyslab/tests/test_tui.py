"""Escape-sequence decoding and terminal translation rules."""

from __future__ import annotations

import pytest

from slab.tui import (
    WHEEL_STEP,
    Decoder,
    Key,
    Paste,
    Pointer,
    Quit,
    Terminal,
    Text,
    Wheel,
    pointer_units,
)

#: Byte sequence to expected events, matching `crates/slab-tui`.
SEQUENCES: list[tuple[bytes, list[object]]] = [
    (b"\x1b[A", [Key("ArrowUp")]),
    (b"\x1b[B", [Key("ArrowDown")]),
    (b"\x1b[C", [Key("ArrowRight")]),
    (b"\x1b[D", [Key("ArrowLeft")]),
    (b"\x1b[H", [Key("Home")]),
    (b"\x1b[F", [Key("End")]),
    (b"\x1b[2~", [Key("Insert")]),
    (b"\x1b[3~", [Key("Delete")]),
    (b"\x1b[5~", [Key("PageUp")]),
    (b"\x1b[6~", [Key("PageDown")]),
    (b"\x1b[15~", [Key("F5")]),
    (b"\x1b[24~", [Key("F12")]),
    (b"\x1bOP", [Key("F1")]),
    (b"\x1bOS", [Key("F4")]),
    (b"\x1b[1;2P", [Key("F1", ("shift",))]),
    (b"\x1b[1;5A", [Key("ArrowUp", ("ctrl",))]),
    (b"\x1b[1;3D", [Key("ArrowLeft", ("alt",))]),
    (b"\x1b[1;9C", [Key("ArrowRight", ("meta",))]),
    (b"\x1b[1;8H", [Key("Home", ("shift", "alt", "ctrl"))]),
    (b"\x1b[Z", [Key("Tab", ("shift",))]),
    (b"\t", [Key("Tab")]),
    (b"\r", [Key("Enter")]),
    (b"\n", [Key("Enter")]),
    (b"\x7f", [Key("Backspace")]),
    (b"\x08", [Key("Backspace")]),
    (b"\x1b\x1b", [Key("Escape")]),
    (b"\x01", [Key("a", ("ctrl",))]),
    (b"\x1a", [Key("z", ("ctrl",))]),
    (b"\x03", [Quit()]),
    (b"a", [Key("a"), Text("a")]),
    (b" ", [Key(" "), Text(" ")]),
    (b"\xc3\xa9", [Key("é"), Text("é")]),
    (b"\xf0\x9f\x9a\x80", [Key("\U0001f680"), Text("\U0001f680")]),
    (b"\x1ba", [Key("a", ("alt",))]),
    (b"\x1b[200~pasted\x1b[201~", [Paste("pasted")]),
    (b"\x1b[<0;1;1M", [Pointer("down", 0, 0, 0)]),
    (b"\x1b[<0;1;1m", [Pointer("up", 0, 0, 0)]),
    (b"\x1b[<1;5;3M", [Pointer("down", 4, 2, 1)]),
    (b"\x1b[<2;5;3M", [Pointer("down", 4, 2, 2)]),
    (b"\x1b[<32;7;4M", [Pointer("move", 6, 3, 0)]),
    (b"\x1b[<35;7;4M", [Pointer("move", 6, 3, 0)]),
    (b"\x1b[<64;2;2M", [Wheel(1, 1, -1)]),
    (b"\x1b[<65;2;2M", [Wheel(1, 1, 1)]),
    (b"\x1b[<16;5;3M", [Pointer("down", 4, 2, 0, ("ctrl",))]),
    (b"\x1b[<4;5;3M", [Pointer("down", 4, 2, 0, ("shift",))]),
]


@pytest.mark.parametrize(("data", "expected"), SEQUENCES, ids=lambda value: repr(value)[:32])
def test_decoder_table(data: bytes, expected: list[object]) -> None:
    """Each byte sequence decodes to exactly the listed events."""
    assert Decoder().feed(data) == expected


def test_printable_key_is_followed_by_text() -> None:
    """A printable character emits `input.key` and then `input.text`."""
    assert Decoder().feed(b"hi") == [Key("h"), Text("h"), Key("i"), Text("i")]


def test_modified_printable_key_emits_no_text() -> None:
    """Ctrl, alt, and meta suppress the text event."""
    for data in (b"\x01", b"\x1bx"):
        events = Decoder().feed(data)
        assert all(not isinstance(event, Text) for event in events), data


def test_lone_escape_waits_for_the_next_byte() -> None:
    """`ESC` alone is ambiguous until a read times out."""
    decoder = Decoder()
    assert decoder.feed(b"\x1b") == []
    assert decoder.pending == b"\x1b"
    assert decoder.flush() == [Key("Escape")]
    assert decoder.pending == b""


def test_flush_leaves_a_partial_sequence_alone() -> None:
    """A half-received CSI sequence survives the timeout and completes later."""
    decoder = Decoder()
    assert decoder.feed(b"\x1b[1;5") == []
    assert decoder.flush() == []
    assert decoder.feed(b"A") == [Key("ArrowUp", ("ctrl",))]


def test_sequences_split_across_reads() -> None:
    """The decoder is incremental, so read boundaries do not matter."""
    decoder = Decoder()
    events: list[object] = []
    for byte in b"\x1b[<0;10;5M":
        events.extend(decoder.feed(bytes([byte])))
    assert events == [Pointer("down", 9, 4, 0)]


def test_split_utf8_character() -> None:
    """A multi-byte character split across reads decodes once it is whole."""
    decoder = Decoder()
    assert decoder.feed(b"\xf0\x9f") == []
    assert decoder.feed(b"\x9a\x80") == [Key("\U0001f680"), Text("\U0001f680")]


def test_split_bracketed_paste() -> None:
    """Paste payloads accumulate across reads until the closing marker."""
    decoder = Decoder()
    assert decoder.feed(b"\x1b[200~one ") == []
    assert decoder.feed(b"two") == []
    assert decoder.feed(b"\x1b[201~") == [Paste("one two")]


def test_pointer_units_are_cell_centres() -> None:
    """Mouse cells map to the centre of the cell in layout units."""
    assert pointer_units(0, 0) == (4.0, 8.0)
    assert pointer_units(1, 1) == (12.0, 24.0)
    assert pointer_units(10, 3) == (84.0, 56.0)


def test_wheel_step_is_three_rows() -> None:
    """One notch scrolls three terminal rows worth of layout units."""
    assert WHEEL_STEP == 48.0


def test_terminal_sequences_are_balanced() -> None:
    """Every mode the driver turns on is turned back off on exit."""
    modes = ("1049", "1000", "1002", "1003", "1006", "2004")
    for mode in modes:
        assert f"[?{mode}h" in Terminal.ENTER
        assert f"[?{mode}l" in Terminal.LEAVE
    assert "[?25l" in Terminal.ENTER
    assert "[?25h" in Terminal.LEAVE
