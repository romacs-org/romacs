//! callproc.c's wait for a synchronous child (`call-process`): its output
//! read until the pipe closes, then its end, each wait quittable; a quit's
//! stop of the child (SIGINT to its group, a second C-g's SIGKILL, a
//! worker's grace) and its reaping; and the quit raised again once the
//! output read before it is inserted. `process.rs` spawns the child.
use super::{Fault, Leaf, Value};
use crate::subprocess as sp;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};

/// The tokens of the quits call-process's waits took and returned with the
/// output read before them, ((quit . TOKEN) . BYTES), not yet raised again
/// nor retired: a list in a hidden variable, which the collector traces.
const REQUITS: &str = "#:vm2-call-process-quits";

type Result<T> = std::result::Result<T, Fault>;

/// What became of call-process's child when its quit's stop ended: reaped
/// (dead, its status taken), or handed, still alive, to the reaper of deleted
/// children, which reports its death by no longer owning it
/// (`subprocess::Table::owns`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildEnd {
    Reaped,
    HandedOff,
}

impl Leaf<'_> {
    /// `call_process`'s wait for child PID: what it wrote to READER until the
    /// pipe closed, then its end, each a quittable wait (`Vm::poll_quittable`).
    /// A C-g ends the child as `call_process_cleanup` does
    /// (`call_process_stop`) and ends the wait with the quit beside what was
    /// read before it; under `inhibit-quit` the wait goes on without looking
    /// at the press again.
    pub(super) fn call_process_wait(&mut self, pid: i32, mut reader: Option<OwnedFd>) -> Result<(Vec<u8>, Option<Fault>)> {
        let pidfd = crate::host::pidfd_open(pid);
        let mut bytes = Vec::new();
        let mut buf = vec![0; 64 * 1024];
        let mut held = false;
        loop {
            // A worker's interrupt or termination stops the child as a C-g
            // does, asked before each wait: one an `inhibit-quit' since
            // unbound held, or one whose wake ended the last wait.
            if let Some(quit) = self.vm.worker_quit() {
                return Ok((bytes, Some(self.call_process_quit(pid, pidfd.as_ref(), quit, Some(Self::WORKER_GRACE)))));
            }
            let (fds, tick) = match (&reader, &pidfd) {
                (Some(read), _) => (vec![(read.as_raw_fd(), libc::POLLIN)], None),
                (None, Some(pidfd)) => (vec![(pidfd.as_raw_fd(), libc::POLLIN)], None),
                // No pidfd (pidfd_open failed): the child's end is asked
                // every tick, left for the caller's wait to reap.
                (None, None) if sp::exited(pid) => break,
                (None, None) => (Vec::new(), Some(Self::CHILD_TICK)),
            };
            let revents = if held { sp::poll(&fds, tick) } else { self.vm.poll_quittable(&fds, tick) }
                .map_err(|e| self.file_errno("Waiting for process", &e))?;
            if revents.first().is_none_or(|&r| r == 0) {
                if let Err(quit) = self.native_poll() {
                    return Ok((bytes, Some(self.call_process_quit(pid, pidfd.as_ref(), quit, None))));
                }
                held = self.vm.quit.pressed();
                continue;
            }
            let Some(read) = &reader else { break };
            match sp::read(read.as_fd(), &mut buf) {
                Ok(0) | Err(_) => reader = None,
                Ok(n) => bytes.extend_from_slice(&buf[..n]),
            }
        }
        Ok((bytes, None))
    }

    /// The quit call-process's wait took, raised again once the output read
    /// before it is inserted (`romacs--call-process-quit`): C-g's, as
    /// `maybe_quit` raised it, so the expansion it leaves defers it
    /// (`quit_watched`). Only TOKEN, the wait's own and not yet raised,
    /// makes it C-g's, and is taken: any other (from a macro, say, or a
    /// token already raised) signals a plain quit, which an expansion treats
    /// as any error.
    pub(crate) fn call_process_requit(&mut self, token: Value) -> Fault {
        if self.take_requit(token) { self.vm.quit_unwinding = true; }
        Fault::Signal(self.sym("quit"), Value::NIL)
    }

    /// A new token for a quit a call-process wait took and returns with the
    /// output read before it: a fresh cons, known by `eq` alone, so no
    /// number or other value stands for it. It is outstanding (in
    /// [`REQUITS`]) until raised again or retired.
    pub(crate) fn issue_requit(&mut self) -> Value {
        let token = self.cons(Value::NIL, Value::NIL);
        let outstanding = self.hidden_value(REQUITS);
        let outstanding = self.cons(token, outstanding);
        self.set_hidden_value(REQUITS, outstanding);
        token
    }

    /// Whether TOKEN is outstanding, which it then is not.
    pub(crate) fn take_requit(&mut self, token: Value) -> bool {
        let (mut kept, mut found, mut tail) = (Vec::new(), false, self.hidden_value(REQUITS));
        while let Some((car, cdr)) = self.pair(tail) {
            if car == token { found = true } else { kept.push(car) }
            tail = cdr;
        }
        if found {
            let rest = kept.into_iter().rev().fold(Value::NIL, |list, t| self.cons(t, list));
            self.set_hidden_value(REQUITS, rest);
        }
        found
    }

    /// QUIT, raised while `call_process` waits for child PID, once the child
    /// is stopped (`call_process_stop`, GRACE as it takes it).
    fn call_process_quit(&mut self, pid: i32, pidfd: Option<&OwnedFd>, quit: Fault, grace: Option<std::time::Duration>) -> Fault {
        // A4's abort offer reads this: HandedOff leaves a live child owned
        // by the reaper until `Table::owns` says not.
        let end = self.call_process_stop(pid, pidfd, grace);
        debug_assert!(end == ChildEnd::Reaped || self.vm.subprocesses.owns(pid), "a handed-off child has one owner");
        quit
    }

    /// `call_process_cleanup` for a quit: SIGINT to the child's process group
    /// (its own, made by spawn's setsid), then a wait for it to die, which a
    /// further C-g ends with SIGKILL (`call_process_kill`), as "If you quit,
    /// the process is killed with SIGINT, or SIGKILL if you quit again". The
    /// child is reaped here, or, if SIGKILL has not ended it within
    /// `KILL_GRACE` (a child in uninterruptible sleep), left to the reaper of
    /// deleted children (`reap_children`, the host's reaper once the VM
    /// goes), with no status made up for it: the quit returns either way.
    /// Without a pidfd (PIDFD None) the child's end is asked every
    /// `CHILD_TICK`, so the wait still hears the stop. A worker's quit, which
    /// no second C-g follows, gives GRACE ([`Self::WORKER_GRACE`]) for the
    /// SIGINT before it sends SIGKILL by itself, as the second C-g would;
    /// the main VM's quit gives None and waits for that C-g. What became of
    /// the child is the answer (`ChildEnd`).
    pub(crate) fn call_process_stop(&mut self, pid: i32, pidfd: Option<&OwnedFd>, grace: Option<std::time::Duration>) -> ChildEnd {
        let kill = |signal| if crate::host::kill(-pid, signal).is_err() { let _ = crate::host::kill(pid, signal); };
        kill(libc::SIGINT);
        let fds: Vec<_> = pidfd.map(|fd| (fd.as_raw_fd(), libc::POLLIN)).into_iter().collect();
        let tick = pidfd.is_none().then_some(Self::CHILD_TICK);
        let deadline = grace.map(|grace| crate::host::monotonic() + grace);
        while !sp::exited(pid) {
            let left = deadline.map(|end| end.saturating_duration_since(crate::host::monotonic()));
            if left.is_some_and(|left| left.is_zero()) {
                kill(libc::SIGKILL);
                break;
            }
            let timeout = match (tick, left) { (Some(tick), Some(left)) => Some(tick.min(left)), (tick, left) => tick.or(left) };
            match self.vm.poll_quittable(&fds, timeout) {
                Ok(_) if self.call_process_stop_requested() => {
                    kill(libc::SIGKILL);
                    break;
                }
                Ok(_) => {}
                // A wait that cannot go on: a stop with a grace leaves no
                // child unkilled.
                Err(_) => {
                    if grace.is_some() { kill(libc::SIGKILL); }
                    break;
                }
            }
        }
        // A worker's stop ends the whole group, not its leader alone: a
        // helper the leader left behind (a shell starts its `&' jobs with
        // SIGINT ignored) would outlive the cancel. The leader is not reaped
        // yet (`sp::exited' waits with WNOWAIT), so its group id cannot have
        // been reused, and this SIGKILL reaches only the child's own group.
        if grace.is_some() { kill(libc::SIGKILL); }
        self.call_process_reap(pid, &fds, Self::KILL_GRACE)
    }

    /// Child PID reaped within GRACE, FDS (its pidfd, if any) waited on and
    /// its end asked every `CHILD_TICK`; or, still alive at the deadline,
    /// handed to the reaper of deleted children, its one owner from then
    /// on, with no status made up for it.
    pub(crate) fn call_process_reap(&mut self, pid: i32, fds: &[(std::os::fd::RawFd, i16)], grace: std::time::Duration) -> ChildEnd {
        let end = crate::host::monotonic() + grace;
        while !sp::exited(pid) {
            let left = end.saturating_duration_since(crate::host::monotonic());
            if left.is_zero() { break; }
            let _ = sp::poll(fds, Some(left.min(Self::CHILD_TICK)));
        }
        if sp::exited(pid) {
            // Dead, or no child of ours any more: nothing left to own.
            let _ = sp::wait_blocking(pid);
            return ChildEnd::Reaped;
        }
        self.vm.subprocesses.deleted.push((pid, None));
        ChildEnd::HandedOff
    }

    /// How long a quit waits for a killed child to end before it leaves
    /// the child to the reaper and returns.
    const KILL_GRACE: std::time::Duration = std::time::Duration::from_millis(100);

    /// How long a worker's quit gives its child's SIGINT before SIGKILL: well
    /// inside `isolate-interrupt-timeout`, the spawner's own escalation.
    const WORKER_GRACE: std::time::Duration = std::time::Duration::from_millis(200);

    /// How often a wait without a pidfd asks whether the child has ended.
    const CHILD_TICK: std::time::Duration = std::time::Duration::from_millis(10);

    /// Whether the wait after the quit's SIGINT should kill the child now
    /// (SIGKILL, reap, return), the request taken: today a second C-g, taken
    /// here as GNU's kill takes it. The one question the wait asks, so a
    /// later stop (an abort) extends this and not the loop.
    fn call_process_stop_requested(&mut self) -> bool {
        let pressed = self.vm.quit.pressed();
        if pressed { self.vm.quit.take(); }
        pressed
    }
}
