//! Owners (ownership design O1 §2): zygote-forked isolates in the role
//! `owner`, which hold buffers and processes for the session. This module
//! holds what Rust keeps of them: liveness by ping, and an owner's own flag.
//! Their lifecycle is Lisp's (`lisp/vm/owner.el`, `lisp/vm/isolate.el`).
//!
//! Liveness: while the spawner has a call outstanding on an owner, or a
//! ping unanswered, it sends a `ping` every half `owner-wedge-delay`. The
//! owner answers with a `pong` from every wait it makes, in Rust
//! (`Vm::poll_for`, `Vm::isolate_wait`): a turn blocked in a wait is never
//! wedged. One whose oldest unanswered ping is older than the delay is: a
//! Lisp loop that never waits, or a native hang. A pong answers its ping and
//! every earlier one, and pings go on while one is unanswered, so a lost
//! ping or pong costs half a delay, never a wedge for good. Until the
//! publication socket (O1a-3b) both frames ride the control socket.
use super::{Result, Vm};
use std::collections::VecDeque;
use std::time::Duration;

/// Unanswered pings kept apart at most. Past it the oldest entry takes in
/// the next, once that one is past its deadline too: a pong can then leave
/// nothing of the run not yet due, so the judgement stays exact.
const KEPT: usize = 4;

/// An owner's liveness, as its spawner sees it.
#[derive(Default)]
pub(crate) struct Ping {
    /// The last sequence number sent.
    seq: u64,
    /// The last sequence number a pong answered.
    acked: u64,
    /// When the last ping went.
    last: Option<std::time::Instant>,
    /// The unanswered pings, oldest first: the last sequence number an
    /// entry stands for, and its first's deadline, when it went plus the
    /// delay then. A ping is judged by the delay it went under: a changed
    /// `owner-wedge-delay` holds for the pings sent after it.
    unanswered: VecDeque<(u64, std::time::Instant)>,
}

impl Ping {
    /// A pong for SEQ arrived: it answers that ping and every earlier one.
    /// One for a ping never sent, or older than one already answered, is
    /// no news.
    pub(crate) fn answered(&mut self, seq: u64) {
        if seq > self.seq || seq <= self.acked { return; }
        self.acked = seq;
        while self.unanswered.front().is_some_and(|&(last, _)| last <= seq) {
            self.unanswered.pop_front();
        }
    }

    /// Wedged at NOW: its oldest unanswered ping is past its deadline.
    fn wedged(&self, now: std::time::Instant) -> bool {
        self.unanswered.front().is_some_and(|&(_, deadline)| now >= deadline)
    }

    /// When the next ping is due: while BUSY (a call is outstanding) or a
    /// ping is unanswered, every half DELAY; else never.
    fn due_at(&self, now: std::time::Instant, delay: Duration, busy: bool) -> Option<std::time::Instant> {
        (busy || !self.unanswered.is_empty()).then(|| self.last.map_or(now, |last| last + delay / 2))
    }

    /// A ping went at NOW, to be answered within DELAY: its sequence number.
    fn went(&mut self, now: std::time::Instant, delay: Duration) -> u64 {
        self.seq += 1;
        self.last = Some(now);
        self.unanswered.push_back((self.seq, now + delay));
        self.seq
    }

    /// Past `KEPT` entries, the oldest takes in the next while that one is
    /// past its deadline at NOW too.
    fn fold(&mut self, now: std::time::Instant) {
        while self.unanswered.len() > KEPT && now >= self.unanswered[1].1 {
            if let Some((last, _)) = self.unanswered.remove(1) { self.unanswered[0].0 = last; }
        }
    }

    /// How long from NOW until it next needs a look: the next ping, or its
    /// wedge judged; None when nothing is coming.
    fn next(&self, now: std::time::Instant, delay: Duration, busy: bool) -> Option<Duration> {
        let wedge = self.unanswered.front().map(|&(_, deadline)| deadline).filter(|&at| at > now);
        [self.due_at(now, delay, busy), wedge].into_iter().flatten().min().map(|at| at.saturating_duration_since(now))
    }
}

