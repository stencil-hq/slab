"""Slab for Python: compile and drive `.slab` documents in process.

This package embeds the Slab compiler, kernel, and Slab Drive Protocol (SDP)
session layer as a WebAssembly module, so a document is parsed and compiled on
the fly at runtime with no build step and no `slab` binary.

The kernel owns layout, hit testing, focus, scrolling, and text editing. A host
built on this package only feeds input in and paints the cells that come back.

Typical use::

    import slab

    with slab.open_file("examples/10-settings.slab") as session:
        session.set_env_cells(cols=100, rows=32, dark=True)
        print(session.render_cells(plain=True).text)
        for signal in session.click(key="#save").signals:
            print(signal.name, signal.text)

:class:`Session.request` reaches every SDP method in `spec/SDP.md`; the typed
helpers on :class:`Session` are shorthands for the ones a host needs most.
"""

from __future__ import annotations

import base64
import json
import os
import threading
from dataclasses import dataclass, field
from pathlib import Path
from types import TracebackType
from typing import Any, Iterable, Mapping

from ._abi import ABI_VERSION, Abi, AbiError

__all__ = [
    "ABI_VERSION",
    "AbiError",
    "Cells",
    "CompileError",
    "DocInfo",
    "Effects",
    "EnvSpec",
    "LoadResult",
    "ProtocolError",
    "Rect",
    "Runtime",
    "Scroll",
    "Session",
    "Signal",
    "SignalMeta",
    "SlabError",
    "open_file",
    "open_slir",
    "open_source",
]

#: Terminal cell width in Slab layout units.
CELL_WIDTH = 8.0

#: Terminal cell height in Slab layout units.
CELL_HEIGHT = 16.0

#: Modifier names SDP accepts on key, pointer, and wheel input.
MODIFIERS = ("shift", "alt", "ctrl", "meta")


class SlabError(Exception):
    """Base class for every error this package raises."""


class ProtocolError(SlabError):
    """An SDP response carried an `error` object instead of a `result`.

    Attributes:
        code: SDP error code, for example `-32601` for an unknown method.
        message: Human-readable message from the session.
        method: Method that produced the error.
    """

    def __init__(self, code: int, message: str, method: str = "") -> None:
        """Builds an error from the decoded SDP `error` object."""
        super().__init__(f"{method}: {message} (code {code})" if method else f"{message} (code {code})")
        self.code = code
        self.message = message
        self.method = method


class CompileError(SlabError):
    """A convenience constructor was handed a source that does not compile.

    Attributes:
        result: The full :class:`LoadResult`, including every diagnostic.
    """

    def __init__(self, result: LoadResult) -> None:
        """Builds an error from a failed load result."""
        detail = "; ".join(result.messages()) or "compile failed"
        super().__init__(detail)
        self.result = result

    @property
    def diags(self) -> list[dict[str, Any]]:
        """Diagnostics reported by the compiler, in source order."""
        return self.result.diags


@dataclass(frozen=True, slots=True)
class Rect:
    """Axis-aligned rectangle in Slab layout units."""

    x: float
    y: float
    w: float
    h: float

    @classmethod
    def decode(cls, value: Any) -> Rect | None:
        """Decodes the `[x, y, w, h]` array SDP uses, or `None` for null."""
        if not isinstance(value, list) or len(value) != 4:
            return None
        return cls(float(value[0]), float(value[1]), float(value[2]), float(value[3]))


