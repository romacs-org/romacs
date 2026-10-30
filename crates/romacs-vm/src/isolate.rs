//! Isolates: worker VMs in child processes that exchange frozen values
//! (vm-concurrency design §4, §4.6). This is the substrate the Lisp half
//! (rlisp/vm/isolate.el) drives: spawning a worker with a control socket,
//! packets of frozen values in either direction, waiting on the sockets and
//! on the workers' pidfds, signals, and the children a worker reports so the
//! parent can reach their process groups after a kill.
//!
//! A packet is a frame: KIND, GENERATION, then either the frozen encoding
//! inline (below a page) or a sealed memfd passed beside the packet. The
//! same frame carries a job, a result, an error or a message; what the value
//! means is the Lisp half's. Values are never decoded here: a frame waits in
//! the peer's inbox as bytes or a descriptor until Lisp takes it, when it is
//! attached as a frozen region (`frozen.rs`).
//!
//! Lifecycle, as the design states it: a worker installs `PR_SET_PDEATHSIG`
//! and checks its parent's pid; SIGTERM ends its loop and drops its VM, whose
//! subprocess table sends SIGHUP to each unreaped child's group; after a
//! SIGKILL the parent signals, through the pidfds the worker reported and had
//! not retired, the original process group of each such child
//! (`PIDFD_SIGNAL_PROCESS_GROUP`, Linux 6.9+); a kernel without the flag
//! signals the leader alone and says so. SIGUSR1 is the interrupt the
//! engine's safepoints turn into `quit` (`threaded.rs`).
use super::*;
use crate::frozen::Backing;
use std::collections::VecDeque;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};

pub(crate) const PROTOCOL: u32 = 1;
const HEADER: usize = 10;
/// A packet buffer: the header, an inline frozen encoding, and slack for
/// the text frames.
const PACKET: usize = crate::frozen::INLINE_LIMIT + 1024;
const FLAG_SEALED: u8 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum Kind { Hello = 1, Job = 2, Result = 3, Error = 4, Message = 5, Child = 6, Retire = 7, Bye = 8, Died = 9, Interrupt = 10 }

impl Kind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::Hello => "hello", Kind::Job => "job", Kind::Result => "result", Kind::Error => "error", Kind::Message => "message",
            Kind::Child => "child", Kind::Retire => "retire", Kind::Bye => "bye", Kind::Died => "died", Kind::Interrupt => "interrupt",
        }
    }
    const SENT: [Kind; 9] = [Kind::Hello, Kind::Job, Kind::Result, Kind::Error, Kind::Message, Kind::Child, Kind::Retire, Kind::Bye, Kind::Interrupt];
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::SENT.into_iter().find(|k| k.name() == name)
    }
    fn of(byte: u8) -> Option<Self> {
        Self::SENT.into_iter().find(|k| *k as u8 == byte)
    }
}

pub(crate) enum Payload {
    None,
    Frozen(Backing),
    Text(String),
    Status(i32),
}

pub(crate) struct Frame {
    pub(crate) kind: Kind,
    pub(crate) generation: u64,
    pub(crate) payload: Payload,
}

/// One end of a control socket and the frames read from it.
pub(crate) struct Peer {
    socket: OwnedFd,
    inbox: VecDeque<Frame>,
    closed: bool,
}

impl Peer {
    fn new(socket: OwnedFd) -> Self {
        Peer { socket, inbox: VecDeque::new(), closed: false }
    }

    fn send(&self, kind: Kind, generation: u64, payload: &Payload) -> std::io::Result<()> {
        let mut bytes = Vec::with_capacity(HEADER + 64);
        bytes.push(kind as u8);
        bytes.extend_from_slice(&generation.to_le_bytes());
        bytes.push(0);
        let mut fd = None;
        match payload {
            Payload::None => {}
            Payload::Text(text) => bytes.extend_from_slice(text.as_bytes()),
            Payload::Status(status) => bytes.extend_from_slice(&status.to_le_bytes()),
            Payload::Frozen(Backing::Inline(inline)) => bytes.extend_from_slice(inline),
            Payload::Frozen(Backing::Sealed(file)) => { bytes[9] = FLAG_SEALED; fd = Some(file.as_fd()); }
        }
        crate::host::send_packet(self.socket.as_fd(), &bytes, fd)
    }