/// What `Vm::owner_ping` reports.
pub(crate) struct PingState {
    pub(crate) wedged: bool,
    pub(crate) next: Option<Duration>,
    pub(crate) sent: u64,
    pub(crate) answered: u64,
}

impl Vm {
    /// In a child forked to be an owner: it serves for the session, not one
    /// job, so its job budget goes, and from now every wait it makes
    /// answers its spawner's pings. Refused in a VM with no spawner.
    pub(crate) fn owner_become(&mut self) -> Result<()> {
        if !self.isolate_has_parent() { return Err("Only a worker isolate can serve as an owner".into()); }
        self.clear_job_budget();
        self.isolates.owner = true;
        Ok(())
    }

    /// In an owner: the spawner's socket, which its waits poll to answer
    /// pings; None elsewhere, or once that socket closed.
    pub(crate) fn owner_control_fd(&self) -> Option<std::os::fd::RawFd> {
        if !self.isolates.owner { return None; }
        self.isolate_parent_socket()
    }

    /// In an owner whose wait saw its spawner's socket readable: answer the
    /// pings it holds, leaving every other frame queued for its turn.
    pub(crate) fn owner_answer_pings(&mut self) {
        let _ = self.isolate_drain_parent();
    }

    /// Owner ID's liveness at DELAY (`owner-wedge-delay`), judged now; its
    /// next ping goes if one is due, BUSY saying a call is outstanding. A
    /// ping that finds the socket full counts as sent: an owner that does
    /// not read is the wedge it measures.
    pub(crate) fn owner_ping(&mut self, id: u64, delay: Duration, busy: bool) -> Result<PingState> {
        let now = crate::host::monotonic();
        let worker = self.isolate_worker_mut(id)?;
        if worker.ping.due_at(now, delay, busy).is_some_and(|at| at <= now) {
            let seq = worker.ping.went(now, delay);
            worker.ping.fold(now);
            worker.send_ping(seq);
        }
        let ping = &worker.ping;
        Ok(PingState { wedged: ping.wedged(now), next: ping.next(now, delay, busy), sent: ping.seq, answered: ping.acked })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: std::time::Instant, ms: u64) -> std::time::Instant { start + Duration::from_millis(ms) }

    fn due(ping: &Ping, now: std::time::Instant, busy: bool) -> bool {
        ping.due_at(now, Duration::from_secs(1), busy).is_some_and(|at| at <= now)
    }

    #[test]
    fn the_oldest_ping_unanswered_for_the_delay_is_a_wedge_and_a_later_pong_clears_it() {
        let (start, delay) = (crate::host::monotonic(), Duration::from_secs(1));
        let mut ping = Ping::default();
        assert!(due(&ping, start, true), "the first ping goes at once");
        let first = ping.went(start, delay);
        assert!(!due(&ping, at(start, 499), true));
        assert!(due(&ping, at(start, 500), true), "every half delay, answered or not");
        let second = ping.went(at(start, 500), delay);
        assert!(!ping.wedged(at(start, 999)));
        assert!(ping.wedged(at(start, 1000)), "the oldest is a delay old");
        ping.answered(first);
        assert!(!ping.wedged(at(start, 1000)), "the second is young");
        assert!(ping.wedged(at(start, 1500)));
        ping.answered(second);
        assert!(!ping.wedged(at(start, 1500)), "a pong answers every earlier ping");
        assert_eq!((ping.seq, ping.acked), (2, 2));
    }

    #[test]
    fn a_pong_never_sent_for_or_older_than_one_answered_is_no_news() {
        let (start, delay) = (crate::host::monotonic(), Duration::from_secs(1));
        let mut ping = Ping::default();
        let first = ping.went(start, delay);
        let second = ping.went(at(start, 500), delay);
        ping.answered(second + 7);
        assert!(ping.wedged(at(start, 1000)), "a pong beyond the last ping sent is refused");
        ping.answered(second);
        ping.answered(first);
        assert_eq!((ping.seq, ping.acked), (2, 2), "a late older pong moves nothing back");
        assert!(!ping.wedged(at(start, 5000)));
    }

