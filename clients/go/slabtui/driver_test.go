package slabtui

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/stencil-hq/slab/clients/go/slab"
)

// examplePath resolves a repository example relative to this package.
func examplePath(name string) string {
	return filepath.Join("..", "..", "..", "examples", name)
}

// newSession opens a repository example sized for an 80x24 terminal.
func newSession(t *testing.T, name string) (context.Context, *slab.Session) {
	t.Helper()
	ctx := context.Background()
	runtime, err := slab.NewRuntime(ctx)
	if err != nil {
		t.Fatalf("NewRuntime: %v", err)
	}
	t.Cleanup(func() { _ = runtime.Close(ctx) })
	session, err := runtime.NewSession(ctx)
	if err != nil {
		t.Fatalf("NewSession: %v", err)
	}
	t.Cleanup(func() { _ = session.Close(ctx) })
	if err := session.OpenFile(ctx, examplePath(name)); err != nil {
		t.Fatalf("OpenFile: %v", err)
	}
	if err := session.SetEnvCells(ctx, 80, 24, true, false); err != nil {
		t.Fatalf("SetEnvCells: %v", err)
	}
	return ctx, session
}

// syncBuffer is a bytes.Buffer safe to read while the driver writes to it.
type syncBuffer struct {
	mu  sync.Mutex
	buf bytes.Buffer
}

// Write appends p to the buffer.
func (b *syncBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.Write(p)
}

// String returns everything written so far.
func (b *syncBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.String()
}

func TestScriptedBytesReachTheKernel(t *testing.T) {
	ctx, session := newSession(t, "10-settings.slab")
	var decoder Decoder
	var dispatcher Dispatcher
	var seen []string
	for _, event := range decoder.Feed([]byte("\t\r")) {
		effects, dispatched, err := dispatcher.Dispatch(ctx, session, event)
		if err != nil {
			t.Fatalf("Dispatch(%s): %v", event.Kind, err)
		}
		if !dispatched {
			t.Fatalf("event %s produced no dispatch", event.Kind)
		}
		for _, signal := range effects.Signals {
			seen = append(seen, signal.Name)
		}
	}
	if len(seen) != 1 || seen[0] != "save" {
		t.Fatalf("signals = %v, want [save]", seen)
	}
}

func TestScriptedMouseBytesReachTheKernel(t *testing.T) {
	ctx, session := newSession(t, "10-settings.slab")
	cells, err := session.Cells(ctx, slab.CellsOptions{Plain: true})
	if err != nil {
		t.Fatalf("Cells: %v", err)
	}
	col, row, ok := findText(cells.Text, "Reset")
	if !ok {
		t.Fatalf("the rendered grid has no Reset button:\n%s", cells.Text)
	}

	// SGR mouse reports are 1-based, so add one to the zero-based cell.
	script := []byte("\x1b[<0;" + strconv.Itoa(col+1) + ";" + strconv.Itoa(row+1) + "M" +
		"\x1b[<0;" + strconv.Itoa(col+1) + ";" + strconv.Itoa(row+1) + "m")
	var decoder Decoder
	var dispatcher Dispatcher
	var seen []string
	for _, event := range decoder.Feed(script) {
		effects, _, err := dispatcher.Dispatch(ctx, session, event)
		if err != nil {
			t.Fatalf("Dispatch(%s): %v", event.Kind, err)
		}
		for _, signal := range effects.Signals {
			seen = append(seen, signal.Name)
		}
	}
	if len(seen) != 1 || seen[0] != "reset" {
		t.Fatalf("signals = %v, want [reset]", seen)
	}
}