    /// Read one packet into the inbox: the frame, or a child report, which
    /// REPORT takes. False once the peer has closed.
    fn receive(&mut self, mut report: impl FnMut(Kind, i32, Option<OwnedFd>)) -> std::io::Result<bool> {
        let received = match crate::host::recv_packet(self.socket.as_fd(), PACKET) {
            Ok(received) => received,
            // A peer that died mid-packet is a closed peer.
            Err(e) if matches!(e.raw_os_error(), Some(libc::ECONNRESET | libc::EPIPE)) => None,
            Err(e) => return Err(e),
        };
        let Some((bytes, mut fds)) = received else {
            self.closed = true;
            return Ok(false);
        };
        if bytes.len() < HEADER { return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "short isolate frame")); }
        let kind = Kind::of(bytes[0]).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "unknown isolate frame"))?;
        let generation = u64::from_le_bytes(bytes[1..9].try_into().unwrap());
        let sealed = bytes[9] & FLAG_SEALED != 0;
        let body = &bytes[HEADER..];
        let payload = match kind {
            Kind::Child | Kind::Retire => {
                let pid = i32::from_le_bytes(body.get(..4).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "short child frame"))?.try_into().unwrap());
                report(kind, pid, fds.pop());
                return Ok(true);
            }
            Kind::Hello => Payload::Text(String::from_utf8_lossy(body).into_owned()),
            Kind::Bye | Kind::Interrupt => Payload::None,
            Kind::Job | Kind::Result | Kind::Error | Kind::Message => {
                if sealed {
                    let fd = fds.pop().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "sealed frame without a descriptor"))?;
                    Payload::Frozen(Backing::Sealed(std::sync::Arc::new(std::fs::File::from(fd))))
                } else {
                    Payload::Frozen(Backing::Inline(std::sync::Arc::new(body.to_vec())))
                }
            }
            Kind::Died => unreachable!("never sent"),
        };
        self.inbox.push_back(Frame { kind, generation, payload });
        Ok(true)
    }
}

/// A worker this VM spawned.
pub(crate) struct Worker {
    peer: Peer,
    pid: i32,
    pidfd: Option<OwnedFd>,
    /// The children it reported and has not retired: pid to pidfd.
    children: HashMap<i32, Option<OwnedFd>>,
    status: Option<i32>,
}

#[derive(Default)]
pub(crate) struct Isolates {
    workers: HashMap<u64, Worker>,
    next: u64,
    /// In a worker: the socket to its spawner.
    parent: Option<Peer>,
    /// In a worker: the generation of the job running, while one does.
    running: Option<u64>,
}

impl Isolates {
    pub(crate) fn is_empty(&self) -> bool {
        self.workers.is_empty() && self.parent.is_none()
    }
}

/// The hello a worker sends first: protocol, executable, boot.
pub(crate) fn hello(boot: &str) -> String {
    let exe = crate::host::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    format!("romacs-isolate\n{PROTOCOL}\n{exe}\n{boot}")
}

/// The worker's parent socket, for the subprocess hooks below, which run
/// outside the VM (`subprocess.rs`).
static PARENT_SOCKET: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

fn report_to_parent(kind: Kind, pid: i32, pidfd: Option<BorrowedFd<'_>>) {
    let socket = PARENT_SOCKET.load(std::sync::atomic::Ordering::SeqCst);
    if socket < 0 { return; }
    let mut bytes = Vec::with_capacity(HEADER + 4);
    bytes.push(kind as u8);
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&pid.to_le_bytes());
    // SAFETY: the socket lives in the VM's parent peer for the process's life.
    let socket = unsafe { BorrowedFd::borrow_raw(socket) };
    let _ = crate::host::send_packet(socket, &bytes, pidfd);
}

/// A child PID was spawned by this process: a worker reports it with a
/// pidfd, so its spawner can reach the child's group should the worker be
/// killed (`subprocess::spawn`).
pub(crate) fn child_spawned(pid: i32) {
    if PARENT_SOCKET.load(std::sync::atomic::Ordering::SeqCst) < 0 { return; }
    let pidfd = crate::host::pidfd_open(pid);
    report_to_parent(Kind::Child, pid, pidfd.as_ref().map(|fd| fd.as_fd()));
}

/// A child PID was reaped: its report is retired.
pub(crate) fn child_reaped(pid: i32) {
    report_to_parent(Kind::Retire, pid, None);
}

