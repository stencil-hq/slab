package slab

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// inlineDoc is a tiny document with two authored strings and one button.
const inlineDoc = `params {
  title text = "Hello Slab"
}

col pad=8 gap=4 {
  text param.title size=14
  row #go focusable act="go" pad=4 {
    text "Launch" size=12
  }
}
`

// examplePath resolves a repository example relative to this package.
func examplePath(name string) string {
	return filepath.Join("..", "..", "..", "examples", name)
}

// newSession creates a runtime and one session, closing both after the test.
func newSession(t *testing.T) (context.Context, *Session) {
	t.Helper()
	ctx := context.Background()
	runtime, err := NewRuntime(ctx)
	if err != nil {
		t.Fatalf("NewRuntime: %v", err)
	}
	t.Cleanup(func() {
		if err := runtime.Close(ctx); err != nil {
			t.Errorf("Runtime.Close: %v", err)
		}
	})
	session, err := runtime.NewSession(ctx)
	if err != nil {
		t.Fatalf("NewSession: %v", err)
	}
	t.Cleanup(func() {
		if err := session.Close(ctx); err != nil {
			t.Errorf("Session.Close: %v", err)
		}
	})
	return ctx, session
}

func TestOpenInlineRendersAuthoredText(t *testing.T) {
	ctx, session := newSession(t)
	if err := session.Open(ctx, inlineDoc, "inline.slab"); err != nil {
		t.Fatalf("Open: %v", err)
	}
	if err := session.SetEnvCells(ctx, 40, 10, true, false); err != nil {
		t.Fatalf("SetEnvCells: %v", err)
	}
	cells, err := session.Cells(ctx, CellsOptions{Plain: true})
	if err != nil {
		t.Fatalf("Cells: %v", err)
	}
	if cells.Cols != 40 {
		t.Errorf("cols = %d, want 40", cells.Cols)
	}
	if cells.Rows <= 0 || cells.Rows > 10 {
		t.Errorf("rows = %d, want 1..10", cells.Rows)
	}
	for _, want := range []string{"Hello Slab", "Launch"} {
		if !strings.Contains(cells.Text, want) {
			t.Errorf("rendered grid is missing %q:\n%s", want, cells.Text)
		}
	}
}

func TestSetEnvCellsDerivesLayoutSize(t *testing.T) {
	ctx, session := newSession(t)
	if err := session.Open(ctx, inlineDoc, "inline.slab"); err != nil {
		t.Fatalf("Open: %v", err)
	}
	if err := session.SetEnvCells(ctx, 80, 24, true, false); err != nil {
		t.Fatalf("SetEnvCells: %v", err)
	}
	env, err := session.Env(ctx)
	if err != nil {
		t.Fatalf("Env: %v", err)
	}
	if env.Width != 640 || env.Height != 384 {
		t.Errorf("env size = %gx%g, want 640x384", env.Width, env.Height)
	}
	if env.Client != "tui" || !env.Dark {
		t.Errorf("env client/dark = %q/%v, want tui/true", env.Client, env.Dark)
	}
}

func TestStyledCellsCarryAnsi(t *testing.T) {
	ctx, session := newSession(t)
	if err := session.Open(ctx, inlineDoc, "inline.slab"); err != nil {
		t.Fatalf("Open: %v", err)
	}
	if err := session.SetEnvCells(ctx, 40, 10, true, false); err != nil {
		t.Fatalf("SetEnvCells: %v", err)
	}
	styled, err := session.Cells(ctx, CellsOptions{Caret: true})
	if err != nil {
		t.Fatalf("Cells: %v", err)
	}
	if !strings.Contains(styled.Text, "\x1b[") {
		t.Error("styled grid carries no ANSI escapes")
	}
	plain, err := session.Cells(ctx, CellsOptions{Plain: true})
	if err != nil {
		t.Fatalf("Cells: %v", err)
	}
	if strings.Contains(plain.Text, "\x1b[") {
		t.Error("plain grid carries ANSI escapes")
	}
}

func TestClickEmitsAuthoredSignal(t *testing.T) {
	ctx, session := newSession(t)
	if err := session.OpenFile(ctx, examplePath("10-settings.slab")); err != nil {
		t.Fatalf("OpenFile: %v", err)
	}
	if err := session.SetEnvCells(ctx, 80, 24, true, false); err != nil {
		t.Fatalf("SetEnvCells: %v", err)
	}
	effects, err := session.ClickKey(ctx, "#reset")
	if err != nil {
		t.Fatalf("ClickKey: %v", err)
	}
	signal, ok := effects.Signal("reset")
	if !ok {
		t.Fatalf("signals = %+v, want one named reset", effects.Signals)
	}
	if !strings.Contains(signal.Meta.Key, "#reset") {
		t.Errorf("signal key = %q, want it to name the reset node", signal.Meta.Key)
	}
	if !effects.Repaint {
		t.Error("a click on a button did not request a repaint")
	}
}

