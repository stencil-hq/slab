"""Behaviour of the runtime, the session, and on-the-fly compilation."""

from __future__ import annotations

import pytest

import slab

from conftest import SETTINGS

#: A minimal document with two authored strings to look for in the render.
HELLO = 'col pad=4 gap=1 { text "greetings" size=13\n  text "from python" size=13 }'

#: A document that cannot be tokenised.
BROKEN = "col pad=4 { ~~~ }"


def test_abi_version_is_one(runtime: slab.Runtime) -> None:
    """The embedded module implements the ABI revision this client speaks."""
    assert runtime.abi_version == slab.ABI_VERSION == 1


def test_inline_source_renders_authored_text(session: slab.Session) -> None:
    """`doc.open` compiles inline source and the render carries its text."""
    result = session.open(HELLO, "hello.slab")
    assert result.ok
    assert result.diags == []
    session.set_env_cells(60, 12)
    cells = session.render_cells(plain=True)
    assert cells.cols == 60
    assert "greetings" in cells.text
    assert "from python" in cells.text


def test_broken_source_returns_diagnostics(session: slab.Session) -> None:
    """A compile failure is data, not an exception."""
    result = session.open(BROKEN, "broken.slab")
    assert result.ok is False
    assert result.diags
    assert all("msg" in diag for diag in result.diags)
    assert result.messages()


def test_broken_source_keeps_the_previous_document(session: slab.Session) -> None:
    """A failed load leaves the document that was already running in place."""
    assert session.open(HELLO, "hello.slab").ok
    session.set_env_cells(60, 12)
    assert session.open(BROKEN, "broken.slab").ok is False
    assert "greetings" in session.render_cells(plain=True).text


def test_open_source_helper_raises_on_a_broken_document() -> None:
    """The convenience constructor promises a ready session, so it raises."""
    with pytest.raises(slab.CompileError) as caught:
        slab.open_source(BROKEN, "broken.slab")
    assert caught.value.diags


def test_open_file_reads_the_source_in_python() -> None:
    """`open_file` reads the file host-side; the module has no filesystem."""
    with slab.open_file(SETTINGS) as opened:
        opened.set_env_cells(100, 32)
        assert "Settings" in opened.render_cells(plain=True).text


def test_unknown_method_raises_protocol_error(session: slab.Session) -> None:
    """An SDP `error` object becomes a `ProtocolError` carrying its code."""
    with pytest.raises(slab.ProtocolError) as caught:
        session.request("nope.nope")
    assert caught.value.code == -32601
    assert caught.value.method == "nope.nope"
    assert "nope.nope" in caught.value.message


def test_unknown_parameter_raises_protocol_error(settings: slab.Session) -> None:
    """A rejected parameter surfaces the SDP error rather than a stack trace."""
    with pytest.raises(slab.ProtocolError) as caught:
        settings.set_param("nope", "x")
    assert caught.value.code == -32000
    assert "unknown param" in caught.value.message


def test_closed_session_raises(runtime: slab.Runtime) -> None:
    """Every method on a closed session fails fast."""
    closed = runtime.new_session()
    closed.close()
    assert closed.closed
    with pytest.raises(slab.SlabError):
        closed.request("doc.info")
    with pytest.raises(slab.SlabError):
        closed.render_cells()
    closed.close()


def test_closed_runtime_refuses_new_sessions() -> None:
    """Closing a runtime tears down its instance."""
    owned = slab.Runtime()
    owned.close()
    assert owned.closed
    with pytest.raises(slab.AbiError):
        owned.new_session()


def test_quit_marks_the_session_finished(session: slab.Session) -> None:
    """`protocol.quit` is observable through the ABI's quit flag."""
    assert session.has_quit is False
    session.quit()
    assert session.has_quit is True


def test_context_manager_closes_the_owned_runtime() -> None:
    """A session from `open_source` closes the runtime it owns."""
    with slab.open_source(HELLO, "hello.slab") as owned:
        holder = owned.runtime
        assert holder.closed is False
    assert owned.closed
    assert holder.closed


def test_doc_info_lists_the_authored_surface(settings: slab.Session) -> None:
    """`doc.info` reports the parameters, signals, and holes of the document."""
    info = settings.info()
    assert info.file.endswith("10-settings.slab")
    assert {"save", "reset", "sort", "draft_submit"} <= set(info.signals)
    assert "rows" in info.holes
    assert {str(param["name"]) for param in info.params} >= {"title", "status", "draft"}


def test_parameters_round_trip(settings: slab.Session) -> None:
    """A parameter write reaches the kernel and shows up in the render."""
    settings.set_param("title", "Driven")
    assert settings.get_param("title") == "Driven"
    assert "Driven" in settings.render_cells(plain=True).text


def test_clock_advances_monotonically(settings: slab.Session) -> None:
    """`clock.advance` moves the virtual clock and reports the new value."""
    assert settings.time() == 0.0
    assert settings.advance(16.0) == pytest.approx(16.0)
    assert settings.advance(4.0) == pytest.approx(20.0)
    with pytest.raises(ValueError):
        settings.advance(-1.0)


def test_environment_uses_terminal_cell_units(settings: slab.Session) -> None:
    """A terminal grid maps to `cols * 8` by `rows * 16` layout units."""
    env = settings.set_env_cells(80, 24, dark=True)
    assert env["width"] == pytest.approx(640.0)
    assert env["height"] == pytest.approx(384.0)
    assert env["client"] == "tui"
    assert env["dark"] is True


def test_typing_into_a_field_emits_a_signal(settings: slab.Session) -> None:
    """Editing is kernel work; the host only forwards the text."""
    settings.click(key="#field")
    effects = settings.text("draft text")
    assert [item.name for item in effects.named("draft")] == ["draft"]
    assert effects.named("draft")[0].text == "draft text"
    submitted = settings.key("Enter")
    assert submitted.named("draft_submit")[0].text == "draft text"


def test_unknown_modifier_is_rejected(settings: slab.Session) -> None:
    """Only the four SDP modifier names are accepted."""
    with pytest.raises(ValueError):
        settings.key("a", ["hyper"])


def test_click_needs_a_key_or_a_point(settings: slab.Session) -> None:
    """`input.click` takes a keyed node or a point, never both and never none."""
    with pytest.raises(ValueError):
        settings.click()
    with pytest.raises(ValueError):
        settings.click(1.0, 2.0, key="#save")