impl Vm {
    /// Spawn a worker: PROGRAM with ARGS, its id. The first frame it sends
    /// is its hello, which `isolate_take` returns for the Lisp half to check.
    pub(crate) fn isolate_start(&mut self, program: &std::path::Path, args: &[std::ffi::OsString]) -> Result<u64> {
        let (ours, theirs) = crate::host::socketpair().map_err(|e| e.to_string())?;
        let pid = crate::host::spawn_isolate(program, args, theirs).map_err(|e| format!("Cannot start isolate worker: {e}"))?;
        let pidfd = crate::host::pidfd_open(pid);
        self.isolates.next += 1;
        let id = self.isolates.next;
        self.isolates.workers.insert(id, Worker { peer: Peer::new(ours), pid, pidfd, children: HashMap::default(), status: None });
        Ok(id)
    }

    /// In a worker: descriptor FD is the socket to the spawner (peer 0).
    pub(crate) fn isolate_adopt_parent(&mut self, socket: OwnedFd) {
        PARENT_SOCKET.store(socket.as_raw_fd(), std::sync::atomic::Ordering::SeqCst);
        self.isolates.parent = Some(Peer::new(socket));
    }

    pub(crate) fn isolate_has_parent(&self) -> bool {
        self.isolates.parent.is_some()
    }

    /// In a worker: the spawner's socket has closed, so it is gone.
    pub(crate) fn isolate_parent_closed(&self) -> bool {
        self.isolates.parent.as_ref().is_some_and(|p| p.closed)
    }

    /// In a worker: the job of GENERATION runs now (None: none does).
    pub(crate) fn isolate_set_running(&mut self, generation: Option<u64>) {
        self.isolates.running = generation;
    }

    /// At an engine safepoint after SIGUSR1: whether the spawner's interrupt
    /// names the running job. The spawner sends an `interrupt' frame for a
    /// generation before the signal, so the frame is readable here: those
    /// naming the running generation are taken, and the answer is yes; one
    /// naming a queued job stays for the loop to cancel that job; a late one
    /// (its job done) is dropped. Without a frame the signal is nobody's.
    pub(crate) fn isolate_interrupt_targets_running(&mut self) -> bool {
        // No spawner to name a generation: the signal is taken as it is.
        let Some(parent) = self.isolates.parent.as_mut() else { return true };
        let Some(running) = self.isolates.running else { return false };
        while !parent.closed {
            let ready = crate::subprocess::poll(&[(parent.socket.as_raw_fd(), libc::POLLIN)], Some(std::time::Duration::ZERO)).unwrap_or_default();
            if ready.first().copied().unwrap_or(0) == 0 { break; }
            if parent.receive(|_, _, _| {}).is_err() { break; }
        }
        let before = parent.inbox.len();
        parent.inbox.retain(|frame| frame.kind != Kind::Interrupt || frame.generation != running);
        let targeted = parent.inbox.len() != before;
        // One quit per job: a safepoint after the job's body (its cleanup,
        // its handler) must not quit again.
        if targeted { self.isolates.running = None; }
        targeted
    }

    fn peer(&self, id: u64) -> Result<&Peer> {
        if id == 0 {
            self.isolates.parent.as_ref().ok_or_else(|| "this VM has no parent isolate".into())
        } else {
            self.isolates.workers.get(&id).map(|w| &w.peer).ok_or_else(|| format!("no isolate {id}"))
        }
    }

    fn peer_mut(&mut self, id: u64) -> Result<&mut Peer> {
        if id == 0 {
            self.isolates.parent.as_mut().ok_or_else(|| "this VM has no parent isolate".into())
        } else {
            self.isolates.workers.get_mut(&id).map(|w| &mut w.peer).ok_or_else(|| format!("no isolate {id}"))
        }
    }

    /// Send VALUE as a frame of KIND with GENERATION to peer ID: frozen as
    /// it is when it is a region's root, else frozen now.
    pub(crate) fn isolate_post(&mut self, id: u64, kind: Kind, generation: u64, value: Value) -> std::result::Result<(), crate::frozen::FreezeError> {
        let payload = match kind {
            Kind::Hello => Payload::Text(self.str_view(value).map(|v| String::from_utf8_lossy(v.bytes()).into_owned()).unwrap_or_default()),
            Kind::Bye | Kind::Interrupt => Payload::None,
            _ => {
                let backing = match self.frozen_backing(value) {
                    Some(backing) => backing,
                    None => {
                        let bytes = self.encode_frozen(value).map_err(crate::frozen::FreezeError::Refused)?;
                        Backing::new(bytes).map_err(|e| crate::frozen::FreezeError::Failed(e.to_string()))?
                    }
                };
                Payload::Frozen(backing)
            }
        };
        let peer = self.peer(id).map_err(crate::frozen::FreezeError::Failed)?;
        peer.send(kind, generation, &payload).map_err(|e| crate::frozen::FreezeError::Failed(format!("isolate send: {e}")))
    }

