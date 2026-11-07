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
/// How long a worker has to end after SIGTERM, past `isolate-kill-timeout',
/// the spawner's own escalation while it lives.
const TERMINATION_SECONDS: u32 = 2;

/// Frame kinds. `fork' (spawner to zygote: make a child, whose socket
/// rides beside the frame), `forked' (zygote to spawner: its pid and
/// pidfd) and `reaped' (zygote to spawner: a child's wait status) are the
/// zygote protocol (worker-vms design §2.2); `died' is never sent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum Kind {
    Hello = 1, Job = 2, Result = 3, Error = 4, Message = 5, Child = 6, Retire = 7, Bye = 8, Died = 9, Interrupt = 10,
    Fork = 11, Forked = 12, Reaped = 13,
}

impl Kind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::Hello => "hello", Kind::Job => "job", Kind::Result => "result", Kind::Error => "error", Kind::Message => "message",
            Kind::Child => "child", Kind::Retire => "retire", Kind::Bye => "bye", Kind::Died => "died", Kind::Interrupt => "interrupt",
            Kind::Fork => "fork", Kind::Forked => "forked", Kind::Reaped => "reaped",
        }
    }
    /// The kinds Lisp posts (`romacs--isolate-post').
    const POSTED: [Kind; 9] = [Kind::Hello, Kind::Job, Kind::Result, Kind::Error, Kind::Message, Kind::Child, Kind::Retire, Kind::Bye, Kind::Interrupt];
    const RECEIVED: [Kind; 12] = [Kind::Hello, Kind::Job, Kind::Result, Kind::Error, Kind::Message, Kind::Child, Kind::Retire, Kind::Bye,
                                  Kind::Interrupt, Kind::Fork, Kind::Forked, Kind::Reaped];
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::POSTED.into_iter().find(|k| k.name() == name)
    }
    fn of(byte: u8) -> Option<Self> {
        Self::RECEIVED.into_iter().find(|k| *k as u8 == byte)
    }
}

pub(crate) enum Payload {
    None,
    Frozen(Backing),
    Text(String),
    Status(i32),
    /// A `fork' frame's socket, the child's end.
    Socket(OwnedFd),
}

/// What a frame reports about processes, taken as it is read rather than
/// queued for Lisp: a worker's subprocess spawned (`child') or reaped
/// (`retire'), and a zygote's child forked (`forked', for the spawner's
/// worker ID) or reaped (`reaped').
pub(crate) enum Report {
    Child(i32, Option<OwnedFd>),
    Retire(i32),
    Forked { id: u64, pid: i32, pidfd: Option<OwnedFd> },
    Reaped { pid: i32, status: i32 },
}

/// How a worker ended: its wait status, or unknown (a forked worker,
/// which only its zygote can reap, dead before the zygote's report came).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Exit { Status(i32), Unknown }

