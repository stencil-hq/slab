// Package slabtui drives a Slab document in a terminal.
//
// The driver owns nothing but the terminal: it translates raw input bytes into
// SDP input calls, paces the virtual clock, and paints the cell grid the kernel
// returns. Layout, hit testing, focus, editing, scrolling, and motion all stay
// inside the kernel, exactly as in the Rust `slab-tui` host.
//
// Typical use:
//
//	rt, _ := slab.NewRuntime(ctx)
//	defer rt.Close(ctx)
//	sess, _ := rt.NewSession(ctx)
//	defer sess.Close(ctx)
//	sess.OpenFile(ctx, "examples/10-settings.slab")
//	slabtui.Run(ctx, sess, slabtui.Options{Dark: true, OnSignal: print})
package slabtui

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	"github.com/stencil-hq/slab/clients/go/slab"
	"golang.org/x/term"
)

// DefaultFPS is the repaint and clock rate used when Options.FPS is zero.
const DefaultFPS = 30.0

// Default terminal size used when the output is not a terminal.
const (
	defaultCols = 80
	defaultRows = 24
)

// Terminal control sequences the driver writes.
const (
	enterScreen = "\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[?2004h\x1b[2J"
	leaveScreen = "\x1b[?2004l\x1b[?1006l\x1b[?1002l\x1b[?1000l\x1b[?25h\x1b[?1049l\x1b[0m"
)

// Options configures a driver run.
//
// The zero value drives the current document on stdin and stdout in the light
// color scheme at [DefaultFPS].
type Options struct {
	// Input is the terminal byte source; nil means os.Stdin.
	Input io.Reader
	// Output receives the painted frames; nil means os.Stdout.
	Output io.Writer
	// Dark selects the dark color scheme.
	Dark bool
	// Coarse selects coarse (touch) pointer semantics.
	Coarse bool
	// FPS paces clock advances and repaints; zero means [DefaultFPS].
	FPS float64
	// Cols overrides the detected column count; zero means detect.
	Cols int
	// Rows overrides the detected row count; zero means detect.
	Rows int
	// OnSignal observes every signal the document emits, in kernel order.
	// Returning an error stops the run and is returned from [Run].
	OnSignal func(slab.Signal) error
}

// clickTracker counts consecutive presses and tracks the pointer position.
type clickTracker struct {
	last    time.Time
	button  int
	x, y    float64
	count   int
	hasLast bool
}

// press records a button press and returns its consecutive click count.
//
// A press counts as consecutive when it repeats the same button within 500 ms
// and within four layout units of the previous press.
func (t *clickTracker) press(button int, x, y float64) int {
	now := time.Now()
	count := 1
	if t.hasLast {
		dx, dy := x-t.x, y-t.y
		if button == t.button && now.Sub(t.last) <= 500*time.Millisecond && dx*dx+dy*dy <= 16 {
			count = t.count + 1
		}
	}
	t.last, t.button, t.x, t.y, t.count, t.hasLast = now, button, x, y, count, true
	return count
}

// Dispatcher translates decoded terminal events into SDP input calls.
//
// It holds the click history a terminal does not provide. The zero value is
// ready to use and is not safe for concurrent use.
type Dispatcher struct {
	clicks clickTracker
}

// Dispatch applies one decoded event to the session.
//
// The boolean result reports whether the event produced a dispatch; it is false
// for [EventQuit] and for events the document ignores.
func (d *Dispatcher) Dispatch(ctx context.Context, sess *slab.Session, event Event) (slab.Effects, bool, error) {
	var (
		effects slab.Effects
		err     error
	)
	switch event.Kind {
	case EventQuit:
		return slab.Effects{}, false, nil
	case EventKey:
		effects, err = sess.Key(ctx, event.Key, event.Mods...)
	case EventText:
		effects, err = sess.Text(ctx, event.Text)
	case EventPaste:
		effects, err = sess.Paste(ctx, event.Text)
	case EventPointerMove:
		x, y := event.PointerXY()
		effects, err = sess.Pointer(ctx, "move", x, y, slab.WithButton(event.Button), slab.WithMods(event.Mods...))
	case EventPointerDown:
		x, y := event.PointerXY()
		clicks := d.clicks.press(event.Button, x, y)
		effects, err = sess.Pointer(ctx, "down", x, y,
			slab.WithButton(event.Button), slab.WithClicks(clicks), slab.WithMods(event.Mods...))
	case EventPointerUp:
		x, y := event.PointerXY()
		effects, err = sess.Pointer(ctx, "up", x, y, slab.WithButton(event.Button), slab.WithMods(event.Mods...))
	case EventWheel:
		x, y := event.PointerXY()
		effects, err = sess.Wheel(ctx, x, y, event.WheelDY(), event.Mods...)
	default:
		return slab.Effects{}, false, fmt.Errorf("slabtui: unhandled event kind %s", event.Kind)
	}
	if err != nil {
		return slab.Effects{}, false, err
	}
	return effects, true, nil
}

