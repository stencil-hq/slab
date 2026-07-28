package slab

import (
	"context"
	"encoding/base64"
	"fmt"
	"os"
	"strconv"
)

// Diag is one compiler diagnostic reported by a document load.
type Diag struct {
	// Code is the short diagnostic category, such as `ref` or `parse`.
	Code string `json:"code"`
	// Level is `error`, `warn`, or `note`.
	Level string `json:"level"`
	// Line is the 1-based source line.
	Line int `json:"line"`
	// Msg is the human-readable diagnostic text.
	Msg string `json:"msg"`
}

// CompileError reports a document that failed to compile.
//
// A compile failure is data, not a transport failure: the previous document
// keeps running.
type CompileError struct {
	// Name is the document label that was compiled.
	Name string
	// Diags lists every diagnostic the compiler produced.
	Diags []Diag
}

// Error implements the error interface.
func (e *CompileError) Error() string {
	for _, diag := range e.Diags {
		if diag.Level == "error" {
			return fmt.Sprintf("slab: compile %s failed: line %d: %s (%s)", e.name(), diag.Line, diag.Msg, diag.Code)
		}
	}
	return fmt.Sprintf("slab: compile %s failed", e.name())
}

// name renders the document label for an error message.
func (e *CompileError) name() string {
	if e.Name == "" {
		return "inline source"
	}
	return strconv.Quote(e.Name)
}

// Open compiles inline source and makes it the live document.
//
// This never touches a filesystem: the module has none. Name is the label
// diagnostics report; pass an empty string to keep the default. A document
// that does not compile returns a [*CompileError].
func (s *Session) Open(ctx context.Context, source, name string) error {
	params := map[string]any{"source": source}
	if name != "" {
		params["name"] = name
	}
	return s.load(ctx, "doc.open", params, name)
}

// OpenFile reads path on the host and opens its contents as the document.
//
// The path becomes the document name, so diagnostics point back at the file.
func (s *Session) OpenFile(ctx context.Context, path string) error {
	source, err := os.ReadFile(path)
	if err != nil {
		return fmt.Errorf("slab: read %s: %w", path, err)
	}
	return s.Open(ctx, string(source), path)
}

// OpenSLIR installs precompiled SLIR bytes as the live document.
//
// The compiler never runs, so this is the fast path for generated Go modules
// that embed the SLIR produced by `slab build FILE -o OUT.slir`. Name is the
// label diagnostics report; pass an empty string to keep the default. Bytes
// the kernel rejects come back as a [*CompileError].
func (s *Session) OpenSLIR(ctx context.Context, slir []byte, name string) error {
	params := map[string]any{"slir": base64.StdEncoding.EncodeToString(slir)}
	if name != "" {
		params["name"] = name
	}
	return s.load(ctx, "doc.open_slir", params, name)
}

// load applies one document-load method and turns a failure into an error.
func (s *Session) load(ctx context.Context, method string, params any, name string) error {
	var result struct {
		OK    bool   `json:"ok"`
		Diags []Diag `json:"diags"`
	}
	if err := s.requestInto(ctx, method, params, &result); err != nil {
		return err
	}
	if !result.OK {
		return &CompileError{Name: name, Diags: result.Diags}
	}
	return nil
}

// SetEnv merges env into the session environment.
func (s *Session) SetEnv(ctx context.Context, env EnvSpec) error {
	return s.requestInto(ctx, "env.set", env, nil)
}

// SetEnvCells sizes the document for a terminal grid of cols by rows cells.
//
// The client is set to `tui`, the width to cols*[CellWidth], and the height to
// rows*[CellHeight].
func (s *Session) SetEnvCells(ctx context.Context, cols, rows int, dark, coarse bool) error {
	return s.SetEnv(ctx, EnvSpec{
		Width:  float64(cols) * CellWidth,
		Height: float64(rows) * CellHeight,
		Client: "tui",
		Dark:   dark,
		Coarse: coarse,
	})
}

// Env returns the current environment.
func (s *Session) Env(ctx context.Context) (EnvSpec, error) {
	var env EnvSpec
	err := s.requestInto(ctx, "env.get", nil, &env)
	return env, err
}

// Cells renders the document as a terminal grid.
func (s *Session) Cells(ctx context.Context, opts CellsOptions) (Cells, error) {
	var cells Cells
	err := s.requestInto(ctx, "render.cells", opts, &cells)
	return cells, err
}