@dataclass(frozen=True, slots=True)
class SignalMeta:
    """Context the kernel attaches to one emitted signal.

    The pointer fields are in Slab layout units; `mods` is the packed modifier
    bitmask the kernel used for the originating event.
    """

    x: float = 0.0
    y: float = 0.0
    dx: float = 0.0
    dy: float = 0.0
    drag_dx: float = 0.0
    drag_dy: float = 0.0
    mods: int = 0
    button: int = 0
    clicks: int = 0
    key: str = ""
    src_key: str = ""
    src_item: str = ""
    cancelled: bool = False
    dropped: bool = False

    @classmethod
    def decode(cls, value: Any) -> SignalMeta:
        """Decodes one `effects.signals[].meta` object, tolerating omissions."""
        if not isinstance(value, dict):
            return cls()
        return cls(
            x=float(value.get("x", 0.0)),
            y=float(value.get("y", 0.0)),
            dx=float(value.get("dx", 0.0)),
            dy=float(value.get("dy", 0.0)),
            drag_dx=float(value.get("drag_dx", 0.0)),
            drag_dy=float(value.get("drag_dy", 0.0)),
            mods=int(value.get("mods", 0)),
            button=int(value.get("button", 0)),
            clicks=int(value.get("clicks", 0)),
            key=str(value.get("key", "")),
            src_key=str(value.get("src_key", "")),
            src_item=str(value.get("src_item", "")),
            cancelled=bool(value.get("cancelled", False)),
            dropped=bool(value.get("dropped", False)),
        )


@dataclass(frozen=True, slots=True)
class Signal:
    """One signal the document emitted, in emission order.

    Attributes:
        name: Authored signal name, for example `save`.
        text: Payload text the document attached.
        item: Item key when the signal came from a list row, else empty.
        meta: Pointer, modifier, and source context.
    """

    name: str
    text: str = ""
    item: str = ""
    meta: SignalMeta = field(default_factory=SignalMeta)

    @classmethod
    def decode(cls, value: Mapping[str, Any]) -> Signal:
        """Decodes one entry of `effects.signals`."""
        return cls(
            name=str(value.get("name", "")),
            text=str(value.get("text", "")),
            item=str(value.get("item", "")),
            meta=SignalMeta.decode(value.get("meta")),
        )


@dataclass(frozen=True, slots=True)
class Scroll:
    """One scroll offset the kernel changed while handling an event."""

    key: str
    axis: int
    off: float


@dataclass(frozen=True, slots=True)
class Effects:
    """Result of one input dispatch.

    Attributes:
        repaint: Whether the scene changed and needs a repaint.
        signals: Ordered signals the document emitted.
        scrolls: Scroll offsets the kernel changed.
        caret: Text caret rectangle, or `None` when there is no caret.
        ime: Input-method rectangle, or `None`.
        cursor: Kernel cursor code for the pointer shape.
        focus: Key of the focused node, or `None`.
        t: Virtual clock value in milliseconds after the dispatch.
    """

    repaint: bool = False
    signals: tuple[Signal, ...] = ()
    scrolls: tuple[Scroll, ...] = ()
    caret: Rect | None = None
    ime: Rect | None = None
    cursor: int = 0
    focus: str | None = None
    t: float = 0.0

    @classmethod
    def decode(cls, result: Mapping[str, Any]) -> Effects:
        """Decodes the `{"effects": ..., "t": ...}` result of an input method."""
        effects = result.get("effects")
        if not isinstance(effects, dict):
            effects = {}
        signals = tuple(
            Signal.decode(item)
            for item in effects.get("signals", [])
            if isinstance(item, dict)
        )
        scrolls = tuple(
            Scroll(str(item.get("key", "")), int(item.get("axis", 0)), float(item.get("off", 0.0)))
            for item in effects.get("scrolls", [])
            if isinstance(item, dict)
        )
        focus = effects.get("focus")
        return cls(
            repaint=bool(effects.get("repaint", False)),
            signals=signals,
            scrolls=scrolls,
            caret=Rect.decode(effects.get("caret")),
            ime=Rect.decode(effects.get("ime")),
            cursor=int(effects.get("cursor", 0)),
            focus=str(focus) if isinstance(focus, str) else None,
            t=float(result.get("t", 0.0)),
        )

    def named(self, name: str) -> tuple[Signal, ...]:
        """Returns the signals whose name equals `name`, in emission order."""
        return tuple(signal for signal in self.signals if signal.name == name)