func TestKeyboardActivationEmitsSignal(t *testing.T) {
	ctx, session := newSession(t)
	if err := session.OpenFile(ctx, examplePath("10-settings.slab")); err != nil {
		t.Fatalf("OpenFile: %v", err)
	}
	if err := session.SetEnvCells(ctx, 80, 24, true, false); err != nil {
		t.Fatalf("SetEnvCells: %v", err)
	}
	if _, err := session.Key(ctx, "Tab"); err != nil {
		t.Fatalf("Key(Tab): %v", err)
	}
	effects, err := session.Key(ctx, "Enter")
	if err != nil {
		t.Fatalf("Key(Enter): %v", err)
	}
	if _, ok := effects.Signal("save"); !ok {
		t.Fatalf("signals = %+v, want one named save", effects.Signals)
	}
	if effects.Focus == "" {
		t.Error("Tab left the document without focus")
	}
}

func TestTypingIntoAFieldChangesText(t *testing.T) {
	ctx, session := newSession(t)
	if err := session.OpenFile(ctx, examplePath("10-settings.slab")); err != nil {
		t.Fatalf("OpenFile: %v", err)
	}
	if err := session.SetEnvCells(ctx, 80, 24, true, false); err != nil {
		t.Fatalf("SetEnvCells: %v", err)
	}
	if _, err := session.ClickKey(ctx, "#field"); err != nil {
		t.Fatalf("ClickKey(#field): %v", err)
	}
	if _, err := session.Text(ctx, "ab"); err != nil {
		t.Fatalf("Text: %v", err)
	}
	var field struct {
		Text string `json:"text"`
	}
	result, err := session.Request(ctx, "field.get", map[string]any{"key": "#field"})
	if err != nil {
		t.Fatalf("field.get: %v", err)
	}
	if err := json.Unmarshal(result, &field); err != nil {
		t.Fatalf("decode field.get: %v", err)
	}
	if field.Text != "ab" {
		t.Errorf("field text = %q, want %q", field.Text, "ab")
	}
}

func TestUnknownMethodIsProtocolError(t *testing.T) {
	ctx, session := newSession(t)
	_, err := session.Request(ctx, "no.such.method", nil)
	var protocolErr *ProtocolError
	if !errors.As(err, &protocolErr) {
		t.Fatalf("err = %v, want a *ProtocolError", err)
	}
	if protocolErr.Code != -32601 {
		t.Errorf("code = %d, want -32601", protocolErr.Code)
	}
	if !strings.Contains(protocolErr.Error(), "no.such.method") {
		t.Errorf("message = %q, want it to name the method", protocolErr.Message)
	}
}

func TestBadParametersAreProtocolErrors(t *testing.T) {
	ctx, session := newSession(t)
	if err := session.Open(ctx, inlineDoc, "inline.slab"); err != nil {
		t.Fatalf("Open: %v", err)
	}
	_, err := session.Request(ctx, "param.set", map[string]any{"name": "nope", "value": "x"})
	var protocolErr *ProtocolError
	if !errors.As(err, &protocolErr) {
		t.Fatalf("err = %v, want a *ProtocolError", err)
	}
}

func TestCompileFailureReportsDiagnostics(t *testing.T) {
	ctx, session := newSession(t)
	err := session.Open(ctx, "row { nope }", "broken.slab")
	var compileErr *CompileError
	if !errors.As(err, &compileErr) {
		t.Fatalf("err = %v, want a *CompileError", err)
	}
	if len(compileErr.Diags) == 0 {
		t.Fatal("compile failure carried no diagnostics")
	}
	if compileErr.Diags[0].Line != 1 || compileErr.Diags[0].Msg == "" {
		t.Errorf("diag = %+v, want line 1 with a message", compileErr.Diags[0])
	}
}

