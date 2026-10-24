use crate::backend::DecodedFrame;

/// Bounded presentation queue: preserve the next frame and coalesce successors.
///
/// A one-slot "latest frame" mailbox can starve a real-time consumer: every
/// newly decoded future frame replaces the one whose presentation deadline is
/// about to arrive. Two slots make the scheduling invariant explicit. The
/// head is stable until consumed, while the tail is always the newest known
/// successor, so latency and native-surface retention both remain bounded.
pub(crate) struct PresentationFrameQueue<F> {
    next: Option<F>,
    newest_successor: Option<F>,
}

impl<F> Default for PresentationFrameQueue<F> {
    fn default() -> Self {
        Self {
            next: None,
            newest_successor: None,
        }
    }
}

impl<F> PresentationFrameQueue<F> {
    /// Queue FRAME and return a coalesced successor, if the queue was full.
    #[must_use]
    pub(crate) fn publish(&mut self, frame: F) -> Option<F> {
        if self.next.is_none() {
            self.next = Some(frame);
            None
        } else {
            self.newest_successor.replace(frame)
        }
    }

    pub(crate) fn take(&mut self) -> Option<F> {
        let next = self.next.take()?;
        self.next = self.newest_successor.take();
        Some(next)
    }

    /// Drop every queued frame at a discontinuity or lifecycle boundary.
    ///
    /// This is intentionally distinct from [`Self::take`], which consumes one
    /// presentation candidate and promotes its successor.
    pub(crate) fn clear(&mut self) {
        self.next = None;
        self.newest_successor = None;
    }
}

impl<F> PresentationFrameQueue<DecodedFrame<F>> {
    pub(crate) fn timing(&self) -> Option<crate::FrameTiming> {
        self.next.as_ref().map(|frame| frame.timing)
    }

    pub(crate) fn successor_timing(&self) -> Option<crate::FrameTiming> {
        self.newest_successor.as_ref().map(|frame| frame.timing)
    }
}