// Run drives sess on a terminal until the user quits or the input ends.
//
// Ctrl+C ends the run: it is never forwarded to the document. The session is
// left open so the caller can inspect it; the terminal is always restored.
func Run(ctx context.Context, sess *slab.Session, opts Options) error {
	input := opts.Input
	if input == nil {
		input = os.Stdin
	}
	output := opts.Output
	if output == nil {
		output = os.Stdout
	}
	fps := opts.FPS
	if fps <= 0 {
		fps = DefaultFPS
	}

	if file, ok := input.(*os.File); ok && term.IsTerminal(int(file.Fd())) {
		state, err := term.MakeRaw(int(file.Fd()))
		if err != nil {
			return fmt.Errorf("slabtui: enter raw mode: %w", err)
		}
		defer func() { _ = term.Restore(int(file.Fd()), state) }()
	}
	if _, err := io.WriteString(output, enterScreen); err != nil {
		return fmt.Errorf("slabtui: enter alternate screen: %w", err)
	}
	defer func() { _, _ = io.WriteString(output, leaveScreen) }()

	loop := &driver{
		session: sess,
		output:  output,
		opts:    opts,
		frame:   time.Duration(float64(time.Second) / fps),
		frameMS: 1000 / fps,
	}
	loop.cols, loop.rows = terminalSize(input, output, opts)
	if err := loop.resize(ctx, loop.cols, loop.rows); err != nil {
		return err
	}

	done := make(chan struct{})
	defer close(done)
	chunks, readErrs := readLoop(input, done)
	resizes := watchResize(done)
	ticker := time.NewTicker(loop.frame)
	defer ticker.Stop()

	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case chunk := <-chunks:
			quit, err := loop.feed(ctx, chunk)
			if err != nil {
				return err
			}
			if quit {
				return sess.Quit(ctx)
			}
			if err := loop.paint(ctx); err != nil {
				return err
			}
		case err := <-readErrs:
			if errors.Is(err, io.EOF) {
				return nil
			}
			return fmt.Errorf("slabtui: read input: %w", err)
		case <-resizes:
			cols, rows := terminalSize(input, output, opts)
			if cols != loop.cols || rows != loop.rows {
				if err := loop.resize(ctx, cols, rows); err != nil {
					return err
				}
			}
		case <-ticker.C:
			if err := sess.Advance(ctx, loop.frameMS); err != nil {
				return err
			}
			if err := loop.paint(ctx); err != nil {
				return err
			}
		}
	}
}

// driver is the mutable state of one [Run] call.
type driver struct {
	session    *slab.Session
	output     io.Writer
	opts       Options
	decoder    Decoder
	dispatcher Dispatcher
	painted    string
	screen     bytes.Buffer
	cols, rows int
	frame      time.Duration
	frameMS    float64
}

// resize applies a new terminal size and repaints from scratch.
func (d *driver) resize(ctx context.Context, cols, rows int) error {
	d.cols, d.rows = cols, rows
	if err := d.session.SetEnvCells(ctx, cols, rows, d.opts.Dark, d.opts.Coarse); err != nil {
		return err
	}
	d.painted = ""
	if _, err := io.WriteString(d.output, "\x1b[2J"); err != nil {
		return fmt.Errorf("slabtui: clear screen: %w", err)
	}
	return d.paint(ctx)
}

// feed decodes one input chunk and dispatches every event it completes.
//
// The boolean result reports a quit request.
func (d *driver) feed(ctx context.Context, chunk []byte) (bool, error) {
	for _, event := range d.decoder.Feed(chunk) {
		if event.Kind == EventQuit {
			return true, nil
		}
		effects, _, err := d.dispatcher.Dispatch(ctx, d.session, event)
		if err != nil {
			return false, err
		}
		if d.opts.OnSignal == nil {
			continue
		}
		for _, signal := range effects.Signals {
			if err := d.opts.OnSignal(signal); err != nil {
				return false, err
			}
		}
	}
	return false, nil
}

// paint renders the document and writes it when the grid changed.
func (d *driver) paint(ctx context.Context) error {
	cells, err := d.session.Cells(ctx, slab.CellsOptions{Plain: false, Caret: true})
	if err != nil {
		return err
	}
	if cells.Text == d.painted {
		return nil
	}
	d.painted = cells.Text
	d.screen.Reset()
	d.screen.WriteString("\x1b[H")
	for index, line := range strings.Split(cells.Text, "\n") {
		if index > 0 {
			d.screen.WriteString("\r\n")
		}
		d.screen.WriteString(line)
		d.screen.WriteString("\x1b[K")
	}
	d.screen.WriteString("\x1b[0m\x1b[J")
	if _, err := d.output.Write(d.screen.Bytes()); err != nil {
		return fmt.Errorf("slabtui: paint: %w", err)
	}
	return nil
}

// terminalSize resolves the grid size from the options or the real terminal.
func terminalSize(input io.Reader, output io.Writer, opts Options) (int, int) {
	cols, rows := opts.Cols, opts.Rows
	if cols <= 0 || rows <= 0 {
		detectedCols, detectedRows, ok := detectSize(output)
		if !ok {
			detectedCols, detectedRows, ok = detectSize(input)
		}
		if !ok {
			detectedCols, detectedRows = defaultCols, defaultRows
		}
		if cols <= 0 {
			cols = detectedCols
		}
		if rows <= 0 {
			rows = detectedRows
		}
	}
	return cols, rows
}

// detectSize asks the operating system for the size of a terminal stream.
func detectSize(stream any) (int, int, bool) {
	file, ok := stream.(*os.File)
	if !ok {
		return 0, 0, false
	}
	cols, rows, err := term.GetSize(int(file.Fd()))
	if err != nil || cols <= 0 || rows <= 0 {
		return 0, 0, false
	}
	return cols, rows, true
}

// readLoop copies input chunks onto a channel until the stream ends.
func readLoop(input io.Reader, done <-chan struct{}) (<-chan []byte, <-chan error) {
	chunks := make(chan []byte)
	errs := make(chan error, 1)
	go func() {
		buf := make([]byte, 4096)
		for {
			count, err := input.Read(buf)
			if count > 0 {
				chunk := make([]byte, count)
				copy(chunk, buf[:count])
				select {
				case chunks <- chunk:
				case <-done:
					return
				}
			}
			if err != nil {
				select {
				case errs <- err:
				case <-done:
				}
				return
			}
		}
	}()
	return chunks, errs
}
