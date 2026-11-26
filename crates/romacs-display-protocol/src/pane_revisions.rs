//! A foreign pane's picture revisions (ledger design O1 §3.3, "K2's
//! trigger; O1 adds none of its own"): the master's displayed view record
//! of a window showing a stand-in holds one, shared as
//! `Arc<PaneRevisions>` with the ownership receiver, which counts the
//! pictures it accepts for the window's current view, and the render
//! thread, which counts the ones it took for composition. One is made per
//! window id, owner incarnation and view generation: a new view or
//! incarnation gets a fresh one at 0, so no reader resets it.
//!
//! Each store is Release and each load Acquire: a reader that sees a
//! revision sees what was posted before it was stored (the picture in the
//! pane mailbox). A store wakes no one: the evaluator reads them when it
//! is awake for its own reasons.
use std::sync::atomic::{AtomicU64, Ordering};

/// The newest picture an owner published for a view and the master
/// accepted, and the newest the render thread took for composition.
#[derive(Debug, Default, romacs_snapshot::Snapshot)]
pub struct PaneRevisions {
    owner_picture: AtomicU64,
    composed: AtomicU64,
}

impl PaneRevisions {
    /// The ownership receiver accepted picture REV for the view, after
    /// posting it to the pane mailbox.
    pub fn bump_picture(&self, rev: u64) {
        self.owner_picture.store(rev, Ordering::Release);
    }
    /// The render thread took picture REV for composition (`ingest_pane`).
    pub fn mark_composed(&self, rev: u64) {
        self.composed.store(rev, Ordering::Release);
    }
    /// The newest picture accepted for the view.
    pub fn picture(&self) -> u64 {
        self.owner_picture.load(Ordering::Acquire)
    }
    /// The newest picture taken for composition.
    pub fn composed(&self) -> u64 {
        self.composed.load(Ordering::Acquire)
    }
    /// Whether a picture was accepted that the render thread has not yet
    /// taken for composition.
    pub fn pending(&self) -> bool {
        self.picture() > self.composed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_picture_bumped_on_another_thread_is_read_with_what_it_posted() {
        let revisions = Arc::new(PaneRevisions::default());
        let posted = Arc::new(AtomicU64::new(0));
        let (theirs, mailbox) = (revisions.clone(), posted.clone());
        std::thread::spawn(move || {
            mailbox.store(42, Ordering::Relaxed);
            theirs.bump_picture(1);
        }).join().unwrap();
        assert_eq!(revisions.picture(), 1);
        assert_eq!(posted.load(Ordering::Relaxed), 42, "what was posted before the bump is seen");
    }

    #[test]
    fn pending_until_composed_catches_up() {
        let revisions = PaneRevisions::default();
        assert!(!revisions.pending(), "a fresh one is not pending");
        revisions.bump_picture(2);
        assert!(revisions.pending());
        revisions.mark_composed(1);
        assert!(revisions.pending());
        revisions.mark_composed(2);
        assert!(!revisions.pending());
    }
}
