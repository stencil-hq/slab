//! Shared windowed-surface policy for the winit hosts.
//!
//! macOS live resize only looks right when each frame is committed inside the
//! `CATransaction` that resizes the window: hosts opt the `CAMetalLayer` into
//! [`enable_transactional_presents`] once after creating the surface, then
//! draw synchronously from `WindowEvent::Resized`. Without it presents are
//! asynchronous and the compositor stretches the previous frame to the new
//! bounds before the fresh drawable lands.
//!
//! The companion acquisition rules hosts follow in their draw loops:
//! - `Timeout` and post-reconfigure `Lost`/`Outdated` must `request_redraw`
//!   before skipping, so a dropped frame is retried instead of leaving stale
//!   layout onscreen until the next input event.
//! - `Occluded` skips quietly; hosts repaint from
//!   `WindowEvent::Occluded(false)` rather than spinning while hidden.
//! - At most one synchronous draw per `Resized` event: transactional drawables
//!   return to the pool only when the resize transaction commits, so queueing a
//!   second render per tick starves `nextDrawable` and stalls the drag.

/// Opts the surface's `CAMetalLayer` into transactional presents.
#[cfg(target_os = "macos")]
pub fn enable_transactional_presents(surface: &wgpu::Surface<'_>) {
	// SAFETY: the hal surface guard is only used to reach the CAMetalLayer
	// and is dropped immediately; the surface is not destroyed through it.
	if let Some(hal_surface) = unsafe { surface.as_hal::<wgpu::hal::api::Metal>() } {
		hal_surface
			.render_layer()
			.lock()
			.setPresentsWithTransaction(true);
	}
}

/// Transactional presents are a macOS `CAMetalLayer` concern; no-op elsewhere.
#[cfg(not(target_os = "macos"))]
pub const fn enable_transactional_presents(_surface: &wgpu::Surface<'_>) {}
