"""Command line entry point: `python -m slab FILE.slab`.

Compiles the file in process through the embedded WebAssembly module and drives
it in the terminal. There is no build step and no `slab` binary involved.
"""

from __future__ import annotations

import argparse
import sys

from . import CompileError, Signal, open_file
from .tui import DEFAULT_FPS, run

__all__ = ["main", "parser"]


def parser() -> argparse.ArgumentParser:
    """Builds the argument parser, so `--help` documents every flag."""
    parsed = argparse.ArgumentParser(
        prog="python -m slab",
        description=(
            "Run a .slab document in the terminal. The document is parsed and "
            "compiled on the fly by the embedded Slab WebAssembly runtime."
        ),
        epilog="Ctrl+C quits. The mouse, keyboard, and clipboard paste all work.",
    )
    parsed.add_argument("file", metavar="FILE.slab", help="path to a .slab source file")
    parsed.add_argument(
        "--dark",
        action="store_true",
        help="request the dark environment instead of the light one",
    )
    parsed.add_argument(
        "--fps",
        type=float,
        default=DEFAULT_FPS,
        metavar="N",
        help=f"clock tick and repaint rate; default {DEFAULT_FPS:g}",
    )
    parsed.add_argument(
        "--quiet",
        action="store_true",
        help="do not print emitted signals after the session ends",
    )
    return parsed


def main(argv: list[str] | None = None) -> int:
    """Runs the driver and returns a process exit status.

    Args:
        argv: Argument list without the program name; defaults to `sys.argv`.

    Returns:
        `0` on a clean exit, `1` when the document did not compile, and `2`
        when standard input is not a terminal.
    """
    args = parser().parse_args(argv)
    if args.fps <= 0:
        print("slab: --fps must be positive", file=sys.stderr)
        return 2
    if not sys.stdin.isatty():
        print("slab: standard input is not a terminal", file=sys.stderr)
        return 2
    try:
        session = open_file(args.file)
    except CompileError as err:
        print(f"slab: {args.file} did not compile", file=sys.stderr)
        for line in err.result.messages():
            print(f"  {line}", file=sys.stderr)
        return 1
    except OSError as err:
        print(f"slab: {err}", file=sys.stderr)
        return 1

    emitted: list[Signal] = []
    try:
        run(session, dark=args.dark, fps=args.fps, on_signal=emitted.append)
    finally:
        session.close()
    if not args.quiet:
        for item in emitted:
            print(f"{item.name}\t{item.text}\t{item.item}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