@dataclass(frozen=True, slots=True)
class Cells:
    """Terminal rendering returned by `render.cells`.

    Attributes:
        text: The painted grid. With `plain=False` it carries ANSI colour and
            the caret; with `plain=True` it is bare text.
        cols: Column count of the grid.
        rows: Row count of the grid.
        notes: Renderer notes, for example unsupported-feature warnings.
    """

    text: str
    cols: int
    rows: int
    notes: tuple[str, ...] = ()

    @classmethod
    def decode(cls, result: Mapping[str, Any]) -> Cells:
        """Decodes a `render.cells` result object."""
        return cls(
            text=str(result.get("text", "")),
            cols=int(result.get("cols", 0)),
            rows=int(result.get("rows", 0)),
            notes=tuple(str(note) for note in result.get("notes", [])),
        )

    def lines(self) -> list[str]:
        """Splits :attr:`text` into rows without trailing empty padding."""
        return self.text.split("\n")


@dataclass(frozen=True, slots=True)
class EnvSpec:
    """Environment payload for `env.set`.

    A terminal host derives the size from its cell grid: `width = cols * 8` and
    `height = rows * 16`. Fields left as `None` keep their current value,
    because `env.set` merges.
    """

    width: float | None = None
    height: float | None = None
    client: str | None = None
    dark: bool | None = None
    coarse: bool | None = None
    theme: str | None = None

    @classmethod
    def for_cells(
        cls, cols: int, rows: int, *, dark: bool = False, coarse: bool = False
    ) -> EnvSpec:
        """Builds a `client="tui"` environment for a `cols` by `rows` grid."""
        return cls(
            width=float(cols) * CELL_WIDTH,
            height=float(rows) * CELL_HEIGHT,
            client="tui",
            dark=dark,
            coarse=coarse,
        )

    def params(self) -> dict[str, Any]:
        """Returns the SDP parameter object, omitting unset fields."""
        out: dict[str, Any] = {}
        for name in ("width", "height", "client", "dark", "coarse", "theme"):
            value = getattr(self, name)
            if value is not None:
                out[name] = value
        return out


@dataclass(frozen=True, slots=True)
class DocInfo:
    """Result of `doc.info`.

    Attributes:
        file: Path or name the document was loaded under.
        params: Parameter declarations, each with `name`, `type`, and for enum
            parameters an `enum` member list.
        themes: Declared theme names.
        holes: Declared host-content hole names.
        signals: Declared signal names.
        env: Current environment, matching `env.get`.
        t: Virtual clock value in milliseconds.
    """

    file: str
    params: tuple[dict[str, Any], ...] = ()
    themes: tuple[str, ...] = ()
    holes: tuple[str, ...] = ()
    signals: tuple[str, ...] = ()
    env: Mapping[str, Any] = field(default_factory=dict)
    t: float = 0.0

    @classmethod
    def decode(cls, result: Mapping[str, Any]) -> DocInfo:
        """Decodes a `doc.info` result object."""
        env = result.get("env")
        return cls(
            file=str(result.get("file", "")),
            params=tuple(item for item in result.get("params", []) if isinstance(item, dict)),
            themes=tuple(str(name) for name in result.get("themes", [])),
            holes=tuple(str(name) for name in result.get("holes", [])),
            signals=tuple(str(name) for name in result.get("signals", [])),
            env=dict(env) if isinstance(env, dict) else {},
            t=float(result.get("t", 0.0)),
        )


@dataclass(frozen=True, slots=True)
class LoadResult:
    """Result of `doc.open`, `doc.load`, or `doc.reload`.

    A compile failure is data, not an exception: `ok` is `False`, `diags` holds
    the diagnostics, and the previously loaded document keeps running.

    Attributes:
        ok: Whether the document compiled and became the live document.
        diags: Compiler diagnostics, each with at least `code` and `msg`.
        reloaded: Whether the load replaced an already-loaded document.
        theme_reset: Whether an unknown reapplied theme fell back to the base.
    """

    ok: bool
    diags: list[dict[str, Any]] = field(default_factory=list)
    reloaded: bool = False
    theme_reset: bool = False

    @classmethod
    def decode(cls, result: Mapping[str, Any]) -> LoadResult:
        """Decodes a load result object."""
        return cls(
            ok=bool(result.get("ok", False)),
            diags=[item for item in result.get("diags", []) if isinstance(item, dict)],
            reloaded=bool(result.get("reloaded", False)),
            theme_reset=bool(result.get("theme_reset", False)),
        )

    def messages(self) -> list[str]:
        """Returns one readable line per diagnostic."""
        out = []
        for diag in self.diags:
            code = diag.get("code", "")
            line = diag.get("line", 0)
            msg = diag.get("msg", diag.get("message", ""))
            out.append(f"{code} line {line}: {msg}" if line else f"{code}: {msg}")
        return out