/// A frame of KIND and GENERATION whose body is INTS.
fn report_frame(kind: Kind, generation: u64, ints: &[i32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER + 4 * ints.len());
    bytes.push(kind as u8);
    bytes.extend_from_slice(&generation.to_le_bytes());
    bytes.push(0);
    for int in ints { bytes.extend_from_slice(&int.to_le_bytes()); }
    bytes
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

    /// Send a frame; with WAIT, a full peer buffer is waited for (the
    /// worker's replies are mandatory), giving up on termination or a
    /// closed peer.
    fn send(&self, kind: Kind, generation: u64, payload: &Payload, wait: bool) -> std::io::Result<()> {
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
            Payload::Socket(socket) => fd = Some(socket.as_fd()),
        }
        self.send_bytes(&bytes, fd, wait)
    }

    /// Send BYTES, one packet, with FD beside it; WAIT as `send'.
    fn send_bytes(&self, bytes: &[u8], fd: Option<BorrowedFd<'_>>, wait: bool) -> std::io::Result<()> {
        loop {
            match crate::host::send_packet(self.socket.as_fd(), bytes, fd) {
                Err(e) if wait && e.kind() == std::io::ErrorKind::WouldBlock => {
                    if crate::host::terminate_pending() { return Err(e); }
                    let ready = crate::subprocess::poll(&[(self.socket.as_raw_fd(), libc::POLLOUT)], Some(std::time::Duration::from_millis(200)))?;
                    if ready.first().is_some_and(|r| r & (libc::POLLERR | libc::POLLHUP) != 0) { return Err(e); }
                }
                other => return other,
            }
        }
    }

    /// Read every packet the socket holds now (`receive` each).
    fn drain(&mut self, mut report: impl FnMut(Report)) -> std::io::Result<()> {
        while !self.closed {
            match self.receive(&mut report) {
                Ok(Received::Frame) => {}
                Ok(Received::Nothing) | Ok(Received::Closed) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Read one packet into the inbox: the frame, or a process report,
    /// which REPORT takes.
    fn receive(&mut self, report: &mut impl FnMut(Report)) -> std::io::Result<Received> {
        let received = match crate::host::recv_packet(self.socket.as_fd(), PACKET) {
            Ok(received) => received,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(Received::Nothing),
            // A peer that died mid-packet is a closed peer.
            Err(e) if matches!(e.raw_os_error(), Some(libc::ECONNRESET | libc::EPIPE)) => None,
            Err(e) => return Err(e),
        };
        let Some((bytes, mut fds)) = received else {
            self.closed = true;
            return Ok(Received::Closed);
        };
        if bytes.len() < HEADER { return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "short isolate frame")); }
        let kind = Kind::of(bytes[0]).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "unknown isolate frame"))?;
        let generation = u64::from_le_bytes(bytes[1..9].try_into().unwrap());
        let sealed = bytes[9] & FLAG_SEALED != 0;
        let body = &bytes[HEADER..];
        let int = |at: usize| -> std::io::Result<i32> {
            let bytes = body.get(at..at + 4).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "short report frame"))?;
            Ok(i32::from_le_bytes(bytes.try_into().unwrap()))
        };
        let payload = match kind {
            Kind::Child | Kind::Retire | Kind::Forked | Kind::Reaped => {
                report(match kind {
                    Kind::Child => Report::Child(int(0)?, fds.pop()),
                    Kind::Retire => Report::Retire(int(0)?),
                    Kind::Forked => Report::Forked { id: generation, pid: int(0)?, pidfd: fds.pop() },
                    _ => Report::Reaped { pid: int(0)?, status: int(4)? },
                });
                return Ok(Received::Frame);
            }
            Kind::Fork => Payload::Socket(fds.pop().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "fork frame without a socket"))?),
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
        Ok(Received::Frame)
    }
}

enum Received { Frame, Nothing, Closed }

/// Read what worker ID's socket holds: its subprocess reports into its
/// children, a zygote's fork reports into FORKED and REAPED.
fn drain_worker(id: u64, worker: &mut Worker, forked: &mut Vec<(u64, i32, Option<OwnedFd>)>, reaped: &mut Vec<(u64, i32, i32)>) -> std::io::Result<()> {
    let children = &mut worker.children;
    worker.peer.drain(|report| match report {
        Report::Child(pid, pidfd) => { children.insert(pid, pidfd); }
        Report::Retire(pid) => { children.remove(&pid); }
        Report::Forked { id: child, pid, pidfd } => forked.push((child, pid, pidfd)),
        Report::Reaped { pid, status } => reaped.push((id, pid, status)),
    })
}

/// Whether forked WORKER, of zygote ZYGOTE, is gone without a pidfd to
/// say so: never forked and its zygote dead (or forgotten), or forked
/// with no pidfd and its socket closed.
fn unforkable(workers: &HashMap<u64, Worker>, zygote: u64, worker: &Worker) -> bool {
    if worker.pid == 0 {
        workers.get(&zygote).is_none_or(|z| z.status.is_some())
    } else {
        worker.pidfd.is_none() && worker.peer.closed
    }
}

