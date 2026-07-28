//go:build unix

package slabtui

import (
	"os"
	"os/signal"
	"syscall"
)

// watchResize reports every terminal resize until done is closed.
func watchResize(done <-chan struct{}) <-chan struct{} {
	signals := make(chan os.Signal, 1)
	signal.Notify(signals, syscall.SIGWINCH)
	resizes := make(chan struct{}, 1)
	go func() {
		defer signal.Stop(signals)
		for {
			select {
			case <-done:
				return
			case <-signals:
				select {
				case resizes <- struct{}{}:
				default:
				}
			}
		}
	}()
	return resizes
}