def _mods(mods: Iterable[str] | None) -> list[str]:
    """Normalises a modifier iterable, rejecting names SDP does not accept."""
    if not mods:
        return []
    out = []
    for name in mods:
        lowered = str(name).lower()
        if lowered not in MODIFIERS:
            raise ValueError(f"unknown modifier {name!r}; expected one of {MODIFIERS}")
        out.append(lowered)
    return out


class Runtime:
    """A compiled Slab module and one WebAssembly instance to run sessions in.

    Compiling the module is the expensive part, so create one runtime and reuse
    it. Sessions are cheap. The runtime serialises calls with a lock, so it is
    safe to share between threads.
    """

    __slots__ = ("_abi", "_lock", "_sessions")

    def __init__(self, engine: Any | None = None) -> None:
        """Instantiates the embedded module.

        Args:
            engine: Optional shared :class:`wasmtime.Engine`. Reusing an engine
                across runtimes reuses the compiled module.
        """
        self._abi = Abi(engine)
        self._lock = threading.RLock()
        self._sessions: list[Session] = []

    @property
    def abi_version(self) -> int:
        """ABI revision the loaded module implements."""
        return self._abi.version

    @property
    def closed(self) -> bool:
        """Whether :meth:`close` already ran."""
        return self._abi.closed

    def new_session(self) -> Session:
        """Creates a session with no document loaded."""
        with self._lock:
            handle = self._abi.session_new()
            session = Session(self, handle)
            self._sessions.append(session)
            return session

    def open_source(self, source: str, name: str = "<source>") -> Session:
        """Creates a session and compiles `source` into it.

        Raises:
            CompileError: The source did not compile.
        """
        session = self.new_session()
        try:
            result = session.open(source, name)
        except BaseException:
            session.close()
            raise
        if not result.ok:
            session.close()
            raise CompileError(result)
        return session

    def open_file(self, path: str | os.PathLike[str]) -> Session:
        """Reads a `.slab` file and compiles it into a new session.

        The WebAssembly module has no filesystem, so the host reads the text and
        sends it through `doc.open`.

        Raises:
            CompileError: The file did not compile.
            OSError: The file could not be read.
        """
        text = Path(path).read_text(encoding="utf-8")
        session = self.new_session()
        try:
            result = session.open(text, str(path))
        except BaseException:
            session.close()
            raise
        if not result.ok:
            session.close()
            raise CompileError(result)
        return session

    def open_slir(self, slir: bytes, name: str = "<slir>") -> Session:
        """Creates a session and installs precompiled SLIR bytes into it.

        Raises:
            CompileError: The payload was rejected.
        """
        session = self.new_session()
        try:
            result = session.open_slir(slir, name)
        except BaseException:
            session.close()
            raise
        if not result.ok:
            session.close()
            raise CompileError(result)
        return session

    def _request(self, handle: int, line: str) -> str:
        """Serialises one raw ABI request against the shared instance."""
        with self._lock:
            return self._abi.request(handle, line)

    def _free(self, handle: int) -> None:
        """Releases one session handle."""
        with self._lock:
            if not self._abi.closed:
                self._abi.session_free(handle)

    def _quit(self, handle: int) -> bool:
        """Reports whether a session already ended through `protocol.quit`."""
        with self._lock:
            return self._abi.session_quit(handle)

    def close(self) -> None:
        """Closes every session and drops the WebAssembly instance."""
        with self._lock:
            for session in list(self._sessions):
                session.close()
            self._sessions.clear()
            self._abi.close()

    def __enter__(self) -> Runtime:
        """Returns the runtime for use in a `with` block."""
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        """Closes the runtime on block exit."""
        self.close()