/// A worker this VM spawned, or had a zygote fork.
pub(crate) struct Worker {
    peer: Peer,
    /// 0 for a forked worker until its zygote's `forked' report.
    pid: i32,
    pidfd: Option<OwnedFd>,
    /// The children it reported and has not retired: pid to pidfd.
    children: HashMap<i32, Option<OwnedFd>>,
    status: Option<Exit>,
    /// The zygote that forks it, which alone can reap it.
    zygote: Option<u64>,
    /// The wait status its zygote reported.
    reported: Option<i32>,
}

impl Worker {
    /// It ended as EXIT: published as one Died frame.
    fn die(&mut self, exit: Exit) {
        self.status = Some(exit);
        let payload = match exit { Exit::Status(status) => Payload::Status(status), Exit::Unknown => Payload::None };
        self.peer.inbox.push_back(Frame { kind: Kind::Died, generation: 0, payload });
    }

    /// Its pidfd has turned readable: its exit, reaping it when it is
    /// this VM's child.
    fn exited(&self) -> Option<Exit> {
        match self.zygote {
            Some(_) => Some(self.reported.map_or(Exit::Unknown, Exit::Status)),
            None => crate::subprocess::wait_status(self.pid, 0).map(Exit::Status),
        }
    }
}

/// The pool's memory, measured off the evaluator: reading a process's
/// `smaps_rollup' walks its page tables, milliseconds each, tens with a
/// pool running, which a session must not wait for at a dispatch.
#[derive(Default)]
struct PoolMemory {
    /// The processes to measure, as the evaluator last saw them.
    pids: std::sync::Mutex<Vec<i32>>,
    /// Their proportional memory together, in kilobytes.
    kilobytes: std::sync::atomic::AtomicU64,
}

