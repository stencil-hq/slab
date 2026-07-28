//go:build !unix

package slabtui

// watchResize reports every terminal resize until done is closed.
//
// Platforms without SIGWINCH never report one; the driver still resizes when
// the caller restarts it with explicit Options.Cols and Options.Rows.
func watchResize(done <-chan struct{}) <-chan struct{} {
	resizes := make(chan struct{})
	go func() {
		<-done
		close(resizes)
	}()
	return resizes
}
