//! Opt-in CPU scopes and completion evidence for display stage attribution.
//!
//! GPU completion is callback receipt time, not a GPU clock timestamp or a
//! compositor fence. Call `gpu_done_after_submitted_work` after the LAST frame
//! submission to include non-root composition as well as the root glyph pass.
use romacs_display_protocol::{
    PresentationId,
    present_trace::{self, Stage},
};

#[must_use]
pub struct Span {
    _scope: present_trace::Span,
}

impl Span {
    pub fn new(start: Stage, end: Stage, frame: u64, presentation: PresentationId) -> Self {
        Self {
            _scope: present_trace::Span::new(start, end, frame, Some(presentation)),
        }
    }
}

/// Register only when tracing is enabled. wgpu dispatches this callback while
/// polling/submitting; its scheduling delay is intentionally included.
pub fn gpu_done_after_submitted_work(
    queue: &wgpu::Queue,
    frame: u64,
    presentation: PresentationId,
) {
    if present_trace::enabled() {
        let draw_seq = present_trace::current_draw();
        queue.on_submitted_work_done(move || {
            present_trace::record_draw(Stage::GpuDone, frame, presentation, draw_seq);
        });
    }
}

pub fn submit(
    queue: &wgpu::Queue,
    buffers: impl IntoIterator<Item = wgpu::CommandBuffer>,
) -> wgpu::SubmissionIndex {
    if !present_trace::enabled() { return queue.submit(buffers); }
    let (frame, presentation) = present_trace::current_draw_identity();
    let _submit = present_trace::current_draw().map(|_| {
        present_trace::Span::new(
            Stage::SubmitStart,
            Stage::SubmitEnd,
            frame,
            Some(presentation),
        )
    });
    queue.submit(buffers)
}

/// Emit once per configured surface (including externally-owned window surfaces).
/// Kept separate from device logging because a renderer can share a device while
/// each window owns its surface configuration.
pub fn surface_fingerprint(
    adapter: &wgpu::Adapter,
    surface: &wgpu::Surface<'_>,
    config: &wgpu::SurfaceConfiguration,
) {
    tracing::warn!(
        adapter = ?adapter.get_info(),
        surface_caps = ?surface.get_capabilities(adapter),
        present_mode = ?config.present_mode,
        maximum_frame_latency = config.desired_maximum_frame_latency,
        format = ?config.format,
        "renderer surface backend fingerprint"
    );
}
