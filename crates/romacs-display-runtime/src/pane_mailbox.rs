//! The pane mailbox (`romacs_display_protocol::pane_sink`): what the
//! ownership side posted since the render thread last took it. Newest wins
//! per (owner, window); a picture it replaces is kept for the render thread
//! to settle, so every settle runs there, outside the lock. Once the render
//! thread closes it (at exit), or once an owner's DEAD state was posted, a
//! post (of that owner's) settles at once, on the poster's thread: no
//! picture is ever left without its credit, and none of a dead owner's is
//! queued.
use romacs_display_protocol::pane_picture::{PaneHead, PanePicture};
use romacs_display_protocol::pane_revisions::PaneRevisions;
use romacs_display_protocol::pane_sink::{PaneSettled, PaneSink, PaneState};
use std::collections::{HashMap, HashSet};
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
    /// Owners whose DEAD state was posted, for the mailbox's life (owner
    /// incarnations are never reused).
    dead: HashSet<u64>,
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
    /// The render command channel, where a DEAD state sends the owner's
    /// death, in order with the uploads.
    commands: OnceLock<crate::thread_comm::CommandSender>,
    /// Run at each take, before the swap: a test's racing poster.
    #[cfg(test)]
    pub(crate) before_take: Mutex<Option<Box<dyn FnMut() + Send>>>,
    /// Run at each take, after the swap: one racing the drain.
    #[cfg(test)]
    pub(crate) after_take: Mutex<Option<Box<dyn FnMut() + Send>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl PaneMailbox {
    /// Install SETTLED; false, and nothing installed, when one is.
    pub(crate) fn install(&self, settled: PaneSettled) -> bool {
        self.settled.set(settled).is_ok()
    }

    /// Send a DEAD state's `OwnerDied` on COMMANDS; false, and nothing
    /// connected, when one is.
    pub(crate) fn connect(&self, commands: crate::thread_comm::CommandSender) -> bool {
        self.commands.set(commands).is_ok()
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
        let taken = if self.posted.swap(false, Ordering::Acquire) { lock(&self.inbox).take() } else { Taken::default() };
        #[cfg(test)]
        if let Some(poster) = lock(&self.after_take).as_mut() {
            poster();
        }
        taken
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
        if inbox.closed || inbox.dead.contains(&owner) {
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

    /// A DEAD state is recorded, then `OwnerDied` sent on the command
    /// channel, behind the dead owner's uploads: the slot is released only
    /// after, so a new owner's uploads come behind it. The command is
    /// exempt from the channel's capacity: the send never waits.
    fn state(&self, owner: u64, state: PaneState) {
        let dead = state.kind() == PaneState::DEAD;
        {
            let mut inbox = lock(&self.inbox);
            if dead {
                inbox.dead.insert(owner);
            }
            if inbox.closed {
                return;
            }
            inbox.states.push((owner, state));
            self.posted.store(true, Ordering::Release);
        }
        if dead && let Some(commands) = self.commands.get() {
            let died = crate::thread_comm::AssetCommand::OwnerDied { owner, slot: state.slot };
            let _ = commands.try_send(crate::thread_comm::RenderCommand::Asset(died));
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
    /// A post of an owner already posted DEAD settles at once, on the
    /// poster's thread, and is never queued.
    #[test]
    fn a_post_after_its_owners_death_settles_at_once() {
        use romacs_display_protocol::pane_picture::{PaneFrame, PaneHead};
        let mailbox = PaneMailbox::default();
        let settled = Arc::new(Mutex::new(Vec::new()));
        let record = settled.clone();
        assert!(mailbox.install(Box::new(move |owner, head: PaneHead| record.lock().unwrap().push((owner, head.v)))));
        let since = romacs_display_protocol::frame_time::observe_platform_now();
        mailbox.state(41, PaneState { code: PaneState::DEAD, since, slot: 0 });
        let head = PaneHead { owner: 41, window: 900, view: 1, request: 1, v: 5, c: 1 };
        let frame = PaneFrame { frame: romacs_display_protocol::FrameGlyphBuffer::with_size(10.0, 10.0), animations: Vec::new(), submitted: None };
        mailbox.post(41, PanePicture::from_frame(head, frame, romacs_display_protocol::DisplayWindowId::new(5)), Arc::default());
        assert_eq!(*settled.lock().unwrap(), vec![(41, 5)]);
        assert!(mailbox.take().pictures.is_empty());
    }
    /// DEAD sends the owner's death on the command channel, besides
    /// recording the state, and never waits for room: with the channel
    /// full, the death is still taken, behind what was queued.
    #[test]
    fn a_dead_state_sends_the_death_even_on_a_full_channel() {
        use crate::thread_comm::{AssetCommand, RenderCommand};
        let mailbox = PaneMailbox::default();
        let (commands, received) = crate::thread_comm::command_channel(1);
        let queued = RenderCommand::Asset(AssetCommand::ImageRetire { image: romacs_display_protocol::ImageId::new(1) });
        commands.try_send(queued).unwrap();
        assert!(mailbox.connect(commands));
        let since = romacs_display_protocol::frame_time::observe_platform_now();
        mailbox.state(41, PaneState { code: PaneState::BUSY, since, slot: 3 });
        mailbox.state(41, PaneState { code: PaneState::DEAD, since, slot: 3 });
        assert!(matches!(received.try_recv(), Ok(RenderCommand::Asset(AssetCommand::ImageRetire { .. }))), "order kept");
        assert!(matches!(received.try_recv(), Ok(RenderCommand::Asset(AssetCommand::OwnerDied { owner: 41, slot: 3 }))));
        assert!(received.try_recv().is_err(), "a live state sends nothing");
        assert_eq!(mailbox.take().states.len(), 2);
    }
}
