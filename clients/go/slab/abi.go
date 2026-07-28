// Package slab embeds the Slab WebAssembly runtime and speaks the Slab Drive
// Protocol (SDP) to it.
//
// The embedded module carries the whole Slab toolchain: the compiler, the
// kernel, and the SDP session layer. A host compiles `.slab` source on the fly
// with [Session.Open] or [Session.OpenFile], feeds input, and reads back
// terminal cells or scene JSON. Nothing in this package reimplements layout,
// hit testing, focus, or editing; the kernel inside the module owns all of it.
//
// Typical use:
//
//	rt, err := slab.NewRuntime(ctx)
//	defer rt.Close(ctx)
//	sess, err := rt.NewSession(ctx)
//	defer sess.Close(ctx)
//	sess.OpenFile(ctx, "examples/10-settings.slab")
//	sess.SetEnvCells(ctx, 80, 24, true, false)
//	cells, err := sess.Cells(ctx, slab.CellsOptions{Caret: true})
//
// A [Runtime] compiles the module once; sessions created from it are cheap.
// The WebAssembly store is not reentrant, so every call funnels through one
// runtime mutex. Sessions of the same runtime are therefore safe to use from
// several goroutines, but calls serialize.
package slab

import (
	"bytes"
	"compress/gzip"
	"context"
	_ "embed"
	"errors"
	"fmt"
	"io"
	"sync"

	"github.com/tetratelabs/wazero"
	"github.com/tetratelabs/wazero/api"
)

// ABIVersion is the C ABI revision this client implements.
//
// [NewRuntime] refuses a module that reports a different number.
const ABIVersion = 1

// headerBytes is the little-endian u32 length prefix on a response block.
const headerBytes = 4

//go:embed slab_abi.wasm.gz
var compressedModule []byte

var (
	moduleOnce  sync.Once
	moduleBytes []byte
	moduleErr   error
)

// abiModule gunzips the embedded module exactly once and caches the result.
func abiModule() ([]byte, error) {
	moduleOnce.Do(func() {
		reader, err := gzip.NewReader(bytes.NewReader(compressedModule))
		if err != nil {
			moduleErr = fmt.Errorf("slab: embedded module is not gzip: %w", err)
			return
		}
		defer reader.Close()
		decoded, err := io.ReadAll(reader)
		if err != nil {
			moduleErr = fmt.Errorf("slab: embedded module is truncated: %w", err)
			return
		}
		moduleBytes = decoded
	})
	return moduleBytes, moduleErr
}

// Runtime owns one compiled and instantiated copy of the Slab ABI module.
//
// Create it with [NewRuntime] and release it with [Runtime.Close]. All sessions
// created from a runtime share its WebAssembly memory and its lock.
type Runtime struct {
	mu      sync.Mutex
	runtime wazero.Runtime
	module  api.Module
	memory  api.Memory

	alloc       api.Function
	free        api.Function
	sessionNew  api.Function
	sessionFree api.Function
	sessionQuit api.Function
	request     api.Function

	nextID uint64
}

// NewRuntime compiles the embedded module and verifies its ABI version.
//
// The module has zero imports, so no WASI or host functions are provided.
func NewRuntime(ctx context.Context) (*Runtime, error) {
	wasm, err := abiModule()
	if err != nil {
		return nil, err
	}
	wazeroRuntime := wazero.NewRuntime(ctx)
	compiled, err := wazeroRuntime.CompileModule(ctx, wasm)
	if err != nil {
		_ = wazeroRuntime.Close(ctx)
		return nil, fmt.Errorf("slab: compile module: %w", err)
	}
	config := wazero.NewModuleConfig().WithName("slab_abi").WithStartFunctions()
	module, err := wazeroRuntime.InstantiateModule(ctx, compiled, config)
	if err != nil {
		_ = wazeroRuntime.Close(ctx)
		return nil, fmt.Errorf("slab: instantiate module: %w", err)
	}
	runtime := &Runtime{runtime: wazeroRuntime, module: module, memory: module.Memory()}
	if runtime.memory == nil {
		_ = wazeroRuntime.Close(ctx)
		return nil, errors.New("slab: module exports no memory")
	}
	exports := []struct {
		name string
		into *api.Function
	}{
		{"slab_alloc", &runtime.alloc},
		{"slab_free", &runtime.free},
		{"slab_session_new", &runtime.sessionNew},
		{"slab_session_free", &runtime.sessionFree},
		{"slab_session_quit", &runtime.sessionQuit},
		{"slab_request", &runtime.request},
	}
	for _, export := range exports {
		function := module.ExportedFunction(export.name)
		if function == nil {
			_ = wazeroRuntime.Close(ctx)
			return nil, fmt.Errorf("slab: module exports no %s", export.name)
		}
		*export.into = function
	}
	version, err := call1(ctx, module.ExportedFunction("slab_abi_version"))
	if err != nil {
		_ = wazeroRuntime.Close(ctx)
		return nil, err
	}
	if uint32(version) != ABIVersion {
		_ = wazeroRuntime.Close(ctx)
		return nil, fmt.Errorf("slab: module reports ABI version %d, want %d", uint32(version), ABIVersion)
	}
	return runtime, nil
}

