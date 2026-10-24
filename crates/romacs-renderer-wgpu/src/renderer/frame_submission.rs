//! One `queue.submit` per frame.
//!
//! Between [`WgpuRenderer::begin_frame_submission`] and
//! [`WgpuRenderer::finish_frame_submission`] every pass records its command
//! buffer here instead of submitting it, and the frame's buffers go to the
//! queue together, in recording order, as one submission. Outside a frame
//! each pass still submits as it finishes.
//!
//! Batching moves every `queue.write_*` staged during the frame in front of
//! the frame's first command buffer. So no pass may rewrite storage that an
//! earlier pass of the same frame reads: the vertex arenas reset and the
//! glyph atlas releases its page pins once per frame, not once per pass
//! ([`FrameSubmissions::is_batching`], [`FrameSubmissions::pin_epoch`]).

use std::cell::{Cell, RefCell};

use super::WgpuRenderer;

#[derive(Default)]
pub(crate) struct FrameSubmissions {
    /// `Some` while a frame is open: its recorded, unsubmitted buffers.
    batch: RefCell<Option<Vec<wgpu::CommandBuffer>>>,
    epoch: Cell<u64>,
    queue_submits: Cell<u64>,
}

impl FrameSubmissions {
    pub(crate) fn is_batching(&self) -> bool {
        self.batch.borrow().is_some()
    }

    /// The atlas pin epoch for a glyph pass starting now: shared by every
    /// pass of an open frame, fresh for each pass outside one.
    pub(crate) fn pin_epoch(&self) -> u64 {
        if !self.is_batching() {
            self.epoch.set(self.epoch.get() + 1);
        }
        self.epoch.get()
    }

    fn begin(&self) {
        let mut batch = self.batch.borrow_mut();
        assert!(batch.is_none(), "frame submission already open");
        *batch = Some(Vec::new());
        self.epoch.set(self.epoch.get() + 1);
    }

    fn take_pending(&self) -> Vec<wgpu::CommandBuffer> {
        self.batch
            .borrow_mut()
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// Submit now when no frame is open; otherwise defer to the frame's
    /// single submission.
    pub(crate) fn record(
        &self,
        queue: &wgpu::Queue,
        buffers: impl IntoIterator<Item = wgpu::CommandBuffer>,
    ) {
        if let Some(batch) = self.batch.borrow_mut().as_mut() {
            batch.extend(buffers);
            return;
        }
        self.submit(queue, buffers);
    }

    /// Submit the open frame's pending buffers followed by `buffers`, for a
    /// caller that needs the real submission index (GPU timing readbacks).
    pub(crate) fn submit_now(
        &self,
        queue: &wgpu::Queue,
        buffers: impl IntoIterator<Item = wgpu::CommandBuffer>,
    ) -> wgpu::SubmissionIndex {
        let pending = self.take_pending();
        self.submit(queue, pending.into_iter().chain(buffers))
    }

    fn flush(&self, queue: &wgpu::Queue) {
        let pending = self.take_pending();
        if !pending.is_empty() {
            self.submit(queue, pending);
        }
    }

    fn finish(&self, queue: &wgpu::Queue) {
        self.flush(queue);
        self.batch.borrow_mut().take();
    }

    fn submit(
        &self,
        queue: &wgpu::Queue,
        buffers: impl IntoIterator<Item = wgpu::CommandBuffer>,
    ) -> wgpu::SubmissionIndex {
        self.queue_submits.set(self.queue_submits.get() + 1);
        crate::diagnostics::submit(queue, buffers)
    }
}

impl WgpuRenderer {
    /// Open a frame: until [`Self::finish_frame_submission`], passes are
    /// recorded and submitted together. Panics if a frame is already open.
    pub fn begin_frame_submission(&mut self) {
        self.submissions.begin();
        self.arenas.begin_frame();
    }

    /// Submit the open frame's recorded work now and keep the frame open.
    /// Call before dropping a surface texture the recorded work draws into.
    pub fn flush_frame_submission(&self) {
        self.submissions.flush(&self.queue);
    }

    /// Submit the frame's recorded work as one `queue.submit` and close it.
    pub fn finish_frame_submission(&mut self) {
        self.submissions.finish(&self.queue);
    }

    /// `queue.submit` calls this renderer has made for its own passes.
    pub fn queue_submit_count(&self) -> u64 {
        self.submissions.queue_submits.get()
    }

    pub(crate) fn submit_commands(&self, buffers: impl IntoIterator<Item = wgpu::CommandBuffer>) {
        self.submissions.record(&self.queue, buffers);
    }

    /// Whether a pass may reset the vertex arenas: only outside an open
    /// frame. The frame reset them when it opened, and its earlier passes
    /// still read what they uploaded.
    pub(crate) fn may_reset_arenas(&self) -> bool {
        !self.submissions.is_batching()
    }
}
