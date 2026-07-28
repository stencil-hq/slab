"""Low-level binding to the six exports of the Slab WebAssembly ABI.

The module `slab_abi.wasm.gz` ships inside this package. It is a
`wasm32-unknown-unknown` module with zero imports, so it needs no WASI and no
host functions: the whole Slab compiler, kernel, and Slab Drive Protocol (SDP)
session layer live behind six exported symbols.

The calling convention is described in `crates/slab-abi/src/lib.rs`:

* `slab_abi_version() -> u32` must equal :data:`ABI_VERSION`.
* `slab_alloc(len) -> ptr` returns a 4-byte aligned block, `0` on failure.
* `slab_free(ptr, len)` releases a block.
* `slab_session_new() -> u32` returns a nonzero session handle.
* `slab_session_free(handle)` destroys a session.
* `slab_session_quit(handle) -> u32` reports `1` after `protocol.quit`.
* `slab_request(handle, ptr, len) -> ptr` answers with one block holding a
  little-endian `u32` byte count followed by that many UTF-8 bytes. The host
  releases it with `slab_free(ptr, 4 + n)`.

Nothing in this module knows about SDP method names. :mod:`slab` builds the
typed API on top of :class:`Abi`.
"""

from __future__ import annotations

import gzip
import threading
from importlib import resources
from typing import Final

from wasmtime import Engine, Func, Instance, Memory, Module, Store

__all__ = [
    "ABI_VERSION",
    "Abi",
    "AbiError",
    "compile_module",
    "wasm_bytes",
]

#: ABI revision this binding implements; the module must report the same value.
ABI_VERSION: Final[int] = 1

#: Name of the gzipped WebAssembly module packaged next to this file.
_ARTIFACT: Final[str] = "slab_abi.wasm.gz"

#: Byte count of the little-endian length prefix on a response block.
_HEADER: Final[int] = 4


class AbiError(RuntimeError):
    """Raised when the WebAssembly module cannot be loaded or driven.

    This covers a version mismatch, a failed allocation, a missing export, and
    use of a runtime that was already closed. Protocol-level failures are never
    reported here; they arrive as ordinary SDP `error` objects.
    """


_BYTES_LOCK = threading.Lock()
_BYTES_CACHE: bytes | None = None

_DEFAULT_LOCK = threading.Lock()
_DEFAULT: tuple[Engine, Module] | None = None


def wasm_bytes() -> bytes:
    """Returns the decompressed WebAssembly module, gunzipping it once.

    The gzipped artifact is read from package data, so the function works from
    a wheel, an editable install, and a source checkout alike.
    """
    global _BYTES_CACHE
    with _BYTES_LOCK:
        if _BYTES_CACHE is None:
            packed = resources.files(__package__).joinpath(_ARTIFACT).read_bytes()
            _BYTES_CACHE = gzip.decompress(packed)
        return _BYTES_CACHE


def default_engine_module() -> tuple[Engine, Module]:
    """Returns the process-wide engine and its compiled module.

    Compilation dominates start-up cost and the module is several megabytes,
    so every runtime that does not bring its own engine shares this one pair
    for the life of the process.
    """
    global _DEFAULT
    with _DEFAULT_LOCK:
        if _DEFAULT is None:
            engine = Engine()
            _DEFAULT = (engine, Module(engine, wasm_bytes()))
        return _DEFAULT


def compile_module(engine: Engine) -> Module:
    """Compiles the packaged module for a caller-supplied engine.

    Nothing is cached: the caller owns the engine's lifetime, and a process
    that creates many runtimes should either reuse one :class:`Runtime` or
    omit the engine entirely to share the default compiled module.
    """
    return Module(engine, wasm_bytes())