class Session:
    """One live Slab document driven over the Slab Drive Protocol.

    Create sessions through :meth:`Runtime.new_session` or the module-level
    :func:`open_source` and :func:`open_file` helpers. Every method below is a
    thin, typed wrapper over :meth:`request`, which reaches every SDP method.
    """

    __slots__ = ("_closed", "_handle", "_next_id", "_owned_runtime", "_runtime")

    def __init__(self, runtime: Runtime, handle: int, *, owned_runtime: bool = False) -> None:
        """Binds a session handle to its runtime.

        Args:
            runtime: Runtime that owns the WebAssembly instance.
            handle: Nonzero handle from the ABI.
            owned_runtime: Whether :meth:`close` should also close the runtime.
                The module-level convenience constructors set this.
        """
        self._runtime = runtime
        self._handle = handle
        self._closed = False
        self._next_id = 0
        self._owned_runtime = owned_runtime

    @property
    def closed(self) -> bool:
        """Whether the session was closed."""
        return self._closed

    @property
    def runtime(self) -> Runtime:
        """Runtime this session lives in."""
        return self._runtime

    def request(self, method: str, params: Mapping[str, Any] | None = None) -> Any:
        """Sends one SDP request and returns its `result` value.

        Args:
            method: SDP method name, for example `scene.tree`.
            params: Parameter object, or `None` for methods that take none.

        Returns:
            The decoded `result` value; usually a dict.

        Raises:
            ProtocolError: The session answered with an `error` object.
            SlabError: The session is closed or the response was not valid SDP.
        """
        if self._closed:
            raise SlabError("the Slab session is closed")
        self._next_id += 1
        body: dict[str, Any] = {"id": self._next_id, "method": method}
        if params:
            body["params"] = dict(params)
        line = json.dumps(body, separators=(",", ":"), ensure_ascii=False)
        raw = self._runtime._request(self._handle, line)
        try:
            response = json.loads(raw)
        except json.JSONDecodeError as err:
            raise SlabError(f"{method}: response was not JSON: {raw[:200]}") from err
        if not isinstance(response, dict):
            raise SlabError(f"{method}: response was not a JSON object")
        error = response.get("error")
        if isinstance(error, dict):
            raise ProtocolError(
                int(error.get("code", 0)), str(error.get("message", "")), method
            )
        if "result" not in response:
            raise SlabError(f"{method}: response carried neither result nor error")
        return response["result"]

    def open(self, source: str, name: str = "<source>") -> LoadResult:
        """Compiles inline source and makes it the live document.

        This is the on-the-fly parsing path. It never touches a filesystem, and
        a compile failure comes back as :class:`LoadResult` with `ok=False`
        while the previous document keeps running.

        Args:
            source: Full `.slab` source text.
            name: Display name used in diagnostics.
        """
        return LoadResult.decode(self.request("doc.open", {"source": source, "name": name}))

    def open_file(self, path: str | os.PathLike[str]) -> LoadResult:
        """Reads a `.slab` file in Python and compiles it with :meth:`open`.

        Raises:
            OSError: The file could not be read.
        """
        text = Path(path).read_text(encoding="utf-8")
        return self.open(text, str(path))

    def open_slir(self, slir: bytes, name: str = "<slir>") -> LoadResult:
        """Installs precompiled SLIR bytes and makes them the live document.

        This skips the compiler entirely, so it is the fast path for documents
        that were built ahead of time with `slab build`. The result shape and
        the failure semantics match :meth:`open`: a rejected payload comes back
        as :class:`LoadResult` with `ok=False` while the previous document
        keeps running.

        Args:
            slir: Raw SLIR bytes; the wire form is base64 and this method
                encodes them.
            name: Display name used in diagnostics.

        Raises:
            ProtocolError: The payload was not decodable as base64.
        """
        encoded = base64.b64encode(bytes(slir)).decode("ascii")
        return LoadResult.decode(
            self.request("doc.open_slir", {"slir": encoded, "name": name})
        )

    def set_env(self, env: EnvSpec) -> dict[str, Any]:
        """Merges `env` into the environment and returns the merged result."""
        return self.request("env.set", env.params())

    def set_env_cells(
        self, cols: int, rows: int, *, dark: bool = False, coarse: bool = False
    ) -> dict[str, Any]:
        """Sizes the document for a `cols` by `rows` terminal grid."""
        return self.set_env(EnvSpec.for_cells(cols, rows, dark=dark, coarse=coarse))

    def get_env(self) -> dict[str, Any]:
        """Returns the current environment, matching `env.get`."""
        return self.request("env.get")

    def render_cells(self, *, plain: bool = False, caret: bool = True) -> Cells:
        """Renders the document as a terminal cell grid.

        Args:
            plain: `True` for bare text, `False` for truecolor ANSI.
            caret: Whether to paint the text caret.
        """
        return Cells.decode(self.request("render.cells", {"plain": plain, "caret": caret}))

    def advance(self, ms: float) -> float:
        """Advances the virtual motion clock and returns the new time."""
        if ms < 0:
            raise ValueError("clock.advance requires a nonnegative duration")
        return float(self.request("clock.advance", {"ms": float(ms)}).get("t", 0.0))

    def time(self) -> float:
        """Returns the current virtual clock value in milliseconds."""
        return float(self.request("clock.get").get("t", 0.0))

    def key(self, key: str, mods: Iterable[str] | None = None) -> Effects:
        """Dispatches one key-down event.

        Args:
            key: Key name such as `Enter`, `ArrowDown`, `F3`, or a literal
                printable character.
            mods: Any of `shift`, `alt`, `ctrl`, `meta`.
        """
        return Effects.decode(self.request("input.key", {"key": key, "mods": _mods(mods)}))

    def text(self, text: str) -> Effects:
        """Dispatches typed text into the focused editor."""
        return Effects.decode(self.request("input.text", {"text": text}))

    def paste(self, text: str) -> Effects:
        """Dispatches a paste of `text` into the focused editor."""
        return Effects.decode(self.request("input.paste", {"text": text}))

    def pointer(
        self,
        kind: str,
        x: float,
        y: float,
        *,
        button: int = 0,
        clicks: int = 1,
        mods: Iterable[str] | None = None,
    ) -> Effects:
        """Dispatches one pointer event in Slab layout units.

        Args:
            kind: `move`, `down`, or `up`.
            x: Horizontal position in layout units.
            y: Vertical position in layout units.
            button: `0` left, `1` middle, `2` right.
            clicks: Consecutive click count for a `down`.
            mods: Modifier names held during the event.
        """
        if kind not in ("move", "down", "up"):
            raise ValueError(f"pointer kind must be 'move', 'down', or 'up', not {kind!r}")
        return Effects.decode(
            self.request(
                "input.pointer",
                {
                    "type": kind,
                    "x": float(x),
                    "y": float(y),
                    "button": int(button),
                    "clicks": int(clicks),
                    "mods": _mods(mods),
                },
            )
        )

    def click(
        self,
        x: float | None = None,
        y: float | None = None,
        *,
        key: str | None = None,
        button: int = 0,
        clicks: int = 1,
        mods: Iterable[str] | None = None,
    ) -> Effects:
        """Moves, presses, and releases at a point or on a keyed node.

        Pass either `x` and `y` in layout units, or `key` naming a node. The
        merged effects of all three dispatches come back as one value.
        """
        params: dict[str, Any] = {
            "button": int(button),
            "clicks": int(clicks),
            "mods": _mods(mods),
        }
        if key is not None:
            if x is not None or y is not None:
                raise ValueError("click takes either a key or a point, not both")
            params["key"] = key
        else:
            if x is None or y is None:
                raise ValueError("click needs a key or both x and y")
            params["x"] = float(x)
            params["y"] = float(y)
        return Effects.decode(self.request("input.click", params))

    def wheel(
        self,
        x: float,
        y: float,
        dy: float,
        *,
        dx: float = 0.0,
        mods: Iterable[str] | None = None,
    ) -> Effects:
        """Dispatches one wheel event; positive `dy` scrolls the content down."""
        return Effects.decode(
            self.request(
                "input.wheel",
                {
                    "x": float(x),
                    "y": float(y),
                    "dx": float(dx),
                    "dy": float(dy),
                    "mods": _mods(mods),
                },
            )
        )

    def set_param(self, name: str, value: Any) -> None:
        """Sets one declared parameter."""
        self.request("param.set", {"name": name, "value": value})

    def get_param(self, name: str) -> Any:
        """Returns the live value of one declared parameter."""
        return self.request("param.get", {"name": name}).get("value")

    def info(self) -> DocInfo:
        """Returns the document's parameters, themes, holes, signals, and env."""
        return DocInfo.decode(self.request("doc.info"))

    def scene_tree(self) -> list[dict[str, Any]]:
        """Returns the flat pre-order scene entries from `scene.tree`."""
        result = self.request("scene.tree")
        entries = result.get("nodes", result.get("entries", []))
        return [item for item in entries if isinstance(item, dict)]

    def find_text(self, text: str) -> list[dict[str, Any]]:
        """Returns scene-ordered, case-sensitive text matches."""
        result = self.request("scene.find", {"text": text})
        return [item for item in result.get("matches", []) if isinstance(item, dict)]

    def quit(self) -> None:
        """Ends the session with `protocol.quit`.

        The handle stays valid and reports `has_quit`; use :meth:`close` to
        release it.
        """
        self.request("protocol.quit")

    @property
    def has_quit(self) -> bool:
        """Whether the session already ended through `protocol.quit`."""
        if self._closed:
            return True
        return self._runtime._quit(self._handle)

    def _own_runtime(self) -> None:
        """Transfers runtime ownership to this session, so closing cascades."""
        self._owned_runtime = True

    def close(self) -> None:
        """Releases the session handle; closing twice is a no-op.

        A session created by :func:`open_source` or :func:`open_file` also
        closes the runtime it owns.
        """
        if self._closed:
            return
        self._closed = True
        self._runtime._free(self._handle)
        if self._owned_runtime:
            self._runtime.close()

    def __enter__(self) -> Session:
        """Returns the session for use in a `with` block."""
        return self

    def __exit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        tb: TracebackType | None,
    ) -> None:
        """Closes the session on block exit."""
        self.close()


