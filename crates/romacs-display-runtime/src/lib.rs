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
pub mod host_types;
mod menu_bar;
pub(crate) mod native_window_wait;
pub mod paint_host;
mod presentation;
mod presentation_feedback;
pub mod thread_comm;
#[cfg(target_os = "linux")]
mod wayland_toplevel_icon;
mod window_chrome;
mod window_icon;
mod window_identity;

pub mod render_thread;

pub use romacs_renderer_wgpu::supports_graphical_face_attribute;

/// Shader-surface composition/validation (re-exported so the frontend can
/// naga-validate user WGSL on the Lisp thread without a renderer dependency).
pub mod shader_surface {
    pub use romacs_renderer_wgpu::shader_surface::*;
}

/// Native model view types.
pub mod model {
    pub use romacs_renderer_wgpu::model::ModelView;
}

pub(crate) use render_thread::frame_stats::FrameSchedSnapshot;

/// Read the current process-global frame-scheduling counters.
///
/// Safe to call from any thread — the counters are relaxed atomics — so the
/// diagnostics server can sample frame timing without touching the render
/// thread's state.
pub fn frame_metrics_snapshot() -> FrameSchedSnapshot {
    render_thread::frame_stats::snapshot()
}

pub(crate) fn wgpu_instance_descriptor_with_display(
    display: winit::event_loop::OwnedDisplayHandle,
) -> wgpu::InstanceDescriptor {
    wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(display))
}

#[cfg(test)]
mod tests;