func TestClosedSessionRejectsRequests(t *testing.T) {
	ctx := context.Background()
	runtime, err := NewRuntime(ctx)
	if err != nil {
		t.Fatalf("NewRuntime: %v", err)
	}
	defer runtime.Close(ctx)
	session, err := runtime.NewSession(ctx)
	if err != nil {
		t.Fatalf("NewSession: %v", err)
	}
	handle := session.handle
	if err := session.Close(ctx); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := session.Close(ctx); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if _, err := session.Request(ctx, "protocol.info", nil); !errors.Is(err, ErrSessionClosed) {
		t.Fatalf("err = %v, want ErrSessionClosed", err)
	}

	// The module itself must also refuse the freed handle, not trust the host.
	runtime.mu.Lock()
	_, rawErr := runtime.requestLocked(ctx, handle, "protocol.info", nil)
	runtime.mu.Unlock()
	var protocolErr *ProtocolError
	if !errors.As(rawErr, &protocolErr) {
		t.Fatalf("raw err = %v, want a *ProtocolError", rawErr)
	}
	if protocolErr.Code != -32000 || !strings.Contains(protocolErr.Message, "unknown session handle") {
		t.Errorf("raw error = %+v, want the unknown-handle failure", protocolErr)
	}
}

func TestSessionsAreIndependent(t *testing.T) {
	ctx := context.Background()
	runtime, err := NewRuntime(ctx)
	if err != nil {
		t.Fatalf("NewRuntime: %v", err)
	}
	defer runtime.Close(ctx)
	first, err := runtime.NewSession(ctx)
	if err != nil {
		t.Fatalf("NewSession: %v", err)
	}
	defer first.Close(ctx)
	second, err := runtime.NewSession(ctx)
	if err != nil {
		t.Fatalf("NewSession: %v", err)
	}
	defer second.Close(ctx)
	if err := first.Open(ctx, inlineDoc, "first.slab"); err != nil {
		t.Fatalf("Open: %v", err)
	}
	if err := first.SetEnvCells(ctx, 40, 6, true, false); err != nil {
		t.Fatalf("SetEnvCells: %v", err)
	}
	if err := second.SetEnvCells(ctx, 40, 6, false, false); err != nil {
		t.Fatalf("SetEnvCells: %v", err)
	}
	firstEnv, err := first.Env(ctx)
	if err != nil {
		t.Fatalf("Env: %v", err)
	}
	secondEnv, err := second.Env(ctx)
	if err != nil {
		t.Fatalf("Env: %v", err)
	}
	if !firstEnv.Dark || secondEnv.Dark {
		t.Errorf("dark = %v/%v, want true/false", firstEnv.Dark, secondEnv.Dark)
	}
	info, err := second.Info(ctx)
	if err == nil && len(info.Params) != 0 {
		t.Errorf("second session sees params %+v from the first document", info.Params)
	}
}

func TestInfoDescribesLoadedDocument(t *testing.T) {
	ctx, session := newSession(t)
	if err := session.OpenFile(ctx, examplePath("10-settings.slab")); err != nil {
		t.Fatalf("OpenFile: %v", err)
	}
	info, err := session.Info(ctx)
	if err != nil {
		t.Fatalf("Info: %v", err)
	}
	if !contains(info.Signals, "reset") || !contains(info.Signals, "save") {
		t.Errorf("signals = %v, want save and reset", info.Signals)
	}
	names := make([]string, 0, len(info.Params))
	for _, param := range info.Params {
		names = append(names, param.Name)
	}
	if !contains(names, "title") {
		t.Errorf("params = %v, want a title parameter", names)
	}
}

func TestAdvanceMovesTheClock(t *testing.T) {
	ctx, session := newSession(t)
	if err := session.Open(ctx, inlineDoc, "inline.slab"); err != nil {
		t.Fatalf("Open: %v", err)
	}
	if err := session.Advance(ctx, 250); err != nil {
		t.Fatalf("Advance: %v", err)
	}
	info, err := session.Info(ctx)
	if err != nil {
		t.Fatalf("Info: %v", err)
	}
	if info.T != 250 {
		t.Errorf("clock = %g, want 250", info.T)
	}
	if err := session.Advance(ctx, -1); err == nil {
		t.Error("Advance accepted a negative duration")
	}
}

func TestQuitEndsTheSession(t *testing.T) {
	ctx, session := newSession(t)
	ended, err := session.Ended(ctx)
	if err != nil {
		t.Fatalf("Ended: %v", err)
	}
	if ended {
		t.Fatal("a fresh session reports itself ended")
	}
	if err := session.Quit(ctx); err != nil {
		t.Fatalf("Quit: %v", err)
	}
	ended, err = session.Ended(ctx)
	if err != nil {
		t.Fatalf("Ended: %v", err)
	}
	if !ended {
		t.Error("session did not report the quit")
	}
}

