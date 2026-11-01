//! Render thread implementation.
//!
//! Owns winit event loop, wgpu, GLib/WebKit. Runs at native VSync.

mod app_handler;
mod asset_commands;
mod bootstrap;
pub(crate) mod child_frames;
mod command_processing;
mod cursor;
mod cursor_runtime;
#[cfg(all(test, target_os = "linux"))]
#[path = "tests/deferred_gui_native_test.rs"]
mod deferred_gui_native_test;
mod device_loss;
mod frame_compositor;
mod frame_ingest;
mod frame_preparation;
mod frame_sched;
mod frame_state;
pub(crate) mod frame_stats;
pub(crate) mod frame_windows;
mod gpu_startup;
mod input;
mod key_repeat;
#[cfg(test)]
#[path = "tests/legacy_frame_admission_test.rs"]
mod legacy_frame_admission_test;
#[cfg(all(test, target_os = "linux"))]
#[path = "tests/legacy_ready_native_test.rs"]
mod legacy_ready_native_test;
mod lifecycle;
mod media;
mod models;
#[cfg(test)]
#[path = "tests/modifier_policy_cooking_test.rs"]
mod modifier_policy_cooking_test;
mod modifier_sides;
#[cfg(all(feature = "gui-test-hooks", target_os = "linux"))]
pub mod native_image_probe;
mod pointer_events;
pub(in crate::render_thread) mod render_pass;
mod render_quality;
#[cfg(test)]
#[path = "tests/render_thread_test.rs"]
mod runtime_tests;
mod scroll_input;
mod startup;
mod state;
mod surface_readback;
#[cfg(test)]
#[path = "tests/core_free_paint_test.rs"]
mod tests;
#[cfg(test)]
#[path = "tests/texture_discipline_test.rs"]
mod texture_discipline_test;

mod geometry_hints;
mod surface_resize;
mod thread_handle;
#[cfg(test)]
#[path = "tests/time_discipline_test.rs"]
mod time_discipline_test;
mod toolbar;
mod transitions;
mod ui_commands;
mod window_commands;
mod window_events;

pub use bootstrap::{
    DaemonRenderRoot, build_render_event_loop, run_render_loop, run_render_loop_current_thread,
};
#[cfg(target_os = "linux")]
pub use bootstrap::{build_render_event_loop_wayland, build_render_event_loop_wayland_stream};
pub(crate) use lifecycle::PopupCommit;
pub use startup::{
    InitialWindowLifetime, InitialWindowReceiver, InitialWindowReply, InitialWindowSize,
    RenderLoopError, RenderLoopExit,
};
use state::{FpsCounter, ImeCursorArea, RenderApp};
pub use state::{
    ImageDecodeTerminal, ImageRenderState, ImageTerminalProbe, ImageTerminalPublication,
    MonitorInfo, RenderShutdownReason, SharedImageRenderState, SharedMonitorInfo,
};
pub use thread_handle::RenderThread;

pub use romacs_renderer_wgpu::SvgResourceContext;
/// Complete CPU semantic decoding, independent of render-loop service.
pub use romacs_renderer_wgpu::image_cache::{
    SemanticImageDecoded, SemanticImageDecoder, SemanticImageRequest,
    SemanticImageSequenceReservation, SemanticImageSource,
};
/// Header-only image geometry, for placing an image whose pixels have not been
/// decoded yet (see `romacs_renderer_wgpu::image_probe`).
pub use romacs_renderer_wgpu::image_probe::{ImageProbeSource, probe_image_layout};

/// Rows of an image whose decode is still running, as published by
/// `ImageDecodeTerminal::Band`.
pub use romacs_renderer_wgpu::RowRange;

use winit::event_loop::EventLoopProxy;

pub type RenderEventLoopProxy = EventLoopProxy;
pub type RenderEventLoop = winit::event_loop::EventLoop;

// All GPU caches (image, video, webkit) are managed by WgpuRenderer
