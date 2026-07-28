"""Drive `examples/10-settings.slab` headlessly and print the signals it emits.

Run it from anywhere::

    uv run python examples/settings.py

The document is compiled on the fly by the embedded WebAssembly runtime, sized
for a terminal grid, rendered as text, and then clicked. Every signal the
document emits is printed with the pointer context the kernel attached.
"""

from __future__ import annotations

import sys
from pathlib import Path

import slab

#: Repository-relative path to the document this example drives.
DOCUMENT = Path(__file__).resolve().parents[3] / "examples" / "10-settings.slab"

#: Terminal grid this example renders for.
COLS, ROWS = 100, 32


def show(session: slab.Session, title: str) -> None:
    """Prints the rendered grid under a heading."""
    cells = session.render_cells(plain=True)
    print(f"--- {title} ({cells.cols}x{cells.rows}) ---")
    print(cells.text.rstrip("\n"))


def report(effects: slab.Effects, label: str) -> None:
    """Prints every signal one interaction produced."""
    if not effects.signals:
        print(f"{label}: no signals")
        return
    for item in effects.signals:
        print(
            f"{label}: signal {item.name!r} text={item.text!r} item={item.item!r} "
            f"at ({item.meta.x:.0f}, {item.meta.y:.0f}) key={item.meta.key}"
        )


def main() -> int:
    """Opens the document, exercises it, and reports what came back."""
    if not DOCUMENT.exists():
        print(f"missing {DOCUMENT}", file=sys.stderr)
        return 1

    with slab.open_file(DOCUMENT) as session:
        session.set_env_cells(COLS, ROWS, dark=True)

        info = session.info()
        print(f"document: {info.file}")
        print(f"params:   {', '.join(str(p['name']) for p in info.params)}")
        print(f"signals:  {', '.join(info.signals)}")
        print(f"holes:    {', '.join(info.holes)}")
        print()

        show(session, "initial")
        print()

        for button in ("#save", "#reset", "#sort"):
            report(session.click(key=button), f"click {button}")

        session.set_param("status", "saved")
        session.set_param("title", "Settings (driven from Python)")

        report(session.key("Tab"), "key Tab")
        report(session.text("hello"), "text")
        report(session.key("Enter"), "key Enter")

        session.advance(120.0)
        print()
        show(session, "after input")
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