// Advance moves the virtual motion clock forward by ms milliseconds.
func (s *Session) Advance(ctx context.Context, ms float64) error {
	if ms < 0 {
		return fmt.Errorf("slab: cannot advance the clock by %g ms", ms)
	}
	return s.requestInto(ctx, "clock.advance", map[string]any{"ms": ms}, nil)
}

// Info returns the loaded document description.
func (s *Session) Info(ctx context.Context) (DocInfo, error) {
	var info DocInfo
	err := s.requestInto(ctx, "doc.info", nil, &info)
	return info, err
}

// Quit ends the session at the protocol level.
//
// The handle stays valid for [Session.Ended] and [Session.Close]; further
// requests are refused by the module.
func (s *Session) Quit(ctx context.Context) error {
	return s.requestInto(ctx, "protocol.quit", nil, nil)
}

// dispatch applies one input method and decodes its effects.
func (s *Session) dispatch(ctx context.Context, method string, params any) (Effects, error) {
	var result inputResult
	if err := s.requestInto(ctx, method, params, &result); err != nil {
		return Effects{}, err
	}
	return result.Effects, nil
}

// Key dispatches one key-down event.
//
// Key names are `Tab`, `Enter`, `Backspace`, `Delete`, `Escape`, `Insert`,
// `Home`, `End`, `PageUp`, `PageDown`, `ArrowLeft`, `ArrowRight`, `ArrowUp`,
// `ArrowDown`, `F1` through `F24`, or the literal printable character.
func (s *Session) Key(ctx context.Context, key string, mods ...string) (Effects, error) {
	params := map[string]any{"key": key}
	if len(mods) > 0 {
		params["mods"] = mods
	}
	return s.dispatch(ctx, "input.key", params)
}

// Text dispatches one text-input event.
func (s *Session) Text(ctx context.Context, text string) (Effects, error) {
	return s.dispatch(ctx, "input.text", map[string]any{"text": text})
}

// Paste dispatches one paste event.
func (s *Session) Paste(ctx context.Context, text string) (Effects, error) {
	return s.dispatch(ctx, "input.paste", map[string]any{"text": text})
}

// PointerOption customizes a pointer or click dispatch.
type PointerOption func(map[string]any)

// WithButton selects the pointer button: [ButtonLeft], [ButtonMiddle], or
// [ButtonRight].
func WithButton(button int) PointerOption {
	return func(params map[string]any) { params["button"] = button }
}

// WithClicks sets the consecutive click count of a press.
func WithClicks(clicks int) PointerOption {
	return func(params map[string]any) { params["clicks"] = clicks }
}

// WithMods sets the held modifiers; see [ModShift], [ModAlt], [ModCtrl], and
// [ModMeta].
func WithMods(mods ...string) PointerOption {
	return func(params map[string]any) {
		if len(mods) > 0 {
			params["mods"] = mods
		}
	}
}

// Pointer dispatches one pointer event of kind `move`, `down`, or `up` at the
// document position (x, y) in layout units.
func (s *Session) Pointer(ctx context.Context, kind string, x, y float64, opts ...PointerOption) (Effects, error) {
	switch kind {
	case "move", "down", "up":
	default:
		return Effects{}, fmt.Errorf("slab: unknown pointer type %q", kind)
	}
	params := map[string]any{"type": kind, "x": x, "y": y}
	for _, opt := range opts {
		opt(params)
	}
	return s.dispatch(ctx, "input.pointer", params)
}

// Wheel dispatches one wheel event at (x, y) with a vertical delta of dy
// layout units; dy is negative when scrolling up.
func (s *Session) Wheel(ctx context.Context, x, y, dy float64, mods ...string) (Effects, error) {
	params := map[string]any{"x": x, "y": y, "dy": dy}
	WithMods(mods...)(params)
	return s.dispatch(ctx, "input.wheel", params)
}

// Click dispatches a move, a press, and a release at (x, y), merging the
// effects of all three into one result.
func (s *Session) Click(ctx context.Context, x, y float64, opts ...PointerOption) (Effects, error) {
	params := map[string]any{"x": x, "y": y}
	for _, opt := range opts {
		opt(params)
	}
	return s.dispatch(ctx, "input.click", params)
}

// ClickKey dispatches a click at the center of the node addressed by key.
func (s *Session) ClickKey(ctx context.Context, key string, opts ...PointerOption) (Effects, error) {
	params := map[string]any{"key": key}
	for _, opt := range opts {
		opt(params)
	}
	return s.dispatch(ctx, "input.click", params)
}