impl PoolMemory {
    /// Measure every half second, at a worker's priority, until the VM
    /// that owns MEMORY is gone.
    fn sample(memory: std::sync::Weak<PoolMemory>) {
        crate::host::lower_priority(crate::host::Priority::Worker);
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let Some(memory) = memory.upgrade() else { return };
            let pids = memory.pids.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
            let total = pids.iter().filter_map(|&pid| crate::host::process_pss(pid)).sum();
            memory.kilobytes.store(total, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

#[derive(Default)]
pub(crate) struct Isolates {
    workers: HashMap<u64, Worker>,
    /// In a spawner, from its first use (`isolate_pool_memory').
    pool_memory: Option<std::sync::Arc<PoolMemory>>,
    next: u64,
    /// In a worker: the socket to its spawner.
    parent: Option<Peer>,
    /// In a worker: the generation of the job running, while one does.
    running: Option<u64>,
    /// In a zygote: the sockets of children asked for, by generation,
    /// from their `fork' frames until forked.
    fork_sockets: HashMap<u64, OwnedFd>,
    /// In a zygote: its children not yet reaped, pid to pidfd.
    forked: HashMap<i32, OwnedFd>,
    /// In a zygote: children forked and not yet reported, by generation:
    /// pid and the gate's write end, which lets the child run.
    unreleased: HashMap<u64, (i32, OwnedFd)>,
}

impl Isolates {
    pub(crate) fn is_empty(&self) -> bool {
        self.workers.is_empty() && self.parent.is_none() && self.pool_memory.is_none()
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
    // SAFETY: the socket lives in the VM's parent peer for the process's life.
    let socket = unsafe { BorrowedFd::borrow_raw(socket) };
    let _ = crate::host::send_packet(socket, &report_frame(kind, 0, &[pid]), pidfd);
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
        self.isolates.workers.insert(id, Worker { peer: Peer::new(ours), pid, pidfd, children: HashMap::default(), status: None, zygote: None, reported: None });
        Ok(id)
    }

    /// Ask zygote ZYGOTE, a worker that became one, for a child: a socket
    /// pair, the child's end sent in a `fork' frame. The child's id, its
    /// pid and pidfd known once the zygote reports it forked.
    pub(crate) fn isolate_fork_request(&mut self, zygote: u64) -> Result<u64> {
        let (ours, theirs) = crate::host::socketpair().map_err(|e| e.to_string())?;
        let id = self.isolates.next + 1;
        let peer = &self.isolates.workers.get(&zygote).ok_or_else(|| format!("no isolate {zygote}"))?.peer;
        peer.send(Kind::Fork, id, &Payload::Socket(theirs), false).map_err(|e| format!("Cannot ask zygote {zygote} to fork: {e}"))?;
        self.isolates.next = id;
        let worker = Worker { peer: Peer::new(ours), pid: 0, pidfd: None, children: HashMap::default(), status: None, zygote: Some(zygote), reported: None };
        self.isolates.workers.insert(id, worker);
        Ok(id)
    }

    /// In a zygote: fork the child the `fork' frame of GENERATION asked
    /// for. In the zygote, the child's pid; in the child, None, once it is
    /// a worker of the spawner's own (`fork_child_reinit'). The child runs
    /// nothing until `isolate_release' has reported it to the spawner: a
    /// zygote that dies first ends it unrun, so a job the spawner never
    /// heard was forked never ran. Refused while a thread other than the
    /// caller and the parked main thread runs: a fork keeps only the caller.
    pub(crate) fn isolate_fork(&mut self, generation: u64) -> Result<Option<i32>> {
        let socket = self.isolates.fork_sockets.remove(&generation).ok_or_else(|| format!("no fork asked as {generation}"))?;
        if let Some(refusal) = self.isolate_fork_refusal() { return Err(refusal); }
        let (gate, release) = crate::host::pipe().map_err(|e| format!("fork gate: {e}"))?;
        let zygote = crate::host::pid() as i32;
        let pid = crate::host::fork_isolate().map_err(|e| format!("fork: {e}"))?;
        if pid == 0 {
            drop(release);
            self.fork_child_reinit(socket, zygote, gate)?;
            return Ok(None);
        }
        drop((socket, gate));
        self.isolates.unreleased.insert(generation, (pid, release));
        Ok(Some(pid))
    }

    /// In a zygote: report the child forked for GENERATION to the
    /// spawner, with its pidfd, then let it run.
    pub(crate) fn isolate_release(&mut self, generation: u64) -> Result<()> {
        let (pid, release) = self.isolates.unreleased.remove(&generation).ok_or_else(|| format!("no child forked as {generation}"))?;
        let pidfd = crate::host::pidfd_open(pid);
        let parent = self.isolates.parent.as_ref().ok_or("a zygote serves a spawner")?;
        parent.send_bytes(&report_frame(Kind::Forked, generation, &[pid]), pidfd.as_ref().map(|fd| fd.as_fd()), true)
            .map_err(|e| format!("Cannot report a fork: {e}"))?;
        if let Some(pidfd) = pidfd { self.isolates.forked.insert(pid, pidfd); }
        crate::host::open_gate(release).map_err(|e| format!("Cannot release a child: {e}"))
    }

    /// The proportional memory of worker ID, or of every live worker, in
    /// kilobytes: a page shared between them counts once across the sum.
    pub(crate) fn isolate_pss(&self, id: Option<u64>) -> Result<u64> {
        let live = |worker: &Worker| if worker.status.is_none() && worker.pid != 0 { crate::host::process_pss(worker.pid) } else { None };
        Ok(match id {
            Some(id) => live(self.isolates.workers.get(&id).ok_or_else(|| format!("no isolate {id}"))?).unwrap_or(0),
            None => self.isolates.workers.values().filter_map(live).sum(),
        })
    }

    /// The pool's proportional memory in kilobytes, as last measured off
    /// the evaluator; the first use measures here, the pool being new.
    pub(crate) fn isolate_pool_memory(&mut self) -> u64 {
        let pids: Vec<i32> = self.isolates.workers.values().filter(|w| w.status.is_none() && w.pid != 0).map(|w| w.pid).collect();
        if let Some(memory) = &self.isolates.pool_memory {
            *memory.pids.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = pids;
            return memory.kilobytes.load(std::sync::atomic::Ordering::Relaxed);
        }
        let total: u64 = pids.iter().filter_map(|&pid| crate::host::process_pss(pid)).sum();
        let memory = std::sync::Arc::new(PoolMemory { pids: std::sync::Mutex::new(pids), kilobytes: total.into() });
        let sampler = std::sync::Arc::downgrade(&memory);
        // Without the thread the figure stays as first measured.
        let _ = std::thread::Builder::new().name("romacs-pool-memory".into()).stack_size(64 * 1024).spawn(move || PoolMemory::sample(sampler));
        self.isolates.pool_memory = Some(memory);
        total
    }

    /// Whether a job runs here now, inside its handler.
    pub(crate) fn isolate_job_running(&self) -> bool {
        self.isolates.running.is_some()
    }

    /// In a child: BYTES of growth over the zygote's heap pace its
    /// collections (`set_job_budget`).
    pub(crate) fn isolate_job_budget(&mut self, bytes: usize) {
        self.set_job_budget(bytes);
    }

    /// In a zygote: its children not yet reaped.
    pub(crate) fn isolate_forked(&self) -> usize {
        self.isolates.forked.len()
    }

    /// Why this worker cannot fork now, or None.
    pub(crate) fn isolate_fork_refusal(&self) -> Option<String> {
        match crate::host::other_threads() {
            Ok(others) if others.is_empty() => None,
            Ok(others) => Some(format!("Threads {others:?} run beside the zygote's evaluator")),
            Err(e) => Some(format!("Cannot list threads: {e}")),
        }
    }

    /// In a child just forked from ZYGOTE, over SOCKET to the spawner: what
    /// is per process, reset (as `world_reinit' does for a restored
    /// process), then its hello. The heap, symbols and features are the
    /// zygote's, which is the point; a job brings its directory and
    /// bindings.
    /// It waits at GATE for the zygote's release, and exits unrun if the
    /// zygote ends first.
    fn fork_child_reinit(&mut self, socket: OwnedFd, zygote: i32, gate: OwnedFd) -> Result<()> {
        crate::host::forked_child_reset(zygote).map_err(|e| e.to_string())?;
        if !crate::host::await_gate(gate) { std::process::exit(0); }
        // The zygote's children, requests and socket are not this child's.
        self.isolates.forked.clear();
        self.isolates.unreleased.clear();
        self.isolates.fork_sockets.clear();
        self.isolates.running = None;
        self.isolates.parent = None;
        self.isolate_adopt_parent(socket);
        self.random = crate::random::Random::from_entropy();
        self.idle_since = None;
        self.gc_stress.rearm();
        self.reset_collection_counters();
        // A finalizer runs once: in the zygote, not in each child.
        self.finalizers.clear();
        self.doomed_finalizers.clear();
        self.post_gc_due = false;
        let hello = Payload::Text(hello("fork"));
        self.isolates.parent.as_ref().expect("adopted").send(Kind::Hello, 0, &hello, true).map_err(|e| e.to_string())
    }

    /// Make the next collection compact the heap's tables, and after it
    /// reserve room in each for NEEDED more entries: a zygote's children
    /// then push into fresh pages instead of reusing free entries spread
    /// over shared ones, or copying a whole table to grow it.
    pub(crate) fn isolate_compact_next(&mut self) {
        self.free.compact = true;
    }

    pub(crate) fn isolate_reserve(&mut self, needed: usize) {
        self.conses.reserve(needed);
        self.objects.reserve(needed / 4);
        self.closures.reserve(needed / 16);
        self.cells.reserve(needed / 16);
        self.slots.reserve(needed / 16);
        // The baseline of its children's budgets.
        self.measure_retained_payload();
    }

    /// In a worker: descriptor FD is the socket to the spawner (peer 0).
    pub(crate) fn isolate_adopt_parent(&mut self, socket: OwnedFd) {
        let _ = crate::host::set_nonblocking(socket.as_fd());
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
        let _ = parent.drain(|_| {});
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
        // A worker's frames to its spawner are mandatory: they wait.
        peer.send(kind, generation, &payload, id == 0).map_err(|e| match e.kind() {
            std::io::ErrorKind::WouldBlock => crate::frozen::FreezeError::QueueFull,
            _ if matches!(e.raw_os_error(), Some(libc::EPIPE | libc::ECONNRESET | libc::ENOTCONN)) => crate::frozen::FreezeError::PeerGone,
            _ => crate::frozen::FreezeError::Failed(format!("isolate send: {e}")),
        })
    }

    /// The descriptors a wait on the isolates needs: every peer's socket
    /// and every live worker's pidfd, readable when something happened.
    pub(crate) fn isolate_fds(&self) -> Vec<(RawFd, i16)> {
        let mut fds = Vec::new();
        if let Some(parent) = &self.isolates.parent && !parent.closed {
            fds.push((parent.socket.as_raw_fd(), libc::POLLIN));
        }
        if self.isolates.parent.is_some() && let Some(wake) = crate::host::isolate_wake_fd() {
            fds.push((wake, libc::POLLIN));
        }
        fds.extend(self.isolates.forked.values().map(|pidfd| (pidfd.as_raw_fd(), libc::POLLIN)));
        for worker in self.isolates.workers.values() {
            if !worker.peer.closed { fds.push((worker.peer.socket.as_raw_fd(), libc::POLLIN)); }
            if worker.status.is_none() && let Some(pidfd) = &worker.pidfd { fds.push((pidfd.as_raw_fd(), libc::POLLIN)); }
        }
        fds
    }

    /// Wait up to TIMEOUT (None: as long as it takes) for a frame or a
    /// worker's death, reading what is ready: whether anything was. In a
    /// worker, a pending interrupt or termination ends the wait at once.
    /// A worker's socket closing with nothing in it is no news: the worker
    /// is exiting, and exit closes its descriptors before its pidfd turns
    /// readable, so the wait goes on for the death that follows.
    pub(crate) fn isolate_wait(&mut self, timeout: Option<std::time::Duration>) -> Result<bool> {
        // Interrupted or terminating: no wait, but what the sockets hold is
        // still read (the frame naming the interrupt's job is there).
        let interrupted = crate::host::interrupt_pending() || crate::host::terminate_pending();
        let timeout = if interrupted { Some(std::time::Duration::ZERO) } else { timeout };
        let deadline = timeout.map(|timeout| crate::host::monotonic() + timeout);
        loop {
            let timeout = deadline.map(|deadline| deadline.saturating_duration_since(crate::host::monotonic()));
            let fds = self.isolate_fds();
            if fds.is_empty() {
                if let Some(timeout) = timeout { std::thread::sleep(timeout.min(std::time::Duration::from_millis(50))); }
                return Ok(interrupted);
            }
            let ready = crate::subprocess::poll(&fds, timeout).map_err(|e| e.to_string())?;
            let ready: Vec<RawFd> = fds.iter().zip(ready).filter(|(_, r)| *r != 0).map(|((fd, _), _)| *fd).collect();
            let mut news = false;
            if let Some(wake) = crate::host::isolate_wake_fd() && ready.contains(&wake) {
                crate::host::drain_isolate_wake();
                news = true;
            }
            let interrupted = interrupted || crate::host::interrupt_pending();
            if ready.is_empty() { return Ok(interrupted); }
            if let Some(parent) = &mut self.isolates.parent && ready.contains(&parent.socket.as_raw_fd()) {
                parent.drain(|_| {}).map_err(|e| e.to_string())?;
                news = true;
            }
            news |= self.reap_forked(&ready);
            // Every report is read and applied before any death is settled:
            // an exited worker's last frames first, and a zygote's `forked'
            // or `reaped' report before its children's deaths.
            let (mut forked, mut reaped, mut exited) = (Vec::new(), Vec::new(), Vec::new());
            for (&id, worker) in self.isolates.workers.iter_mut() {
                let gone = worker.status.is_none() && worker.pidfd.as_ref().is_some_and(|fd| ready.contains(&fd.as_raw_fd()));
                if gone || ready.contains(&worker.peer.socket.as_raw_fd()) {
                    let before = worker.peer.inbox.len();
                    drain_worker(id, worker, &mut forked, &mut reaped).map_err(|e| e.to_string())?;
                    news |= !worker.peer.closed || worker.peer.inbox.len() != before;
                }
                if gone { exited.push(id); }
            }
            self.apply_fork_reports(forked, reaped);
            for id in exited {
                news = true;
                let worker = self.isolates.workers.get_mut(&id).expect("listed");
                if let Some(exit) = worker.exited() { worker.die(exit); }
            }
            news |= self.settle_unforked();
            if news || interrupted { return Ok(true); }
        }
    }

    /// In a zygote: reap each child whose pidfd is in READY and report its
    /// status to the spawner, which cannot reap a process it did not fork.
    /// Whether any was reaped.
    fn reap_forked(&mut self, ready: &[RawFd]) -> bool {
        let exited: Vec<i32> = self.isolates.forked.iter().filter(|(_, fd)| ready.contains(&fd.as_raw_fd())).map(|(&pid, _)| pid).collect();
        let mut reaped = false;
        for pid in exited {
            let Some(status) = crate::subprocess::wait_status(pid, 0) else { continue };
            self.isolates.forked.remove(&pid);
            reaped = true;
            if let Some(parent) = &self.isolates.parent {
                let _ = parent.send_bytes(&report_frame(Kind::Reaped, 0, &[pid, status]), None, true);
            }
        }
        reaped
    }

    /// Apply the zygotes' reports read: a forked worker learns its pid and
    /// pidfd; a reaped one its status, should its death not be settled yet.
    fn apply_fork_reports(&mut self, forked: Vec<(u64, i32, Option<OwnedFd>)>, reaped: Vec<(u64, i32, i32)>) {
        let workers = &mut self.isolates.workers;
        for (id, pid, pidfd) in forked {
            if let Some(worker) = workers.get_mut(&id) && worker.pid == 0 {
                (worker.pid, worker.pidfd) = (pid, pidfd);
            }
        }
        for (zygote, pid, status) in reaped {
            if let Some(worker) = workers.values_mut().find(|w| w.zygote == Some(zygote) && w.pid == pid) {
                worker.reported = Some(status);
            }
        }
    }

    /// A worker never forked dies (status unknown) once its zygote has,
    /// whose socket, read before its death was settled, held no `forked'
    /// report for it: the zygote released no such child, so it ran
    /// nothing. Whether any died.
    fn settle_unforked(&mut self) -> bool {
        let workers = &mut self.isolates.workers;
        let orphans: Vec<u64> = workers.iter()
            .filter(|(_, w)| w.status.is_none() && w.zygote.is_some_and(|z| unforkable(workers, z, w)))
            .map(|(&id, _)| id).collect();
        for id in &orphans { workers.get_mut(id).expect("listed").die(Exit::Unknown); }
        !orphans.is_empty()
    }

    /// The next frame from peer ID as `(KIND GENERATION VALUE)`, its value
    /// attached as a frozen region, or nil when none waits. A frame whose
    /// value cannot be attached now stays queued and the error is signalled.
    pub(crate) fn isolate_take(&mut self, id: u64) -> Result<Value> {
        let Some(mut frame) = self.peer_mut(id)?.inbox.pop_front() else { return Ok(Value::NIL) };
        // A fork's socket waits here for `isolate_fork'.
        if let Payload::Socket(_) = frame.payload
            && let Payload::Socket(socket) = std::mem::replace(&mut frame.payload, Payload::None) {
            self.isolates.fork_sockets.insert(frame.generation, socket);
        }
        let value = match &frame.payload {
            Payload::None | Payload::Socket(_) => Value::NIL,
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
        // A worker not forked yet has nothing to signal; its zygote's end
        // ends it (`unforkable').
        if worker.status.is_some() || worker.pid == 0 { return Ok(()); }
        let sent = match &worker.pidfd {
            Some(pidfd) => crate::host::pidfd_send_signal(pidfd.as_fd(), signal, false),
            None => crate::host::kill(worker.pid, signal),
        };
        match sent {
            // Gone already: a forked worker its zygote has reaped.
            Err(e) if e.raw_os_error() == Some(libc::ESRCH) && worker.zygote.is_some() => Ok(()),
            other => other.map_err(|e| e.to_string()),
        }
    }

    /// The worker's exit once it has exited, reaping it. A death seen here
    /// is published as the wait's is: one Died frame, whichever path sees
    /// it first, so a waiter after the reap still learns of it. A forked
    /// worker's is seen at its pidfd, or at its zygote's end.
    pub(crate) fn isolate_reap(&mut self, id: u64) -> Result<Option<Exit>> {
        let worker = self.isolates.workers.get(&id).ok_or_else(|| format!("no isolate {id}"))?;
        let exit = match (worker.status, worker.zygote) {
            (Some(_), _) => None,
            (None, None) => worker.exited(),
            (None, Some(zygote)) => match &worker.pidfd {
                Some(pidfd) => {
                    let ready = crate::subprocess::poll(&[(pidfd.as_raw_fd(), libc::POLLIN)], Some(std::time::Duration::ZERO)).map_err(|e| e.to_string())?;
                    if ready.first().is_some_and(|r| *r != 0) { worker.exited() } else { None }
                }
                None => unforkable(&self.isolates.workers, zygote, worker).then_some(Exit::Unknown),
            },
        };
        if let Some(exit) = exit {
            // Its last frames and reports first, as the wait reads them.
            let (mut forked, mut reaped) = (Vec::new(), Vec::new());
            let _ = drain_worker(id, self.isolates.workers.get_mut(&id).expect("found"), &mut forked, &mut reaped);
            self.apply_fork_reports(forked, reaped);
            self.isolates.workers.get_mut(&id).expect("found").die(exit);
        }
        Ok(self.isolates.workers.get(&id).expect("found").status)
    }

    pub(crate) fn isolate_pid(&self, id: u64) -> Option<i32> {
        self.isolates.workers.get(&id).map(|w| w.pid).filter(|&pid| pid != 0)
    }

    /// SIGHUP to the original process group of every child the worker
    /// reported and did not retire, through its pidfd: what the worker's own
    /// subprocess table would have done had it shut down. The outcomes, one
    /// per child: `(PID . ok)`, `(PID . gone)` (ESRCH: the group is no more),
    /// `(PID . leader-only)` (no PIDFD_SIGNAL_PROCESS_GROUP on this kernel:
    /// the leader alone was signalled), `(PID . ERRNO-TEXT)`.
    pub(crate) fn isolate_signal_children(&mut self, id: u64) -> Result<Value> {
        let worker = self.isolates.workers.get_mut(&id).ok_or_else(|| format!("no isolate {id}"))?;
        // Reports still in the socket, from a worker that died or was killed.
        let (mut forked, mut reaped) = (Vec::new(), Vec::new());
        let _ = drain_worker(id, worker, &mut forked, &mut reaped);
        let children = std::mem::take(&mut worker.children);
        self.apply_fork_reports(forked, reaped);
        self.settle_unforked();
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
    crate::host::limit_termination(TERMINATION_SECONDS);
    // SAFETY: the launcher put the socket at FD and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

impl Vm {
    /// Serve a spawner over SOCKET as a worker that booted as BOOT says:
    /// the hello, then the Lisp loop until the spawner's bye or a
    /// termination signal.
    /// A child forked from it serves the same loop, from the zygote's
    /// fork onward, and exits here: no main thread is left to return to.
    pub fn serve_isolate(&mut self, socket: OwnedFd, boot: &str) -> std::result::Result<(), String> {
        self.isolate_adopt_parent(socket);
        let text = hello(boot);
        self.isolates.parent.as_ref().expect("adopted").send(Kind::Hello, 0, &Payload::Text(text), true).map_err(|e| e.to_string())?;
        let served = self.eval_source("(romacs--isolate-serve)").map(|_| ());
        if crate::host::forked_child() {
            let shutdown = self.shutdown_background_processes();
            if let Err(error) = served.as_ref().and(shutdown.as_ref()) { eprintln!("isolate worker: {error}"); }
            std::process::exit(i32::from(served.is_err() || shutdown.is_err()));
        }
        served
    }
}