class Abi:
    """One instantiated Slab ABI module with its own linear memory.

    An instance is cheap to drive but is not thread-safe on its own; callers
    that share one across threads must serialize their calls. Sessions created
    through :meth:`session_new` all live inside this instance's memory.
    """

    __slots__ = (
        "_alloc",
        "_closed",
        "_free",
        "_memory",
        "_request",
        "_session_free",
        "_session_new",
        "_session_quit",
        "_store",
        "engine",
        "module",
    )

    def __init__(self, engine: Engine | None = None) -> None:
        """Instantiates the packaged module and verifies its ABI version.

        Args:
            engine: Optional :class:`wasmtime.Engine` the caller owns. Omitting
                it shares the process-wide engine and its compiled module.

        Raises:
            AbiError: The module is missing an export or reports a version
                other than :data:`ABI_VERSION`.
        """
        self._closed = False
        if engine is None:
            self.engine, self.module = default_engine_module()
        else:
            self.engine = engine
            self.module = compile_module(engine)
        self._store = Store(self.engine)
        instance = Instance(self._store, self.module, [])
        exports = instance.exports(self._store)

        def export(name: str) -> Func:
            item = exports.get(name)
            if not isinstance(item, Func):
                raise AbiError(f"slab_abi.wasm is missing the '{name}' function export")
            return item

        memory = exports.get("memory")
        if not isinstance(memory, Memory):
            raise AbiError("slab_abi.wasm is missing the 'memory' export")
        self._memory: Memory = memory
        self._alloc = export("slab_alloc")
        self._free = export("slab_free")
        self._request = export("slab_request")
        self._session_new = export("slab_session_new")
        self._session_free = export("slab_session_free")
        self._session_quit = export("slab_session_quit")

        version = export("slab_abi_version")(self._store)
        if version != ABI_VERSION:
            raise AbiError(
                f"slab_abi.wasm reports ABI version {version}, expected {ABI_VERSION}"
            )

    @property
    def version(self) -> int:
        """ABI revision reported by the module; always :data:`ABI_VERSION`."""
        return ABI_VERSION

    @property
    def closed(self) -> bool:
        """Whether :meth:`close` already dropped the instance."""
        return self._closed

    def _live(self) -> Store:
        """Returns the store, or raises when the instance is already closed."""
        if self._closed:
            raise AbiError("the Slab runtime is closed")
        return self._store

    def session_new(self) -> int:
        """Creates an SDP session and returns its nonzero handle.

        Raises:
            AbiError: The module refused to create a session.
        """
        handle = int(self._session_new(self._live()))
        if handle == 0:
            raise AbiError("slab_session_new returned the invalid handle 0")
        return handle

    def session_free(self, handle: int) -> None:
        """Destroys a session handle; freeing an unknown handle is a no-op."""
        self._session_free(self._live(), handle)

    def session_quit(self, handle: int) -> bool:
        """Reports whether the session already ended through `protocol.quit`."""
        return bool(self._session_quit(self._live(), handle))

    def request(self, handle: int, line: str) -> str:
        """Applies one SDP request line and returns its JSON response.

        The response is always one complete JSON object, including for unknown
        handles and malformed input, so transport failure and protocol failure
        never have to be told apart.

        Args:
            handle: Session handle from :meth:`session_new`.
            line: One single-line SDP request object.

        Returns:
            The response JSON text, without a trailing newline.

        Raises:
            AbiError: Guest memory for the request or response ran out.
        """
        store = self._live()
        body = line.encode("utf-8")
        length = len(body)
        ptr = int(self._alloc(store, length))
        if ptr == 0:
            raise AbiError(f"slab_alloc failed for a {length}-byte request")
        try:
            self._memory.write(store, body, ptr)
            block = int(self._request(store, handle, ptr, length))
        finally:
            self._free(store, ptr, length)
        if block == 0:
            raise AbiError("slab_request could not allocate a response block")
        header = self._memory.read(store, block, block + _HEADER)
        count = int.from_bytes(header, "little")
        try:
            payload = self._memory.read(store, block + _HEADER, block + _HEADER + count)
            return bytes(payload).decode("utf-8")
        finally:
            self._free(store, block, _HEADER + count)

    def close(self) -> None:
        """Marks the instance dead so later calls raise :class:`AbiError`.

        Closing twice is a no-op. The linear memory is reclaimed once the last
        reference to this object goes away.
        """
        self._closed = True
