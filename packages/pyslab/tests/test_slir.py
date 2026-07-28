"""Load precompiled SLIR, the ahead-of-time counterpart to `doc.open`."""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest

import slab

from conftest import REPO_ROOT, SETTINGS


@pytest.fixture(scope="session")
def settings_slir(tmp_path_factory: pytest.TempPathFactory) -> bytes:
    """Builds `examples/10-settings.slab` with `slab build` and returns the bytes.

    The test is skipped when the Rust toolchain or the example is unavailable,
    because the Python package itself never needs cargo.
    """
    if shutil.which("cargo") is None:
        pytest.skip("cargo is not installed")
    if not SETTINGS.exists():
        pytest.skip(f"missing {SETTINGS}")
    out = tmp_path_factory.mktemp("slir") / "settings.slir"
    built = subprocess.run(
        [
            "cargo",
            "run",
            "-q",
            "-p",
            "slab-cli",
            "--",
            "build",
            str(SETTINGS),
            "-o",
            str(out),
        ],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if built.returncode != 0 or not out.exists():
        pytest.skip(f"slab build is unavailable: {built.stderr.strip()[-200:]}")
    return Path(out).read_bytes()


def test_open_slir_renders_the_document(settings_slir: bytes, runtime: slab.Runtime) -> None:
    """Precompiled SLIR installs without running the compiler."""
    loaded = runtime.new_session()
    try:
        result = loaded.open_slir(settings_slir, "settings.slir")
        assert result.ok
        assert result.diags == []
        loaded.set_env_cells(100, 32)
        cells = loaded.render_cells(plain=True)
        assert "Settings" in cells.text
        assert cells.cols == 100
    finally:
        loaded.close()


def test_open_slir_document_is_interactive(settings_slir: bytes, runtime: slab.Runtime) -> None:
    """A SLIR-loaded document behaves exactly like a compiled one."""
    loaded = runtime.new_session()
    try:
        loaded.open_slir(settings_slir, "settings.slir")
        loaded.set_env_cells(100, 32)
        assert [item.name for item in loaded.click(key="#save").signals] == ["save"]
        assert {"save", "reset", "sort"} <= set(loaded.info().signals)
    finally:
        loaded.close()


def test_module_level_open_slir_owns_its_runtime(settings_slir: bytes) -> None:
    """`slab.open_slir` returns a ready session that closes its own runtime."""
    with slab.open_slir(settings_slir, "settings.slir") as owned:
        holder = owned.runtime
        owned.set_env_cells(100, 32)
        assert "Settings" in owned.render_cells(plain=True).text
    assert holder.closed


def test_bad_slir_payload_returns_diagnostics(session: slab.Session) -> None:
    """A rejected payload is data, matching `doc.open` failure semantics."""
    result = session.open_slir(b"not slir at all", "bad.slir")
    assert result.ok is False
    assert result.diags
    assert result.messages()


def test_module_level_open_slir_raises_on_a_bad_payload() -> None:
    """The convenience constructor promises a ready session, so it raises."""
    with pytest.raises(slab.CompileError) as caught:
        slab.open_slir(b"not slir at all", "bad.slir")
    assert caught.value.diags