// Close releases the module, its memory, and every session still open on it.
func (r *Runtime) Close(ctx context.Context) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.runtime == nil {
		return nil
	}
	err := r.runtime.Close(ctx)
	r.runtime = nil
	r.module = nil
	r.memory = nil
	if err != nil {
		return fmt.Errorf("slab: close runtime: %w", err)
	}
	return nil
}

// call1 invokes a single-result export and returns its raw result word.
func call1(ctx context.Context, function api.Function, args ...uint64) (uint64, error) {
	if function == nil {
		return 0, errors.New("slab: missing export")
	}
	results, err := function.Call(ctx, args...)
	if err != nil {
		return 0, fmt.Errorf("slab: wasm trap: %w", err)
	}
	if len(results) != 1 {
		return 0, fmt.Errorf("slab: export returned %d results, want 1", len(results))
	}
	return results[0], nil
}

// allocLocked reserves a block of linear memory. The caller holds r.mu.
func (r *Runtime) allocLocked(ctx context.Context, size uint32) (uint32, error) {
	if size == 0 {
		return 0, errors.New("slab: cannot allocate an empty block")
	}
	pointer, err := call1(ctx, r.alloc, uint64(size))
	if err != nil {
		return 0, err
	}
	if uint32(pointer) == 0 {
		return 0, fmt.Errorf("slab: allocation of %d bytes failed", size)
	}
	return uint32(pointer), nil
}

// freeLocked releases a block previously allocated at the same length.
func (r *Runtime) freeLocked(ctx context.Context, pointer, size uint32) {
	if pointer == 0 {
		return
	}
	// A free cannot fail in a way the host can act on; the block is gone either
	// way and reporting it would mask the caller's real error.
	_, _ = r.free.Call(ctx, uint64(pointer), uint64(size))
}

// rawRequestLocked applies one SDP line and returns the response bytes.
//
// The caller holds r.mu. The response block is read out of linear memory and
// copied, because any later allocation may move the memory view.
func (r *Runtime) rawRequestLocked(ctx context.Context, handle uint32, line []byte) ([]byte, error) {
	if r.module == nil {
		return nil, ErrRuntimeClosed
	}
	size := uint32(len(line))
	pointer, err := r.allocLocked(ctx, size)
	if err != nil {
		return nil, err
	}
	defer r.freeLocked(ctx, pointer, size)
	if !r.memory.Write(pointer, line) {
		return nil, errors.New("slab: request body does not fit in linear memory")
	}
	block, err := call1(ctx, r.request, uint64(handle), uint64(pointer), uint64(size))
	if err != nil {
		return nil, err
	}
	blockPointer := uint32(block)
	if blockPointer == 0 {
		return nil, errors.New("slab: runtime could not allocate a response")
	}
	header, ok := r.memory.Read(blockPointer, headerBytes)
	if !ok {
		return nil, errors.New("slab: response header is out of bounds")
	}
	count := uint32(header[0]) | uint32(header[1])<<8 | uint32(header[2])<<16 | uint32(header[3])<<24
	payload, ok := r.memory.Read(blockPointer+headerBytes, count)
	if !ok {
		r.freeLocked(ctx, blockPointer, headerBytes+count)
		return nil, errors.New("slab: response payload is out of bounds")
	}
	out := make([]byte, count)
	copy(out, payload)
	r.freeLocked(ctx, blockPointer, headerBytes+count)
	return out, nil
}
