"""`python -m slab` argument handling and pre-frame overrides."""

from __future__ import annotations

import pytest

import slab
from slab.__main__ import _apply_overrides, parser

#: A document with a declared theme and two scalars to override.
THEMED = (
    "theme dusk { }\n"
    'params { label text = "hi"\n  flag bool = false }\n'
    "col pad=4 { text param.label size=13 }"
)


def test_parser_collects_repeated_sets_and_theme() -> None:
    """`--set` is repeatable and `--theme` takes one name."""
    args = parser().parse_args(
        ["doc.slab", "--set", "label=hey", "--set", "flag=true", "--theme", "dusk"]
    )
    assert args.sets == ["label=hey", "flag=true"]
    assert args.theme == "dusk"


def test_overrides_apply_before_the_first_frame(session: slab.Session) -> None:
    """Raw `--set` values are typed like the slab-tui CLI."""
    assert session.open(THEMED).ok
    session.set_env_cells(40, 6)
    args = parser().parse_args(
        ["doc.slab", "--set", "label=driven", "--set", "flag=true", "--theme", "dusk"]
    )
    assert _apply_overrides(session, args) is None
    assert session.get_param("label") == "driven"
    assert session.get_param("flag") is True
    assert session.get_env().get("theme") == "dusk"


def test_unknown_theme_is_reported(session: slab.Session) -> None:
    """An unknown `--theme` produces the protocol's message, not a crash."""
    assert session.open(THEMED).ok
    args = parser().parse_args(["doc.slab", "--theme", "nope"])
    assert "unknown theme" in (_apply_overrides(session, args) or "")


def test_malformed_set_is_reported(session: slab.Session) -> None:
    """A `--set` without `param=value` is rejected with a usage message."""
    assert session.open(THEMED).ok
    args = parser().parse_args(["doc.slab", "--set", "oops"])
    assert "param=value" in (_apply_overrides(session, args) or "")


def test_unknown_param_set_is_reported(session: slab.Session) -> None:
    """A `--set` naming an undeclared param surfaces the protocol error."""
    assert session.open(THEMED).ok
    args = parser().parse_args(["doc.slab", "--set", "ghost=1"])
    assert "ghost" in (_apply_overrides(session, args) or "")


def test_run_rejects_a_nonpositive_tick_interval() -> None:
    """`slab.tui.run` validates `tick_interval` before touching the terminal."""
    from slab.tui import run

    with pytest.raises(ValueError):
        run(object(), tick_interval=0.0)  # type: ignore[arg-type]