func TestDispatchCountsConsecutiveClicks(t *testing.T) {
	ctx, session := newSession(t, "10-settings.slab")
	cells, err := session.Cells(ctx, slab.CellsOptions{Plain: true})
	if err != nil {
		t.Fatalf("Cells: %v", err)
	}
	col, row, ok := findText(cells.Text, "Reset")
	if !ok {
		t.Fatalf("the rendered grid has no Reset button:\n%s", cells.Text)
	}
	var dispatcher Dispatcher
	press := Event{Kind: EventPointerDown, Col: col, Row: row}
	for attempt := range 2 {
		if _, dispatched, err := dispatcher.Dispatch(ctx, session, press); err != nil || !dispatched {
			t.Fatalf("press %d: dispatched=%v err=%v", attempt+1, dispatched, err)
		}
	}
	if dispatcher.clicks.count != 2 {
		t.Errorf("click count = %d, want 2 for a repeated press", dispatcher.clicks.count)
	}
	away := Event{Kind: EventPointerDown, Col: col + 10, Row: row + 3}
	if _, _, err := dispatcher.Dispatch(ctx, session, away); err != nil {
		t.Fatalf("distant press: %v", err)
	}
	if dispatcher.clicks.count != 1 {
		t.Errorf("click count = %d, want 1 after a press elsewhere", dispatcher.clicks.count)
	}
}

func TestRunPaintsAndReportsSignals(t *testing.T) {
	ctx, session := newSession(t, "10-settings.slab")
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatalf("Pipe: %v", err)
	}
	defer reader.Close()
	output := &syncBuffer{}

	var seen []string
	runCtx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()

	go func() {
		// Tab focuses the first button, Enter activates it, Ctrl+C quits.
		_, _ = writer.Write([]byte("\t\r"))
		time.Sleep(200 * time.Millisecond)
		_, _ = writer.Write([]byte("\x03"))
		_ = writer.Close()
	}()

	err = Run(runCtx, session, Options{
		Input:  reader,
		Output: output,
		Dark:   true,
		FPS:    60,
		Cols:   80,
		Rows:   24,
		OnSignal: func(signal slab.Signal) error {
			seen = append(seen, signal.Name)
			return nil
		},
	})
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if len(seen) != 1 || seen[0] != "save" {
		t.Fatalf("signals = %v, want [save]", seen)
	}

	painted := output.String()
	for _, want := range []string{enterScreen, leaveScreen, "Settings", "Reset"} {
		if !strings.Contains(painted, want) {
			t.Errorf("driver output is missing %q", want)
		}
	}
	ended, err := session.Ended(ctx)
	if err != nil {
		t.Fatalf("Ended: %v", err)
	}
	if !ended {
		t.Error("Ctrl+C did not quit the session")
	}
}

func TestRunStopsOnInputEnd(t *testing.T) {
	ctx, session := newSession(t, "00-player.slab")
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatalf("Pipe: %v", err)
	}
	defer reader.Close()
	go func() {
		_, _ = writer.Write([]byte("\x1b[B"))
		time.Sleep(100 * time.Millisecond)
		_ = writer.Close()
	}()
	runCtx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	output := &syncBuffer{}
	if err := Run(runCtx, session, Options{Input: reader, Output: output, Cols: 60, Rows: 20}); err != nil {
		t.Fatalf("Run: %v", err)
	}
	if !strings.Contains(output.String(), leaveScreen) {
		t.Error("the driver did not restore the terminal")
	}
	ended, err := session.Ended(ctx)
	if err != nil {
		t.Fatalf("Ended: %v", err)
	}
	if ended {
		t.Error("an input EOF quit the session, which only Ctrl+C should do")
	}
}

func TestRunPropagatesSignalHandlerErrors(t *testing.T) {
	ctx, session := newSession(t, "10-settings.slab")
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatalf("Pipe: %v", err)
	}
	defer reader.Close()
	go func() {
		_, _ = writer.Write([]byte("\t\r"))
	}()
	runCtx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	stop := errStop{}
	err = Run(runCtx, session, Options{
		Input:    reader,
		Output:   &syncBuffer{},
		Cols:     80,
		Rows:     24,
		OnSignal: func(slab.Signal) error { return stop },
	})
	_ = writer.Close()
	if err != stop {
		t.Fatalf("Run err = %v, want the handler error", err)
	}
}

// errStop is the sentinel a test signal handler returns to stop the driver.
type errStop struct{}

// Error implements the error interface.
func (errStop) Error() string { return "stop" }

// findText locates the first cell of want in a plain cell grid.
func findText(grid, want string) (int, int, bool) {
	for row, line := range strings.Split(grid, "\n") {
		column := strings.Index(line, want)
		if column < 0 {
			continue
		}
		// Cells are one rune wide, so the column is the rune offset.
		return len([]rune(line[:column])), row, true
	}
	return 0, 0, false
}