def open_source(source: str, name: str = "<source>") -> Session:
    """Compiles inline source into a ready session that owns its runtime.

    Args:
        source: Full `.slab` source text.
        name: Display name used in diagnostics.

    Returns:
        A session with the document loaded. Closing it closes the runtime.

    Raises:
        CompileError: The source did not compile; the error carries `diags`.
    """
    runtime = Runtime()
    try:
        session = runtime.open_source(source, name)
    except BaseException:
        runtime.close()
        raise
    session._own_runtime()
    return session


def open_file(path: str | os.PathLike[str]) -> Session:
    """Reads a `.slab` file and compiles it into a ready session.

    The file is read in Python because the WebAssembly module has no
    filesystem; the text goes to the session through `doc.open`.

    Args:
        path: Path to a `.slab` source file.

    Returns:
        A session with the document loaded. Closing it closes the runtime.

    Raises:
        CompileError: The file did not compile; the error carries `diags`.
        OSError: The file could not be read.
    """
    runtime = Runtime()
    try:
        session = runtime.open_file(path)
    except BaseException:
        runtime.close()
        raise
    session._own_runtime()
    return session


def open_slir(slir: bytes, name: str = "<slir>") -> Session:
    """Installs precompiled SLIR into a ready session that owns its runtime.

    This is the ahead-of-time counterpart to :func:`open_source`: the bytes
    come from `slab build`, so the compiler never runs.

    Args:
        slir: Raw SLIR bytes.
        name: Display name used in diagnostics.

    Returns:
        A session with the document loaded. Closing it closes the runtime.

    Raises:
        CompileError: The payload was rejected; the error carries `diags`.
        ProtocolError: The payload was not decodable as base64.
    """
    runtime = Runtime()
    try:
        session = runtime.open_slir(slir, name)
    except BaseException:
        runtime.close()
        raise
    session._own_runtime()
    return session
