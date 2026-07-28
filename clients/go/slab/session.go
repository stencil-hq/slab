package slab

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
)

// ErrSessionClosed is returned by a request on a session that was closed.
var ErrSessionClosed = errors.New("slab: session is closed")

// ErrRuntimeClosed is returned once the owning runtime has been closed.
var ErrRuntimeClosed = errors.New("slab: runtime is closed")

// ProtocolError is an SDP `error` object returned instead of a result.
//
// Codes follow the protocol: -32700 invalid JSON, -32600 invalid request,
// -32601 unknown method, -32602 invalid parameters, and -32000 for document,
// key, parameter, theme, render, or filesystem failures.
type ProtocolError struct {
	// Code is the numeric SDP error code.
	Code int `json:"code"`
	// Message is the human-readable failure description.
	Message string `json:"message"`
}

// Error implements the error interface.
func (e *ProtocolError) Error() string {
	return fmt.Sprintf("slab: sdp error %d: %s", e.Code, e.Message)
}

// Session is one live SDP session: at most one document plus its kernel state.
//
// Create it with [Runtime.NewSession] and release it with [Session.Close].
type Session struct {
	runtime *Runtime
	handle  uint32
	closed  bool
}

// wireRequest is one SDP request line.
type wireRequest struct {
	ID     uint64 `json:"id"`
	Method string `json:"method"`
	Params any    `json:"params,omitempty"`
}

// wireResponse is one SDP response line.
type wireResponse struct {
	Result json.RawMessage `json:"result"`
	Error  *ProtocolError  `json:"error"`
}

// NewSession creates a session with no document loaded.
//
// The session starts in the default 800x600 `gpu` environment; a terminal host
// follows up with [Session.SetEnvCells].
func (r *Runtime) NewSession(ctx context.Context) (*Session, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.module == nil {
		return nil, ErrRuntimeClosed
	}
	handle, err := call1(ctx, r.sessionNew)
	if err != nil {
		return nil, err
	}
	if uint32(handle) == 0 {
		return nil, errors.New("slab: runtime could not create a session")
	}
	return &Session{runtime: r, handle: uint32(handle)}, nil
}

// Close frees the session handle and its document. Closing twice is a no-op.
func (s *Session) Close(ctx context.Context) error {
	s.runtime.mu.Lock()
	defer s.runtime.mu.Unlock()
	if s.closed {
		return nil
	}
	s.closed = true
	if s.runtime.module == nil {
		return nil
	}
	if _, err := s.runtime.sessionFree.Call(ctx, uint64(s.handle)); err != nil {
		return fmt.Errorf("slab: free session: %w", err)
	}
	return nil
}

// Ended reports whether the session has been ended through `protocol.quit`.
func (s *Session) Ended(ctx context.Context) (bool, error) {
	s.runtime.mu.Lock()
	defer s.runtime.mu.Unlock()
	if s.closed {
		return true, nil
	}
	if s.runtime.module == nil {
		return false, ErrRuntimeClosed
	}
	ended, err := call1(ctx, s.runtime.sessionQuit, uint64(s.handle))
	if err != nil {
		return false, err
	}
	return ended == 1, nil
}

// Request applies one SDP method and returns its raw `result` payload.
//
// This is the generic escape hatch: every method in the SDP table is reachable
// through it. Pass nil params for methods that take none. A protocol failure is
// returned as a [*ProtocolError].
func (s *Session) Request(ctx context.Context, method string, params any) (json.RawMessage, error) {
	s.runtime.mu.Lock()
	defer s.runtime.mu.Unlock()
	if s.closed {
		return nil, ErrSessionClosed
	}
	return s.runtime.requestLocked(ctx, s.handle, method, params)
}

// requestLocked marshals, dispatches, and decodes one SDP round trip.
func (r *Runtime) requestLocked(ctx context.Context, handle uint32, method string, params any) (json.RawMessage, error) {
	r.nextID++
	line, err := json.Marshal(wireRequest{ID: r.nextID, Method: method, Params: params})
	if err != nil {
		return nil, fmt.Errorf("slab: encode %s request: %w", method, err)
	}
	raw, err := r.rawRequestLocked(ctx, handle, line)
	if err != nil {
		return nil, err
	}
	var response wireResponse
	if err := json.Unmarshal(raw, &response); err != nil {
		return nil, fmt.Errorf("slab: decode %s response: %w", method, err)
	}
	if response.Error != nil {
		return nil, response.Error
	}
	return response.Result, nil
}

// requestInto applies one method and decodes its result into out.
func (s *Session) requestInto(ctx context.Context, method string, params any, out any) error {
	result, err := s.Request(ctx, method, params)
	if err != nil {
		return err
	}
	if out == nil {
		return nil
	}
	if err := json.Unmarshal(result, out); err != nil {
		return fmt.Errorf("slab: decode %s result: %w", method, err)
	}
	return nil
}
