// Command example drives one Slab document in the terminal.
//
// It opens a `.slab` file, hands it to the slabtui driver, and prints every
// signal the document emitted once the driver exits. Press Ctrl+C to quit.
//
// Usage:
//
//	go run ./example [-file PATH] [-light] [-fps N]
package main

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"runtime"

	"github.com/stencil-hq/slab/clients/go/slab"
	"github.com/stencil-hq/slab/clients/go/slabtui"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintf(os.Stderr, "example: %v\n", err)
		os.Exit(1)
	}
}

// run parses the flags, drives the document, and reports the signals seen.
func run() error {
	file := flag.String("file", defaultFile(), "path to the .slab document to drive")
	light := flag.Bool("light", false, "use the light color scheme instead of dark")
	fps := flag.Float64("fps", slabtui.DefaultFPS, "repaint and clock rate")
	flag.Parse()

	ctx := context.Background()
	rt, err := slab.NewRuntime(ctx)
	if err != nil {
		return err
	}
	defer func() { _ = rt.Close(ctx) }()

	session, err := rt.NewSession(ctx)
	if err != nil {
		return err
	}
	defer func() { _ = session.Close(ctx) }()

	if err := session.OpenFile(ctx, *file); err != nil {
		return err
	}

	var seen []slab.Signal
	err = slabtui.Run(ctx, session, slabtui.Options{
		Dark: !*light,
		FPS:  *fps,
		OnSignal: func(signal slab.Signal) error {
			seen = append(seen, signal)
			return nil
		},
	})
	if err != nil && !errors.Is(err, context.Canceled) {
		return err
	}

	fmt.Printf("%s: %d signal(s)\n", *file, len(seen))
	for _, signal := range seen {
		fmt.Printf("  %s item=%q text=%q key=%s\n", signal.Name, signal.Item, signal.Text, signal.Meta.Key)
	}
	return nil
}

// defaultFile locates `examples/10-settings.slab` relative to the repository.
//
// The path is derived from this source file, so `go run ./example` works from
// any working directory inside a checkout. It falls back to a repository-root
// relative path when the source tree is not present.
func defaultFile() string {
	const relative = "examples/10-settings.slab"
	_, source, _, ok := runtime.Caller(0)
	if !ok {
		return relative
	}
	path := filepath.Join(filepath.Dir(source), "..", "..", "..", relative)
	if _, err := os.Stat(path); err != nil {
		return relative
	}
	return path
}
