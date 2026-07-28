"""Shared fixtures: one compiled runtime for the whole test session."""

from __future__ import annotations

from collections.abc import Iterator
from pathlib import Path

import pytest

import slab

#: Repository root, four levels above this file.
REPO_ROOT = Path(__file__).resolve().parents[3]

#: Repo example every integration test drives.
SETTINGS = REPO_ROOT / "examples" / "10-settings.slab"


@pytest.fixture(scope="session")
def runtime() -> Iterator[slab.Runtime]:
    """Compiles the embedded module once and shares it across every test."""
    created = slab.Runtime()
    yield created
    created.close()


@pytest.fixture
def session(runtime: slab.Runtime) -> Iterator[slab.Session]:
    """Yields a fresh session with no document loaded."""
    created = runtime.new_session()
    yield created
    created.close()


@pytest.fixture
def settings(runtime: slab.Runtime) -> Iterator[slab.Session]:
    """Yields a session with `examples/10-settings.slab` loaded and sized."""
    created = runtime.open_file(SETTINGS)
    created.set_env_cells(100, 32)
    yield created
    created.close()
