"""Drive a real repository example and check the signals it emits."""

from __future__ import annotations

import pytest

import slab
from slab.tui import pointer_units

from conftest import SETTINGS


def test_example_file_is_present() -> None:
    """The integration tests need the repository checkout."""
    assert SETTINGS.exists(), f"missing {SETTINGS}"


def test_click_on_save_emits_the_save_signal(settings: slab.Session) -> None:
    """Clicking the authored `#save` button emits exactly that signal."""
    effects = settings.click(key="#save")
    assert [item.name for item in effects.signals] == ["save"]
    assert effects.signals[0].meta.key.endswith("#save/row@0")


def test_click_on_each_button_emits_its_own_signal(settings: slab.Session) -> None:
    """Each button carries a distinct action; the kernel resolves the hit."""
    for node, name in (("#save", "save"), ("#reset", "reset"), ("#sort", "sort")):
        assert [item.name for item in settings.click(key=node).signals] == [name]


def test_click_at_terminal_coordinates_hits_the_same_button(settings: slab.Session) -> None:
    """A cell click converted with `pointer_units` reaches the same node."""
    keyed = settings.click(key="#save")
    x, y = keyed.signals[0].meta.x, keyed.signals[0].meta.y
    col, row = int(x // 8), int(y // 16)
    px, py = pointer_units(col, row)
    effects = settings.click(px, py)
    assert [item.name for item in effects.signals] == ["save"]


def test_click_on_empty_space_emits_nothing(settings: slab.Session) -> None:
    """Hit testing lives in the kernel; a miss produces no signal."""
    px, py = pointer_units(95, 30)
    assert settings.click(px, py).signals == ()


def test_rendered_grid_matches_the_requested_terminal_size(settings: slab.Session) -> None:
    """`render.cells` honours the environment the host set."""
    settings.set_env_cells(72, 20)
    cells = settings.render_cells(plain=True)
    assert cells.cols == 72
    assert cells.rows <= 20
    assert "Save" in cells.text


def test_coloured_render_carries_ansi_and_the_caret(settings: slab.Session) -> None:
    """The driver paints `plain=False, caret=True` output verbatim."""
    settings.click(key="#field")
    coloured = settings.render_cells(plain=False, caret=True)
    plain = settings.render_cells(plain=True, caret=False)
    assert "\x1b[" in coloured.text
    assert "\x1b[" not in plain.text
    assert all(isinstance(note, str) for note in coloured.notes)


def test_wheel_scrolls_the_panel(settings: slab.Session) -> None:
    """Scrolling is kernel work; the host only reports notches in units."""
    settings.set_env_cells(100, 20)
    px, py = pointer_units(50, 14)
    effects = settings.wheel(px, py, 3.0 * 16.0)
    assert isinstance(effects, slab.Effects)
    assert effects.t == pytest.approx(0.0)