    /// The descriptors a wait on the isolates needs: every peer's socket
    /// and every live worker's pidfd, readable when something happened.
    pub(crate) fn isolate_fds(&self) -> Vec<(RawFd, i16)> {
        let mut fds = Vec::new();
        if let Some(parent) = &self.isolates.parent && !parent.closed {
            fds.push((parent.socket.as_raw_fd(), libc::POLLIN));
        }
        for worker in self.isolates.workers.values() {
            if !worker.peer.closed { fds.push((worker.peer.socket.as_raw_fd(), libc::POLLIN)); }
            if worker.status.is_none() && let Some(pidfd) = &worker.pidfd { fds.push((pidfd.as_raw_fd(), libc::POLLIN)); }
        }
        fds
    }

    /// Wait up to TIMEOUT (None: as long as it takes) for a frame or a
    /// worker's death, reading what is ready: whether anything was. In a
    /// worker, a pending interrupt or termination ends the wait at once.
    pub(crate) fn isolate_wait(&mut self, timeout: Option<std::time::Duration>) -> Result<bool> {
        if crate::host::interrupt_pending() || crate::host::terminate_pending() { return Ok(true); }
        let fds = self.isolate_fds();
        if fds.is_empty() {
            if let Some(timeout) = timeout { std::thread::sleep(timeout.min(std::time::Duration::from_millis(50))); }
            return Ok(false);
        }
        let ready = crate::subprocess::poll(&fds, timeout).map_err(|e| e.to_string())?;
        let ready: Vec<RawFd> = fds.iter().zip(ready).filter(|(_, r)| *r != 0).map(|((fd, _), _)| *fd).collect();
        if ready.is_empty() { return Ok(false); }
        if let Some(parent) = &mut self.isolates.parent && ready.contains(&parent.socket.as_raw_fd()) {
            parent.receive(|_, _, _| {}).map_err(|e| e.to_string())?;
        }
        for worker in self.isolates.workers.values_mut() {
            if ready.contains(&worker.peer.socket.as_raw_fd()) {
                let children = &mut worker.children;
                worker.peer.receive(|kind, pid, pidfd| match kind {
                    Kind::Child => { children.insert(pid, pidfd); }
                    _ => { children.remove(&pid); }
                }).map_err(|e| e.to_string())?;
            }
            if worker.status.is_none() && worker.pidfd.as_ref().is_some_and(|fd| ready.contains(&fd.as_raw_fd())) {
                if let Some(status) = crate::subprocess::wait_status(worker.pid, 0) {
                    worker.status = Some(status);
                    worker.peer.inbox.push_back(Frame { kind: Kind::Died, generation: 0, payload: Payload::Status(status) });
                }
            }
        }
        Ok(true)
    }

    /// The next frame from peer ID as `(KIND GENERATION VALUE)`, its value
    /// attached as a frozen region, or nil when none waits. A frame whose
    /// value cannot be attached now stays queued and the error is signalled.
    pub(crate) fn isolate_take(&mut self, id: u64) -> Result<Value> {
        let Some(frame) = self.peer_mut(id)?.inbox.pop_front() else { return Ok(Value::NIL) };
        let value = match &frame.payload {
            Payload::None => Value::NIL,
            Payload::Text(text) => self.make_lisp_string(text.as_bytes().to_vec(), true),
            Payload::Status(status) => Value::integer(i64::from(*status))?,
            Payload::Frozen(backing) => match self.attach(backing.clone()) {
                Ok(root) => root,
                Err(error) => { self.peer_mut(id)?.inbox.push_front(frame); return Err(error); }
            },
        };
        let kind = self.intern(frame.kind.name());
        let generation = Value::integer(frame.generation as i64)?;
        Ok(self.list_value(&[kind, generation, value]))
    }

    pub(crate) fn isolate_signal(&self, id: u64, signal: i32) -> Result<()> {
        let worker = self.isolates.workers.get(&id).ok_or_else(|| format!("no isolate {id}"))?;
        if worker.status.is_some() { return Ok(()); }
        match &worker.pidfd {
            Some(pidfd) => crate::host::pidfd_send_signal(pidfd.as_fd(), signal, false).map_err(|e| e.to_string()),
            None => crate::host::kill(worker.pid, signal).map_err(|e| e.to_string()),
        }
    }

