//! The pane mailbox (`romacs_display_protocol::pane_sink`): what the
//! ownership side posted since the render thread last took it. Newest wins
//! per (owner, window); a picture it replaces is kept for the render thread
//! to settle, so every settle runs there, outside the lock. Once the render
//! thread closes it (at exit), a post settles at once, on the poster's
//! thread: no picture is ever left without its credit.
use romacs_display_protocol::pane_picture::{PaneHead, PanePicture};
use romacs_display_protocol::pane_revisions::PaneRevisions;
use romacs_display_protocol::pane_sink::{PaneSettled, PaneSink, PaneState};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

/// A posted picture, with its owner and its view's revisions.
pub(crate) struct Posted {
    pub owner: u64,
    pub picture: PanePicture,
    pub revisions: Arc<PaneRevisions>,
}

/// What one render round takes.
#[derive(Default)]
pub(crate) struct Taken {
    pub pictures: Vec<Posted>,
    /// Pictures a newer one replaced before the round took them.
    pub superseded: Vec<(u64, PaneHead)>,
    /// Owners' states, in the order posted.
    pub states: Vec<(u64, PaneState)>,
}

#[derive(Default)]
struct Inbox {
    pictures: HashMap<(u64, u64), Posted>,
    superseded: Vec<(u64, PaneHead)>,
    states: Vec<(u64, PaneState)>,
    /// The render thread is gone: nothing more is taken.
    closed: bool,
}

impl Inbox {
    fn take(&mut self) -> Taken {
        Taken {
            pictures: self.pictures.drain().map(|(_, posted)| posted).collect(),
            superseded: std::mem::take(&mut self.superseded),
            states: std::mem::take(&mut self.states),
        }
    }
}

#[derive(Default)]
pub(crate) struct PaneMailbox {
    inbox: Mutex<Inbox>,
    /// Something was posted since the last take: a round with nothing to
    /// take takes no lock.
    posted: AtomicBool,
    settled: OnceLock<PaneSettled>,
    /// Run at each take, before the swap: a test's racing poster.
    #[cfg(test)]
    pub(crate) before_take: Mutex<Option<Box<dyn FnMut() + Send>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl PaneMailbox {
    /// Install SETTLED; false, and nothing installed, when one is.
    pub(crate) fn install(&self, settled: PaneSettled) -> bool {
        self.settled.set(settled).is_ok()
    }

    /// The pane path is done with OWNER's picture HEAD.
    pub(crate) fn settle(&self, owner: u64, head: PaneHead) {
        if let Some(settled) = self.settled.get() {
            settled(owner, head);
        }
    }

    /// Everything posted since the last take.
    pub(crate) fn take(&self) -> Taken {
        #[cfg(test)]
        if let Some(poster) = lock(&self.before_take).as_mut() {
            poster();
        }
        if !self.posted.swap(false, Ordering::Acquire) {
            return Taken::default();
        }
        lock(&self.inbox).take()
    }

    /// Close the mailbox, as the render thread exits: everything posted
    /// and not taken, for the caller to settle. A later post settles at once.
    pub(crate) fn close(&self) -> Taken {
        let mut inbox = lock(&self.inbox);
        inbox.closed = true;
        inbox.take()
    }
}

impl PaneSink for PaneMailbox {
    fn post(&self, owner: u64, picture: PanePicture, revisions: Arc<PaneRevisions>) {
        let (key, head) = ((owner, picture.head.window), picture.head);
        let mut inbox = lock(&self.inbox);
        if inbox.closed {
            drop(inbox);
            self.settle(owner, head);
            return;
        }
        if let Some(old) = inbox.pictures.insert(key, Posted { owner, picture, revisions }) {
            inbox.superseded.push((old.owner, old.picture.head));
        }
        // Set under the lock, after the insert: a take that sees it finds the picture.
        self.posted.store(true, Ordering::Release);
    }

    fn state(&self, owner: u64, state: PaneState) {
        let mut inbox = lock(&self.inbox);
        if !inbox.closed {
            inbox.states.push((owner, state));
            self.posted.store(true, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A round with nothing posted takes no lock: here the lock is held,
    /// and the take still returns.
    #[test]
    fn a_take_with_nothing_posted_takes_no_lock() {
        let mailbox = PaneMailbox::default();
        let held = lock(&mailbox.inbox);
        let taken = mailbox.take();
        assert!(taken.pictures.is_empty() && taken.superseded.is_empty() && taken.states.is_empty());
        drop(held);
    }
}
