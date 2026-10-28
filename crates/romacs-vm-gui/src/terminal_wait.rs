use crossbeam_channel::{Receiver, Sender, bounded};
use romacs_display_runtime::thread_comm::InputEvent;
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub(crate) struct Mailbox {
    owners: Arc<Mutex<BTreeSet<u64>>>,
    signal: Sender<()>,
    ready: Receiver<()>,
}

impl Default for Mailbox {
    fn default() -> Self {
        let (signal, ready) = bounded(1);
        Self { owners: Arc::new(Mutex::new(BTreeSet::new())), signal, ready }
    }
}

impl Mailbox {
    pub(crate) fn mark(&self, owner: u64) {
        let mut owners = self.owners.lock().unwrap();
        owners.insert(owner);
        self.wake();
    }

    /// Wake the wait for native work that is not a terminal's (a completed
    /// background command); the wait's caller polls that work itself.
    pub(crate) fn wake(&self) { let _ = self.signal.try_send(()); }

    pub(crate) fn take(&self) -> Vec<u64> {
        let mut owners = self.owners.lock().unwrap();
        let result = std::mem::take(&mut *owners).into_iter().collect();
        while self.ready.try_recv().is_ok() {}
        result
    }

    pub(crate) fn ready(&self) -> Receiver<()> { self.ready.clone() }
}

pub(crate) enum Wake {
    Input(InputEvent),
    Terminal,
    Timeout,
    Closed,
}

pub(crate) fn wait(input: &Receiver<InputEvent>, terminal: &Receiver<()>, timeout: Option<Duration>) -> Wake {
    let deadline = timeout.map(crossbeam_channel::after).unwrap_or_else(crossbeam_channel::never);
    crossbeam_channel::select_biased! {
        recv(input) -> event => event.map(Wake::Input).unwrap_or(Wake::Closed),
        recv(terminal) -> event => if event.is_ok() { Wake::Terminal } else { Wake::Closed },
        recv(deadline) -> _ => Wake::Timeout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalescing_retains_every_owner_and_rearms_after_a_wait() {
        let mailbox = Mailbox::default();
        let ready = mailbox.ready();
        for _ in 0..100 { mailbox.mark(1); }
        mailbox.mark(2);
        assert_eq!(ready.len(), 1);
        ready.recv().unwrap();
        mailbox.mark(3);
        assert_eq!(mailbox.take(), vec![1, 2, 3], "COALESCING_MUST_RETAIN_EVERY_TERMINAL_OWNER");
        assert!(ready.is_empty());
        mailbox.mark(2);
        assert!(ready.try_recv().is_ok(), "DRAINED_MAILBOX_MUST_REARM");
        assert_eq!(mailbox.take(), vec![2]);
    }

    #[test]
    fn drain_and_producer_interleavings_do_not_lose_owners() {
        let mailbox = Mailbox::default();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let producer = mailbox.clone();
        let gate = barrier.clone();
        let child = std::thread::spawn(move || {
            for id in 1..=1000 {
                gate.wait();
                producer.mark(id);
                gate.wait();
            }
        });
        for id in 1..=1000 {
            barrier.wait();
            let mut seen = mailbox.take();
            barrier.wait();
            seen.extend(mailbox.take());
            assert_eq!(seen, vec![id], "CONCURRENT_DRAIN_MUST_NOT_LOSE_OR_DUPLICATE_OWNER");
            mailbox.mark(id);
            assert!(mailbox.ready().try_recv().is_ok());
            assert_eq!(mailbox.take(), vec![id]);
        }
        child.join().unwrap();
    }

    #[test]
    fn input_has_priority_and_idle_waits_do_not_invent_terminal_events() {
        let mailbox = Mailbox::default();
        let ready = mailbox.ready();
        let (keys, input) = bounded(1);
        assert!(matches!(wait(&input, &ready, Some(Duration::ZERO)), Wake::Timeout));
        mailbox.mark(1);
        keys.send(InputEvent::WindowClose { emacs_frame_id: 0 }).unwrap();
        assert!(matches!(wait(&input, &ready, None), Wake::Input(InputEvent::WindowClose { .. })));
        assert!(matches!(wait(&input, &ready, None), Wake::Terminal));
        assert!(matches!(wait(&input, &ready, Some(Duration::ZERO)), Wake::Timeout), "CONSUMED_WAKE_MUST_NOT_SPIN_WITH_STALE_OWNER");
        assert_eq!(mailbox.take(), vec![1]);
        drop(keys);
        assert!(matches!(wait(&input, &ready, None), Wake::Closed));
    }
}