func TestPointerOptionsReachTheKernel(t *testing.T) {
	ctx, session := newSession(t)
	if err := session.OpenFile(ctx, examplePath("10-settings.slab")); err != nil {
		t.Fatalf("OpenFile: %v", err)
	}
	if err := session.SetEnvCells(ctx, 80, 24, true, false); err != nil {
		t.Fatalf("SetEnvCells: %v", err)
	}
	effects, err := session.ClickKey(ctx, "#reset", WithClicks(2), WithMods(ModShift))
	if err != nil {
		t.Fatalf("ClickKey: %v", err)
	}
	signal, ok := effects.Signal("reset")
	if !ok {
		t.Fatalf("signals = %+v, want one named reset", effects.Signals)
	}
	if signal.Meta.Mods == 0 {
		t.Error("the shift modifier never reached the kernel")
	}
	if _, err := session.Pointer(ctx, "sideways", 0, 0); err == nil {
		t.Error("Pointer accepted an unknown event type")
	}
}

func TestUnknownRuntimeIsRefusedAfterClose(t *testing.T) {
	ctx := context.Background()
	runtime, err := NewRuntime(ctx)
	if err != nil {
		t.Fatalf("NewRuntime: %v", err)
	}
	session, err := runtime.NewSession(ctx)
	if err != nil {
		t.Fatalf("NewSession: %v", err)
	}
	if err := runtime.Close(ctx); err != nil {
		t.Fatalf("Close: %v", err)
	}
	if err := runtime.Close(ctx); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	if _, err := session.Request(ctx, "protocol.info", nil); !errors.Is(err, ErrRuntimeClosed) {
		t.Fatalf("err = %v, want ErrRuntimeClosed", err)
	}
	if _, err := runtime.NewSession(ctx); !errors.Is(err, ErrRuntimeClosed) {
		t.Fatalf("err = %v, want ErrRuntimeClosed", err)
	}
}

// contains reports whether values holds want.
func contains(values []string, want string) bool {
	for _, value := range values {
		if value == want {
			return true
		}
	}
	return false
}

// buildSLIR compiles a repository example to SLIR bytes with the native CLI.
//
// The test is skipped when the toolchain is unavailable, so `go test ./...`
// still works outside a full Rust checkout.
func buildSLIR(t *testing.T, example string) []byte {
	t.Helper()
	root, err := filepath.Abs(filepath.Join("..", "..", ".."))
	if err != nil {
		t.Fatalf("resolve repository root: %v", err)
	}
	if _, err := os.Stat(filepath.Join(root, "Cargo.toml")); err != nil {
		t.Skip("no Rust checkout: cannot build SLIR")
	}
	cargo, err := exec.LookPath("cargo")
	if err != nil {
		t.Skip("cargo is not installed: cannot build SLIR")
	}
	out := filepath.Join(t.TempDir(), "doc.slir")
	cmd := exec.Command(cargo, "run", "-q", "-p", "slab-cli", "--",
		"build", filepath.Join("examples", example), "-o", out)
	cmd.Dir = root
	if output, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("slab build: %v\n%s", err, output)
	}
	slir, err := os.ReadFile(out)
	if err != nil {
		t.Fatalf("read SLIR: %v", err)
	}
	return slir
}

func TestOpenSLIRInstallsPrecompiledDocument(t *testing.T) {
	slir := buildSLIR(t, "10-settings.slab")
	ctx, session := newSession(t)
	if err := session.OpenSLIR(ctx, slir, "10-settings.slir"); err != nil {
		t.Fatalf("OpenSLIR: %v", err)
	}
	if err := session.SetEnvCells(ctx, 80, 24, true, false); err != nil {
		t.Fatalf("SetEnvCells: %v", err)
	}
	cells, err := session.Cells(ctx, CellsOptions{Plain: true})
	if err != nil {
		t.Fatalf("Cells: %v", err)
	}
	if !strings.Contains(cells.Text, "Settings") {
		t.Errorf("rendered grid is missing \"Settings\":\n%s", cells.Text)
	}
	info, err := session.Info(ctx)
	if err != nil {
		t.Fatalf("Info: %v", err)
	}
	if !contains(info.Signals, "reset") {
		t.Errorf("signals = %v, want the authored reset signal", info.Signals)
	}
	effects, err := session.ClickKey(ctx, "#reset")
	if err != nil {
		t.Fatalf("ClickKey: %v", err)
	}
	if _, ok := effects.Signal("reset"); !ok {
		t.Errorf("signals = %+v, want one named reset", effects.Signals)
	}
}

func TestOpenSLIRRejectsGarbage(t *testing.T) {
	ctx, session := newSession(t)
	err := session.OpenSLIR(ctx, []byte("not slir at all"), "junk.slir")
	if err == nil {
		t.Fatal("OpenSLIR accepted bytes that are not SLIR")
	}
}