    /// The worker's wait status once it has exited, reaping it.
    pub(crate) fn isolate_reap(&mut self, id: u64) -> Result<Option<i32>> {
        let worker = self.isolates.workers.get_mut(&id).ok_or_else(|| format!("no isolate {id}"))?;
        if worker.status.is_none() {
            worker.status = crate::subprocess::wait_status(worker.pid, 0);
        }
        Ok(worker.status)
    }

    pub(crate) fn isolate_pid(&self, id: u64) -> Option<i32> {
        self.isolates.workers.get(&id).map(|w| w.pid)
    }

    /// SIGHUP to the original process group of every child the worker
    /// reported and did not retire, through its pidfd: what the worker's own
    /// subprocess table would have done had it shut down. The outcomes, one
    /// per child: `(PID . ok)`, `(PID . gone)` (ESRCH: the group is no more),
    /// `(PID . leader-only)` (no PIDFD_SIGNAL_PROCESS_GROUP on this kernel:
    /// the leader alone was signalled), `(PID . ERRNO-TEXT)`.
    pub(crate) fn isolate_signal_children(&mut self, id: u64) -> Result<Value> {
        let worker = self.isolates.workers.get_mut(&id).ok_or_else(|| format!("no isolate {id}"))?;
        let children = std::mem::take(&mut worker.children);
        let mut outcomes = Vec::new();
        for (pid, pidfd) in children {
            let outcome = match pidfd {
                None => "unreachable".to_owned(),
                Some(pidfd) => match crate::host::pidfd_send_signal(pidfd.as_fd(), libc::SIGHUP, true) {
                    Ok(()) => "ok".into(),
                    Err(e) if e.raw_os_error() == Some(libc::ESRCH) => "gone".into(),
                    Err(e) if e.raw_os_error() == Some(libc::EINVAL) => match crate::host::pidfd_send_signal(pidfd.as_fd(), libc::SIGHUP, false) {
                        Ok(()) => "leader-only".into(),
                        Err(e) => e.to_string(),
                    },
                    Err(e) => e.to_string(),
                },
            };
            let pid = Value::integer(i64::from(pid))?;
            let outcome = self.intern(&outcome);
            outcomes.push(self.keymap_cons(pid, outcome));
        }
        Ok(self.list_value(&outcomes))
    }

    /// The children worker ID reported and has not retired.
    pub(crate) fn isolate_children(&self, id: u64) -> Vec<i32> {
        self.isolates.workers.get(&id).map(|w| w.children.keys().copied().collect()).unwrap_or_default()
    }

    /// Forget worker ID: its socket closes; its pidfd is dropped.
    pub(crate) fn isolate_close(&mut self, id: u64) -> bool {
        self.isolates.workers.remove(&id).is_some()
    }

    pub(crate) fn isolate_ids(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self.isolates.workers.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// Frames waiting from peer ID.
    pub(crate) fn isolate_inbox_len(&self, id: u64) -> usize {
        self.peer(id).map_or(0, |p| p.inbox.len())
    }
}

/// A worker's start: SIGTERM once its spawner's thread is gone, the
/// spawner PARENT checked against the pid this process has now (a parent
/// gone before the prctl leaves another), the isolate signals caught, and
/// descriptor FD taken as the control socket.
pub fn worker_socket(fd: i32, parent: Option<i32>) -> std::result::Result<OwnedFd, String> {
    crate::host::set_parent_death_signal(libc::SIGTERM).map_err(|e| e.to_string())?;
    if let Some(parent) = parent && crate::host::parent_pid() != parent {
        return Err(format!("isolate worker: parent {parent} is gone"));
    }
    crate::host::catch_isolate_signals().map_err(|e| e.to_string())?;
    // SAFETY: the launcher put the socket at FD and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

impl Vm {
    /// Serve a spawner over SOCKET as a worker that booted as BOOT says:
    /// the hello, then the Lisp loop until the spawner's bye or a
    /// termination signal.
    pub fn serve_isolate(&mut self, socket: OwnedFd, boot: &str) -> std::result::Result<(), String> {
        self.isolate_adopt_parent(socket);
        let text = hello(boot);
        self.isolates.parent.as_ref().expect("adopted").send(Kind::Hello, 0, &Payload::Text(text)).map_err(|e| e.to_string())?;
        self.eval_source("(romacs--isolate-serve)").map(|_| ())
    }
}
