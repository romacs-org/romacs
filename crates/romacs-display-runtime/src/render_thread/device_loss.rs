//! GPU device-loss detection for the render thread.
//!
//! A user shader with an infinite loop can hang the GPU
//! (`docs/display-engine/SHADER_SURFACES.md`: naga guarantees memory safety,
//! not termination). The driver then resets (TDR) and the wgpu device is
//! lost. Two signals feed one recovery decision:
//!
//! 1. wgpu's device-lost callback (`Device::set_device_lost_callback`),
//!    which may fire on any thread from inside wgpu's maintain paths, and
//! 2. a streak of consecutive `CurrentSurfaceTexture::Lost` acquisitions —
//!    some backends only report a reset through the swapchain.
//!
//! Both set one shared flag; `handle_about_to_wait` drains it and runs
//! `RenderApp::recover_from_device_loss`.
//!
//! wgpu's uncaptured-error handler panics by default, ending the session on
//! any validation error. [`DeviceLossDetector::uncaptured_errors`] replaces
//! it: an out-of-memory or internal error is a loss, recovered the same
//! way; a validation error is told once and repaints every window, whose
//! frame it may have spoiled. Only recovery failing
//! [`MAX_FAILED_RECOVERIES`] times in a row ends the session.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Consecutive `CurrentSurfaceTexture::Lost` results escalated to a device
/// loss. A one-off Lost is a normal swapchain hiccup and is merely skipped;
/// a device that keeps answering Lost never presents again without a
/// rebuild.
pub(super) const CONSECUTIVE_SURFACE_LOST_THRESHOLD: u32 = 30;

/// Consecutive failed GPU rebuilds, a second apart, before the device is
/// called unrecoverable and the session ends saying so.
pub(super) const MAX_FAILED_RECOVERIES: u32 = 10;

/// What an uncaptured wgpu error asks of the render thread.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Fault {
    /// The device may be unusable: rebuild it.
    Lost,
    /// A command was rejected: repaint, once per distinct error.
    Rejected(String),
}

pub(super) fn fault(error: &wgpu::Error) -> Fault {
    match error {
        wgpu::Error::OutOfMemory { .. } | wgpu::Error::Internal { .. } => Fault::Lost,
        wgpu::Error::Validation { description, .. } => Fault::Rejected(description.clone()),
    }
}

/// Whether DESCRIPTION is new to SEEN, which keeps a bounded few.
fn first_seen(seen: &std::sync::Mutex<Vec<String>>, description: &str) -> bool {
    let mut seen = seen.lock().unwrap_or_else(|e| e.into_inner());
    if seen.len() >= 32 || seen.iter().any(|known| known == description) { return false; }
    seen.push(description.to_owned());
    true
}

/// Shared device-lost latch plus the consecutive surface-Lost streak.
pub(super) struct DeviceLossDetector {
    /// Set by the wgpu device-lost callback (any thread) or by the streak
    /// escalation; drained once per event-loop pass by [`Self::take`].
    flag: Arc<AtomicBool>,
    consecutive_surface_lost: u32,
    /// Set by a newly seen validation error; drained by [`Self::take_repaint`].
    repaint: Arc<AtomicBool>,
    failed_recoveries: u32,
    /// After a failed rebuild, none before this: other wakes come sooner.
    retry_at: Option<romacs_display_protocol::frame_time::EventTime>,
}

impl DeviceLossDetector {
    pub(super) fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            consecutive_surface_lost: 0,
            repaint: Arc::new(AtomicBool::new(false)),
            failed_recoveries: 0,
            retry_at: None,
        }
    }

    /// Clone of the shared flag for the wgpu device-lost callback.
    pub(super) fn shared_flag(&self) -> Arc<AtomicBool> {
        self.flag.clone()
    }

    /// wgpu's uncaptured-error handler for this detector's device. It may
    /// run on any thread, so it only logs and latches.
    pub(super) fn uncaptured_errors(&self) -> impl Fn(wgpu::Error) + Send + Sync + 'static {
        let (lost, repaint) = (self.flag.clone(), self.repaint.clone());
        let seen = std::sync::Mutex::new(Vec::new());
        move |error| match fault(&error) {
            Fault::Lost => {
                tracing::error!(%error, "wgpu error: rebuilding the GPU device");
                lost.store(true, Ordering::SeqCst);
            }
            Fault::Rejected(description) => {
                if first_seen(&seen, &description) {
                    tracing::error!(%error, "wgpu validation error: repainting (told once)");
                    repaint.store(true, Ordering::SeqCst);
                }
            }
        }
    }

    /// Drain the repaint latch a new validation error set.
    pub(super) fn take_repaint(&self) -> bool {
        self.repaint.swap(false, Ordering::SeqCst)
    }

    /// Count a rebuild's outcome at NOW; true once rebuilds failed
    /// [`MAX_FAILED_RECOVERIES`] times in a row. The next waits a second.
    pub(super) fn record_recovery(
        &mut self,
        recovered: bool,
        now: romacs_display_protocol::frame_time::EventTime,
    ) -> bool {
        self.failed_recoveries = if recovered { 0 } else { self.failed_recoveries + 1 };
        self.retry_at = (!recovered).then(|| now.plus(std::time::Duration::from_secs(1)));
        self.failed_recoveries >= MAX_FAILED_RECOVERIES
    }

    /// When a latched loss may next be rebuilt, if not yet at NOW.
    pub(super) fn retry_wait(
        &self,
        now: romacs_display_protocol::frame_time::EventTime,
    ) -> Option<romacs_display_protocol::frame_time::EventTime> {
        self.retry_at.filter(|at| *at > now && self.flag.load(Ordering::SeqCst))
    }

    /// Latch a loss from the render thread itself (debug simulation, or a
    /// failed recovery that must be retried on the next pass).
    pub(super) fn mark_lost_now(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    /// Drain the latch; true means a recovery pass must run now.
    pub(super) fn take(&mut self) -> bool {
        let lost = self.flag.swap(false, Ordering::SeqCst);
        if lost {
            self.consecutive_surface_lost = 0;
        }
        lost
    }

    /// Record one `CurrentSurfaceTexture::Lost` acquisition. Returns true
    /// when the streak reaches the threshold, at which point the shared flag
    /// has been latched and the streak reset.
    pub(super) fn record_surface_lost(&mut self) -> bool {
        self.consecutive_surface_lost += 1;
        if self.consecutive_surface_lost >= CONSECUTIVE_SURFACE_LOST_THRESHOLD {
            self.consecutive_surface_lost = 0;
            self.flag.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    /// Record a successful surface acquisition, resetting the streak.
    pub(super) fn record_surface_acquired(&mut self) {
        self.consecutive_surface_lost = 0;
    }
}

#[cfg(test)]
#[path = "device_loss/tests/device_loss_test.rs"]
mod tests;
