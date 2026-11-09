//! Romacs Display Runtime
//!
//! A GPU-accelerated display engine for Romacs using wgpu.
//!
//! # Architecture
//!
//! ```text
//! Editor Runtime (Rust) ──► Scene Graph ──► wgpu ──► GPU
//! ```

// Backend/window entry points and GPU-plumbing helpers take many positional
// parameters (geometry, handles, callbacks); folding them into structs is a
// separate refactor, so this bulk category is allowed crate-wide.
#![allow(clippy::too_many_arguments)]

pub mod backend;
pub(crate) mod clipboard;
pub mod core;
pub mod display_identity;
pub mod display_scale;
#[cfg(feature = "gui-test-hooks")]
pub mod gui_test_controls;
pub mod host_types;
pub mod macos_bundle_runtime;
mod menus;
pub mod native_window_wait;
#[cfg(feature = "core-free")]
pub mod paint_host;
mod presentation;
mod presentation_feedback;
pub mod thread_comm;
mod tooltips;
#[cfg(target_os = "linux")]
mod wayland_toplevel_icon;
mod window_chrome;
mod window_icon;
mod window_identity;

pub mod render_thread;

pub use crate::core::*;
pub use romacs_renderer_wgpu::supports_graphical_face_attribute;

/// Shader-surface composition/validation (re-exported so the frontend can
/// naga-validate user WGSL on the Lisp thread without a renderer dependency).
pub mod shader_surface {
    pub use romacs_renderer_wgpu::shader_surface::*;
}

/// Native model view types the frontend passes in `SurfaceSource::Model`.
pub mod model {
    pub use romacs_renderer_wgpu::model::ModelView;
}

/// Library version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub const CORE_BACKEND: &str = "core-free";

pub use render_thread::frame_stats::{
    DEMAND_REASON_NAMES, FRAME_TIME_BUCKET_UPPER_US, FrameSchedSnapshot, WindowFrameSnapshot,
};

/// Read the current process-global frame-scheduling counters.
///
/// Safe to call from any thread — the counters are relaxed atomics — so the
/// diagnostics server can sample frame timing without touching the render
/// thread's state.
pub fn frame_metrics_snapshot() -> FrameSchedSnapshot {
    render_thread::frame_stats::snapshot()
}

/// Read the per-native-window demand attribution (currently-active demand
/// reasons and per-reason planned-frame counts), sorted by window id.
///
/// Safe to call from any thread: the render thread publishes into the shared
/// map only on event-loop wakes it was already performing, so sampling this
/// never wakes or blocks the render loop at idle.
pub fn window_frame_metrics_snapshot() -> Vec<WindowFrameSnapshot> {
    render_thread::frame_stats::window_snapshots()
}

pub(crate) fn wgpu_instance_descriptor_with_display(
    display: winit::event_loop::OwnedDisplayHandle,
) -> wgpu::InstanceDescriptor {
    wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(display))
}

/// Initialize the display engine.
///
/// Logging is initialized separately by the binary entry point via
/// the application and is assumed to already be set up when this function runs.
pub fn init() -> Result<(), DisplayError> {
    tracing::info!(
        "Romacs display engine v{} initializing (wgpu backend)",
        VERSION
    );
    Ok(())
}

#[cfg(test)]
mod tests;
