//! Where an owner's pane pictures reach the render thread (design O1b.md
//! rev 3, §6.2; typing-modes o1b3/DESIGN.md). The master's ownership
//! receiver posts each picture it decoded; the render thread takes them
//! before it drains its command channel, checks each against the root
//! frame's `ForeignPaneEntry` and draws the one it composes. In memory:
//! no transport, no bytes.
//!
//! Credits come back through one callback, [`PaneSettled`], installed with
//! the sink: called once per picture when the pane path is done with it
//! (taken for composition, superseded in the mailbox or the held slot, or
//! dropped), on the render thread, outside any lock; once the render thread
//! has closed the sink, synchronously on the posting thread.
use crate::frame_time::EventTime;
use crate::pane_picture::{PaneHead, PanePicture};
use crate::pane_revisions::PaneRevisions;
use std::sync::Arc;

/// The render thread's pane mailbox, as the ownership side posts to it.
pub trait PaneSink: Send + Sync {
    /// PICTURE from OWNER, decoded and validated, whose window's view
    /// counts its revisions in REVISIONS: newest wins per (owner, window),
    /// and the render thread wakes. The render thread calls
    /// `revisions.mark_composed(v)` for the picture it swaps in.
    fn post(&self, owner: u64, picture: PanePicture, revisions: Arc<PaneRevisions>);

    /// OWNER's state as the over-pane hook sees it, from now on. Called by
    /// ownership (its turn marks and pings), not by the picture receiver.
    fn state(&self, owner: u64, state: PaneState);
}

/// Called with a picture's owner and head once the pane path is done with
/// it: on the render thread, outside any lock. Once the render thread has
/// closed the sink (as it exits), a post settles synchronously on the
/// posting thread, and may overlap the callbacks of the exit's final drain.
pub type PaneSettled = Box<dyn Fn(u64, PaneHead) + Send + Sync>;

/// An owner's state, for what draws over its panes (ownership's veil).
/// The render thread keeps the last one per owner and does not read it.
///
/// `code`'s low byte is the kind ([`PaneState::NONE`], [`PaneState::BUSY`],
/// [`PaneState::WEDGED`], [`PaneState::DEAD`]). The bits above it are the
/// rungs the veil may offer: [`PaneState::QUITS`], bit 8, when C-g quits (a
/// quit reaches the owner); bits 9 and up are reserved (owner-log's "restart
/// from checkpoint" and "abort back to").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneState {
    pub code: u32,
    /// When the state began, on the render thread's frame clock: a hook
    /// ages it with `FrameSample::since_at_presentation`.
    pub since: EventTime,
}

impl PaneState {
    pub const NONE: u32 = 0;
    pub const BUSY: u32 = 1;
    pub const WEDGED: u32 = 2;
    pub const DEAD: u32 = 3;
    /// C-g quits the owner.
    pub const QUITS: u32 = 1 << 8;

    /// The kind: `code`'s low byte.
    #[must_use]
    pub fn kind(self) -> u32 {
        self.code & 0xff
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_states_kind_is_its_low_byte_and_its_rungs_lie_above() {
        let state = PaneState { code: PaneState::WEDGED | PaneState::QUITS, since: crate::frame_time::observe_platform_now() };
        assert_eq!(state.kind(), PaneState::WEDGED);
        assert_ne!(state.code & PaneState::QUITS, 0);
    }
}