    #[test]
    fn a_lost_final_ping_is_asked_again_once_calls_ended() {
        let (start, delay) = (crate::host::monotonic(), Duration::from_secs(1));
        let mut ping = Ping::default();
        let first = ping.went(start, delay);
        ping.answered(first);
        // The call's last ping is lost (or its pong is), and the call ends.
        ping.went(at(start, 500), delay);
        assert!(ping.wedged(at(start, 1500)));
        assert!(due(&ping, at(start, 1500), false), "unanswered: pinged though no call is outstanding");
        let again = ping.went(at(start, 1500), delay);
        ping.answered(again);
        assert!(!ping.wedged(at(start, 1600)), "the owner answers the next one");
        assert!(!due(&ping, at(start, 9000), false), "all answered and no call: no more pings");
        assert_eq!(ping.next(at(start, 9000), delay, false), None);
    }

    #[test]
    fn a_late_partial_pong_keeps_the_oldest_unanswered_deadline() {
        let (start, delay) = (crate::host::monotonic(), Duration::from_secs(1));
        let mut ping = Ping::default();
        let first = ping.went(start, delay);
        let second = ping.went(at(start, 500), delay);
        ping.went(at(start, 1000), delay);
        ping.went(at(start, 1500), delay);
        ping.answered(first);
        assert!(ping.wedged(at(start, 1500)),
                "the second ping is still unanswered and already 1 s old");
        ping.answered(second);
        assert!(!ping.wedged(at(start, 1500)),
                "acknowledging the second ping leaves only younger pings");
    }

    #[test]
    fn a_long_wedge_keeps_few_entries_and_stays_exact() {
        let (start, delay) = (crate::host::monotonic(), Duration::from_secs(1));
        let mut ping = Ping::default();
        for i in 0..200 { ping.went(at(start, i * 500), delay); ping.fold(at(start, i * 500)); }
        assert!(ping.unanswered.len() <= KEPT + 1, "{}", ping.unanswered.len());
        ping.answered(100);
        assert!(ping.wedged(at(start, 99_600)), "what is left of the merged run is past its wedge");
        ping.answered(200);
        assert!(!ping.wedged(at(start, 99_600)));
        assert_eq!((ping.seq, ping.acked), (200, 200));
    }

    #[test]
    fn a_ping_is_judged_by_the_delay_it_went_under() {
        let start = crate::host::monotonic();
        let (short, long) = (Duration::from_millis(10), Duration::from_secs(1));
        let mut ping = Ping::default();
        for i in 0..100 { ping.went(at(start, i * 10), short); ping.fold(at(start, i * 10)); }
        ping.answered(89);
        assert!(ping.wedged(at(start, 1000)), "unanswered past the delay it went under");
        let late = ping.went(at(start, 1000), long);
        ping.answered(100);
        assert!(!ping.wedged(at(start, 1500)), "a raised delay holds for the pings after it");
        assert!(ping.wedged(at(start, 2000)));
        ping.answered(late);
        assert!(!ping.wedged(at(start, 2000)));
    }

    #[test]
    fn looks_come_for_the_next_ping_or_the_wedge() {
        let (start, delay) = (crate::host::monotonic(), Duration::from_secs(1));
        let mut ping = Ping::default();
        assert_eq!(ping.next(start, delay, false), None, "nothing outstanding and none asked: no look");
        assert_eq!(ping.next(start, delay, true), Some(Duration::ZERO));
        ping.went(start, delay);
        assert_eq!(ping.next(at(start, 100), delay, true), Some(Duration::from_millis(400)), "the next ping");
        assert_eq!(ping.next(at(start, 100), delay, false), Some(Duration::from_millis(400)), "unanswered: still pinged");
        assert_eq!(ping.next(at(start, 5000), delay, false), Some(Duration::ZERO), "wedged: still probed");
    }
}
