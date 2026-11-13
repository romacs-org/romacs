//! The keeper's one loop: sessions' requests, children's changes, timers.
//!
//! Single-threaded, and never blocked on a session: sockets are
//! nonblocking, replies queue, and a session that does not read only
//! stalls itself. The keeper never reads or writes a kept descriptor.
//!
//! Its system calls go through rustix, but for two that no crate in the
//! lock file covers safely: the SIGCHLD signalfd with its blocked mask
//! ([`child_signals`]), and the peer's pidfd by `SO_PEERPIDFD` ([`peer`]),
//! an option rustix does not have.

use std::collections::VecDeque;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};

use romacs_record::{channel, hello, Decoder, Handle, Hello, Reader, Record, Writer};
use rustix::event::{PollFd, PollFlags, Timespec};
use rustix::io::FdFlags;
use rustix::process::{PidfdFlags, Resource, Signal};

use crate::proto::{self, count, kind};
use crate::spawn::{self, Spec, Status, Whom};
use crate::table::{Attachment, Child, Entry, Table, TABLE_VERSION};

/// How long a closed handle's child has after its signal, and a superseded
/// session after its kill, in milliseconds.
const GRACE: i32 = 2000;
const GRACE_MS: u64 = 2000;
pub const DEFAULT_IDLE_MS: u64 = 60_000;

/// What differs between keeper builds, so tests can stand in for the next.
#[derive(Clone, Copy)]
pub struct Build {
    /// The newest table this build reads; it writes that version.
    pub table: u16,
}

impl Default for Build {
    fn default() -> Self { Self { table: TABLE_VERSION } }
}

struct Conn {
    socket: UnixStream,
    decoder: Decoder,
    fds: VecDeque<OwnedFd>,
    out: VecDeque<(Vec<u8>, Vec<OwnedFd>)>,
    sent: usize,
    seq: u64,
    attachment: u64,
    greeted: bool,
    dead: bool,
}

pub struct Keeper {
    pub table: Table,
    listener: UnixListener,
    conns: Vec<Conn>,
    signals: OwnedFd,
    build: Build,
    exe_hash: u64,
    idle_since: u64,
    stop: bool,
}

type Reply = Result<(Writer, Vec<OwnedFd>), String>;

/// Milliseconds since the epoch: the clock of deadlines.
#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn os(result: libc::c_int) -> io::Result<libc::c_int> {
    if result < 0 { Err(io::Error::last_os_error()) } else { Ok(result) }
}

/// A descriptor that reads when SIGCHLD arrives; the signal stays blocked.
/// Direct calls: rustix has no signalfd, and changes the signal mask and a
/// disposition only through unsafe runtime functions.
fn child_signals() -> io::Result<OwnedFd> {
    // SAFETY: the set is initialised by sigemptyset before use; the calls
    // take it by pointer for their duration. signalfd returns a new
    // descriptor owned here.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&raw mut set);
        libc::sigaddset(&raw mut set, libc::SIGCHLD);
        libc::sigprocmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut());
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        Ok(OwnedFd::from_raw_fd(os(libc::signalfd(-1, &raw const set, libc::SFD_CLOEXEC | libc::SFD_NONBLOCK))?))
    }
}

/// The process at the other end of SOCKET: its pid and a pidfd for it.
fn peer(socket: &UnixStream) -> io::Result<(i32, OwnedFd)> {
    let pid = rustix::net::sockopt::socket_peercred(socket)?.pid;
    // The pidfd the kernel made at connect, or else one opened on the pid
    // it recorded then.
    match peer_pidfd(socket) {
        Some(pidfd) => Ok((pid.as_raw_nonzero().get(), pidfd)),
        None => Ok((pid.as_raw_nonzero().get(), rustix::process::pidfd_open(pid, PidfdFlags::empty())?)),
    }
}

/// `SO_PEERPIDFD`, which rustix does not have: `None` on a kernel without it.
fn peer_pidfd(socket: &UnixStream) -> Option<OwnedFd> {
    let mut pidfd: RawFd = -1;
    let mut len: libc::socklen_t = 4;
    // SAFETY: getsockopt writes at most LEN bytes into PIDFD, an int of
    // that size; on success it is a new descriptor owned here alone.
    unsafe {
        let got = libc::getsockopt(socket.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERPIDFD, (&raw mut pidfd).cast(), &raw mut len);
        (got == 0 && pidfd >= 0).then(|| OwnedFd::from_raw_fd(pidfd))
    }
}

fn timeout(ms: i32) -> Timespec { Timespec { tv_sec: i64::from(ms / 1000), tv_nsec: i64::from(ms % 1000) * 1_000_000 } }

fn readable(fd: BorrowedFd<'_>, timeout_ms: i32) -> bool {
    rustix::event::poll(&mut [PollFd::new(&fd, PollFlags::IN)], Some(&timeout(timeout_ms))).is_ok_and(|ready| ready > 0)
}

fn set_cloexec(fd: BorrowedFd<'_>, on: bool) {
    let _ = rustix::io::fcntl_setfd(fd, if on { FdFlags::CLOEXEC } else { FdFlags::empty() });
}

/// The identity of an executable whose BYTES the caller read: a hash of them.
#[must_use]
pub fn bytes_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// The identity of the executable at PATH, read here: 0 if it cannot be read.
#[must_use]
pub fn executable_hash(path: &std::path::Path) -> u64 {
    std::fs::read(path).map_or(0, |bytes| bytes_hash(&bytes))
}

impl Keeper {
    /// A keeper over TABLE, which names its listening socket.
    ///
    /// # Errors
    /// A table without a listener, or the system's refusal of a signalfd.
    pub fn new(mut table: Table, build: Build) -> io::Result<Self> {
        let listener = UnixListener::from(table.listener.take().ok_or(io::ErrorKind::InvalidInput)?);
        listener.set_nonblocking(true)?;
        // Orphaned descendants are reparented to this process, which reaps them.
        let _ = rustix::process::set_child_subreaper(Some(rustix::process::getpid()));
        let mut limit = rustix::process::getrlimit(Resource::Nofile);
        limit.current = limit.maximum;
        let _ = rustix::process::setrlimit(Resource::Nofile, limit);
        if table.idle_ms == 0 { table.idle_ms = DEFAULT_IDLE_MS; }
        let mut keeper = Self { table, listener, conns: Vec::new(), signals: child_signals()?, build,
                                exe_hash: executable_hash(std::path::Path::new("/proc/self/exe")), idle_since: now_ms(), stop: false };
        // Changes that arrived while no keeper was looking (across an exec).
        keeper.reap();
        Ok(keeper)
    }

    /// Serve until asked to stop or idle for long enough.
    pub fn run(&mut self) {
        while !self.stop {
            let now = now_ms();
            let busy = !self.table.handles.is_empty() || !self.conns.is_empty();
            if busy { self.idle_since = now; }
            if now.saturating_sub(self.idle_since) >= self.table.idle_ms { break; }
            let timer = self.table.timers.first().map_or(u64::MAX, |(at, _)| at.saturating_sub(now));
            let idle = if busy { u64::MAX } else { self.idle_since + self.table.idle_ms - now };
            self.wait(i32::try_from(timer.min(idle)).unwrap_or(60_000));
            self.timers();
        }
    }

    fn wait(&mut self, timeout_ms: i32) {
        let sessions: Vec<(u64, BorrowedFd<'_>)> = self.table.attachments.iter()
            .filter_map(|(id, a)| a.pidfd.as_ref().map(|pidfd| (*id, pidfd.as_fd()))).collect();
        let mut polls = vec![PollFd::new(&self.listener, PollFlags::IN), PollFd::new(&self.signals, PollFlags::IN)];
        polls.extend(self.conns.iter().map(|c| PollFd::new(&c.socket, if c.out.is_empty() { PollFlags::IN } else { PollFlags::IN | PollFlags::OUT })));
        polls.extend(sessions.iter().map(|(_, pidfd)| PollFd::new(pidfd, PollFlags::IN)));
        if !rustix::event::poll(&mut polls, Some(&timeout(timeout_ms))).is_ok_and(|ready| ready > 0) { return; }
        let revents: Vec<PollFlags> = polls.iter().map(PollFd::revents).collect();
        let sessions: Vec<u64> = sessions.into_iter().map(|(id, _)| id).collect();
        let ready = |index: usize| !revents[index].is_empty();
        let count = self.conns.len();
        for (index, id) in sessions.iter().enumerate() {
            if ready(2 + count + index) { self.session_ended(*id); }
        }
        for index in 0..count {
            if revents[2 + index].contains(PollFlags::OUT) { self.flush(index); }
            if !revents[2 + index].difference(PollFlags::OUT).is_empty() && !self.conns[index].dead { self.read(index); }
        }
        if ready(1) {
            let mut buffer = [0u8; 512];
            while rustix::io::read(&self.signals, &mut buffer).is_ok_and(|read| read > 0) {}
            self.reap();
        }
        if ready(0) { self.accept(); }
        self.conns.retain(|c| !c.dead);
    }

    fn accept(&mut self) {
        while let Ok((socket, _)) = self.listener.accept() {
            let Ok((pid, pidfd)) = peer(&socket) else { continue };
            if socket.set_nonblocking(true).is_err() { continue; }
            let known = self.table.attachments.iter().find(|(_, a)| a.pid == pid && a.pidfd.is_some()).map(|(id, _)| *id);
            let attachment = known.unwrap_or_else(|| {
                self.table.last_attachment += 1;
                self.table.attachments.insert(self.table.last_attachment, Attachment { pid, pidfd: Some(pidfd), owned: 0 });
                self.table.last_attachment
            });
            // A session has one control connection: its newest. An older
            // one may still look open (a forked child holds the socket for
            // a moment, or for good) and must not be sent what is meant
            // for the session.
            for conn in &mut self.conns { conn.dead |= conn.attachment == attachment; }
            let mut conn = Conn { socket, decoder: Decoder::default(), fds: VecDeque::new(), out: VecDeque::new(), sent: 0, seq: 0,
                                  attachment, greeted: false, dead: false };
            let mut wire = Vec::new();
            Hello { epoch: self.table.epoch, pid: std::process::id(), classes: vec![proto::class()] }.record().encode_into(&mut wire);
            conn.out.push_back((wire, Vec::new()));
            self.conns.push(conn);
            self.flush(self.conns.len() - 1);
        }
    }

    /// The session process of ATTACHMENT exited: that is its detach. Its
    /// handles stay, owned by nobody alive, for a successor to claim.
    fn session_ended(&mut self, id: u64) {
        let Some(attachment) = self.table.attachments.get_mut(&id) else { return };
        attachment.pidfd = None;
        if attachment.owned == 0 { self.table.attachments.remove(&id); }
        // A process that inherited the socket does not keep the session attached.
        for conn in &mut self.conns { conn.dead |= conn.attachment == id; }
        // A deadline that passed while this session still lived takes
        // effect now that nobody owns the handle.
        let now = now_ms();
        for handle in self.owned_where(|e| e.owner == id && e.deadline != 0 && e.deadline <= now) { self.close(handle, libc::SIGHUP); }
    }

    fn send(&mut self, index: usize, kind: u16, payload: &[u8], fds: Vec<OwnedFd>) {
        let conn = &mut self.conns[index];
        conn.seq += 1;
        let mut wire = Vec::new();
        romacs_record::record::encode(conn.seq, proto::CLASS, kind, payload, &mut wire);
        conn.out.push_back((wire, fds));
        self.flush(index);
    }

    fn flush(&mut self, index: usize) {
        let conn = &mut self.conns[index];
        while let Some((bytes, fds)) = conn.out.front() {
            let borrowed: Vec<_> = if conn.sent == 0 { fds.iter().map(AsFd::as_fd).collect() } else { Vec::new() };
            match channel::send(&conn.socket, &bytes[conn.sent..], &borrowed) {
                Ok(sent) if conn.sent + sent == bytes.len() => { conn.out.pop_front(); conn.sent = 0; }
                Ok(sent) => conn.sent += sent,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return,
                Err(_) => { conn.dead = true; return; }
            }
        }
    }

    fn read(&mut self, index: usize) {
        let mut buffer = [0u8; 16384];
        loop {
            let conn = &mut self.conns[index];
            let mut fds = Vec::new();
            match channel::receive(&conn.socket, &mut buffer, &mut fds) {
                Ok(0) => { conn.dead = true; break; }
                Ok(count) => { conn.decoder.push(&buffer[..count]); conn.fds.extend(fds); }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => { conn.dead = true; break; }
            }
        }
        while !self.conns[index].dead && !self.stop {
            match self.conns[index].decoder.pop() {
                Ok(Some(record)) => self.record(index, &record),
                Ok(None) => break,
                Err(_) => self.conns[index].dead = true,
            }
        }
    }

    fn record(&mut self, index: usize, record: &Record) {
        if !self.conns[index].greeted {
            // A peer of another framing is dropped: it reads the reason
            // from this side's hello, which it has already been sent.
            let agreed = Hello::read(record).is_ok_and(|theirs| hello::agree(&proto::class(), &theirs.classes).version().is_ok());
            self.conns[index].greeted = agreed;
            self.conns[index].dead = !agreed;
            // What changed while this session had no connection.
            let attachment = self.conns[index].attachment;
            let pending: Vec<Handle> = self.table.handles.iter()
                .filter(|(_, e)| e.owner == attachment && e.child.as_ref().is_some_and(|c| c.unreported)).map(|(h, _)| *h).collect();
            for handle in pending { self.report(handle); }
            return;
        }
        if (record.class, record.kind) == (proto::CLASS, kind::EXEC) {
            return self.exec_request(index, record);
        }
        let reply = if record.class == proto::CLASS {
            self.request(index, record.kind, &mut Reader(&record.payload))
        } else {
            Err(format!("this keeper has no class {}", record.class))
        };
        self.conns[index].fds.clear();
        self.reply(index, record.seq, reply);
    }

    fn reply(&mut self, index: usize, seq: u64, reply: Reply) {
        let mut payload = Writer::default().u64(seq).u8(u8::from(reply.is_err()));
        let fds = match reply {
            Ok((body, fds)) => { payload.0.extend(body.finish()); fds }
            Err(message) => { payload = payload.str(&message); Vec::new() }
        };
        self.send(index, kind::REPLY, &payload.finish(), fds);
    }

    /// Replace this process by the keeper the request names. The new
    /// binary first checks a copy of the table; a refusal is the reply and
    /// nothing changes. Otherwise the session is told yes, every session
    /// is told to reconnect, and the exec follows.
    fn exec_request(&mut self, index: usize, record: &Record) {
        let path = Reader(&record.payload).bytes().map(|b| std::path::PathBuf::from(String::from_utf8_lossy(b).into_owned()));
        let checked = path.map_err(|e| e.to_string()).and_then(|path| crate::exec::check(&self.table, self.build, &path).map(|()| path));
        match checked {
            Err(message) => self.reply(index, record.seq, Err(message)),
            Ok(path) => {
                self.reply(index, record.seq, Ok((Writer::default(), Vec::new())));
                self.exec(&path);
            }
        }
    }

    fn handle(&self, r: &mut Reader<'_>) -> Result<Handle, String> {
        let raw = r.u64().map_err(|e| e.to_string())?;
        Handle::from_raw(raw).filter(|h| self.table.handles.get(h).is_some_and(|e| !e.closing)).ok_or_else(|| format!("no handle {raw}"))
    }

    /// The row of HANDLE, and duplicates of its open descriptors.
    fn handed(&self, handle: Handle) -> Reply {
        let entry = &self.table.handles[&handle];
        let fds = entry.slots.iter().flatten().map(OwnedFd::try_clone).collect::<io::Result<Vec<_>>>().map_err(|e| e.to_string())?;
        if fds.len() > channel::MAX_FDS { return Err(format!("handle {handle} has more descriptors than one reply carries")); }
        Ok((self.table.row(handle).expect("present").write(Writer::default()), fds))
    }

    fn insert(&mut self, attachment: u64, r: &mut Reader<'_>, slots: Vec<OwnedFd>, child: Option<Child>) -> Result<Handle, String> {
        let bad = |e: romacs_record::Error| e.to_string();
        let handle = self.table.allocator.allocate().ok_or("handles exhausted")?;
        let entry = Entry { tag: r.str().map_err(bad)?.into(), version: r.u16().map_err(bad)?, metadata: r.bytes().map_err(bad)?.to_vec(),
                            kept: r.u8().map_err(bad)? != 0, deadline: 0, slots: slots.into_iter().map(Some).collect(), child, owner: 0, closing: false };
        if let Some(child) = &entry.child { self.table.pids.insert(child.pid, handle); }
        self.table.handles.insert(handle, entry);
        self.table.set_owner(handle, attachment);
        Ok(handle)
    }

    fn request(&mut self, index: usize, kind: u16, r: &mut Reader<'_>) -> Reply {
        let bad = |e: romacs_record::Error| e.to_string();
        let me = self.conns[index].attachment;
        let done = || Ok((Writer::default(), Vec::new()));
        match kind {
            kind::SPAWN => {
                let mut head = Reader(r.0);
                head.str().map_err(bad)?;
                head.u16().map_err(bad)?;
                head.bytes().map_err(bad)?;
                head.u8().map_err(bad)?;
                let spec = Spec::read(&mut head).map_err(bad)?;
                let child = spawn::spawn(&spec).map_err(|e| format!("spawn {}: {e}", String::from_utf8_lossy(spec.argv.first().map_or(&[][..], Vec::as_slice))))?;
                let state = Child { pid: child.pid, pidfd: child.pidfd, status: Status::Running, leader: spec.setsid, unreported: false };
                let handle = self.insert(me, r, child.fds, Some(state))?;
                self.handed(handle)
            }
            kind::STORE => {
                let fds = self.conns[index].fds.drain(..).collect();
                let handle = self.insert(me, r, fds, None)?;
                Ok((self.table.row(handle).expect("present").write(Writer::default()), Vec::new()))
            }
            kind::LIST => {
                let rows = self.table.handles.keys().filter_map(|h| self.table.row(*h));
                Ok((rows.fold(Writer::default().u32(count(self.table.handles.len())), |w, row| row.write(w)), Vec::new()))
            }
            kind::CLAIM => {
                let handle = self.handle(r)?;
                let supersedes = r.u32().map_err(bad)?.cast_signed();
                self.take(handle, me, supersedes)?;
                self.handed(handle)
            }
            kind::CLOSE_SLOT => {
                let handle = self.handle(r)?;
                let slot = r.u32().map_err(bad)? as usize;
                self.table.handles.get_mut(&handle).and_then(|e| e.slots.get_mut(slot)).ok_or("no such slot")?.take();
                done()
            }
            kind::CLOSE => {
                let handle = self.handle(r)?;
                self.close(handle, r.u32().map_err(bad)?.cast_signed());
                done()
            }
            kind::SIGNAL => {
                let handle = self.handle(r)?;
                let (signal, group) = (r.u32().map_err(bad)?.cast_signed(), r.u8().map_err(bad)? != 0);
                let child = self.table.handles[&handle].child.as_ref().ok_or_else(|| format!("handle {handle} has no child"))?;
                if child.status.ended() { return Err(format!("the child of handle {handle} has exited")); }
                spawn::signal(&child.pidfd, (group && child.leader).then_some(child.pid), signal).map_err(|e| e.to_string())?;
                done()
            }
            kind::SET => {
                let handle = self.handle(r)?;
                let what = r.u8().map_err(bad)?;
                let entry = self.table.handles.get_mut(&handle).expect("checked");
                match what {
                    0 => entry.metadata = r.bytes().map_err(bad)?.to_vec(),
                    1 => entry.kept = r.u8().map_err(bad)? != 0,
                    _ => {
                        self.table.timers.remove(&(entry.deadline, handle));
                        entry.deadline = r.u64().map_err(bad)?;
                        if entry.deadline != 0 { self.table.timers.insert((entry.deadline, handle)); }
                    }
                }
                done()
            }
            kind::CONFIGURE => {
                self.table.idle_ms = r.u64().map_err(bad)?.max(1);
                done()
            }
            kind::DETACH => {
                if r.u8().map_err(bad)? != 0 {
                    for handle in self.owned_where(|e| e.owner == me && !e.kept) { self.close(handle, libc::SIGHUP); }
                }
                done()
            }
            kind::CLOSE_ORPHANS => {
                let orphans = self.owned_where(|e| !e.kept);
                let orphans: Vec<Handle> = orphans.into_iter().filter(|h| !self.table.owner_alive(self.table.handles[h].owner)).collect();
                for handle in &orphans { self.close(*handle, libc::SIGHUP); }
                Ok((Writer::default().u32(count(orphans.len())), Vec::new()))
            }
            kind::SHUTDOWN => {
                for handle in self.owned_where(|_| true) { self.close(handle, libc::SIGHUP); }
                self.stop = true;
                done()
            }
            kind::INFO => Ok((Writer::default().u32(std::process::id()).u64(self.exe_hash).u64(self.table.idle_ms)
                              .u64(self.table.handles.len() as u64), Vec::new())),
            other => Err(format!("this keeper has no request {other}")),
        }
    }

    fn owned_where(&self, keep: impl Fn(&Entry) -> bool) -> Vec<Handle> {
        self.table.handles.iter().filter(|(_, e)| !e.closing && keep(e)).map(|(h, _)| *h).collect()
    }

    /// Make ATTACHMENT the owner of HANDLE, if no other live session is.
    /// A live owner whose pid is SUPERSEDES is killed first: a duplicate
    /// cannot be revoked, so the old session must be gone before the new
    /// one is handed the descriptors.
    fn take(&mut self, handle: Handle, attachment: u64, supersedes: i32) -> Result<(), String> {
        let owner = self.table.handles[&handle].owner;
        let mut killed = None;
        if owner != attachment && let Some(Attachment { pid, pidfd: Some(pidfd), .. }) = self.table.attachments.get(&owner) {
            if *pid != supersedes {
                return Err(format!("handle {handle} is owned by session {pid}, which is alive"));
            }
            let _ = rustix::process::pidfd_send_signal(pidfd, Signal::KILL);
            if !readable(pidfd.as_fd(), GRACE) {
                return Err(format!("session {pid}, which owns handle {handle}, did not die"));
            }
            killed = Some(owner);
        }
        self.table.set_owner(handle, attachment);
        let entry = self.table.handles.get_mut(&handle).expect("present");
        // A claim answers the deadline: the handle has an owner again.
        self.table.timers.remove(&(entry.deadline, handle));
        entry.deadline = 0;
        if let Some(child) = &mut entry.child { child.unreported = false; }
        // Only now is the old session's end taken in: what it still owns
        // past its deadline goes, and this handle is no longer among it.
        if let Some(owner) = killed { self.session_ended(owner); }
        Ok(())
    }

    /// Close HANDLE's descriptors and signal its child. The handle is
    /// forgotten at once without a live child, else when the child is reaped.
    fn close(&mut self, handle: Handle, signal: i32) {
        let Some(entry) = self.table.handles.get_mut(&handle) else { return };
        entry.slots.clear();
        self.table.timers.remove(&(entry.deadline, handle));
        match &entry.child {
            Some(child) if !child.status.ended() => {
                if signal != 0 { let _ = spawn::signal(&child.pidfd, child.leader.then_some(child.pid), signal); }
                entry.closing = true;
                entry.deadline = now_ms() + GRACE_MS;
                self.table.timers.insert((entry.deadline, handle));
            }
            _ => self.table.remove(handle),
        }
    }

    fn timers(&mut self) {
        let now = now_ms();
        while let Some(&(at, handle)) = self.table.timers.first().filter(|(at, _)| *at <= now) {
            self.table.timers.remove(&(at, handle));
            let Some(entry) = self.table.handles.get_mut(&handle) else { continue };
            if entry.closing {
                // Its signal was not enough.
                if let Some(child) = &entry.child { let _ = spawn::signal(&child.pidfd, child.leader.then_some(child.pid), libc::SIGKILL); }
            } else if entry.deadline == at && !self.table.owner_alive(self.table.handles[&handle].owner) {
                // A deadline is for a handle nobody claimed in time. Under
                // a live owner it stays set, and holds when that owner goes.
                self.close(handle, libc::SIGHUP);
            }
        }
    }

    /// Take every pending change of every child, the keeper's own and the
    /// orphans it inherited as subreaper.
    fn reap(&mut self) {
        while let Some((pid, _)) = spawn::wait(Whom::PeekAny) {
            let Some(&handle) = self.table.pids.get(&pid) else {
                // An orphan: nobody asks for its status.
                let _ = spawn::wait(Whom::Pid(pid));
                continue;
            };
            let entry = self.table.handles.get_mut(&handle).expect("indexed");
            let child = entry.child.as_mut().expect("indexed");
            let Some((_, status)) = spawn::wait(Whom::Child(&child.pidfd)) else { break };
            (child.status, child.unreported) = (status, true);
            if status.ended() {
                self.table.pids.remove(&pid);
                if entry.closing { self.table.remove(handle); continue; }
            }
            self.report(handle);
        }
    }

    /// Tell HANDLE's owner its child's status, if the owner is connected.
    fn report(&mut self, handle: Handle) {
        let owner = self.table.handles[&handle].owner;
        let Some(index) = self.conns.iter().position(|c| c.attachment == owner && c.greeted && !c.dead) else { return };
        let row = self.table.row(handle).expect("present");
        self.send(index, kind::EVENT, &row.write(Writer::default().u8(0)).finish(), Vec::new());
        if let Some(child) = &mut self.table.handles.get_mut(&handle).expect("present").child { child.unreported = false; }
    }

    /// Replace this process by the keeper at PATH. Returns only when the
    /// exec itself failed after the check passed; this keeper then goes on,
    /// and the sessions that reconnect find it unchanged.
    fn exec(&mut self, path: &std::path::Path) {
        // Sessions reconnect; everything they were told so far must arrive.
        for index in 0..self.conns.len() {
            self.send(index, kind::EVENT, &[1], Vec::new());
            let _ = self.conns[index].socket.set_nonblocking(false);
            let _ = self.conns[index].socket.set_write_timeout(Some(std::time::Duration::from_millis(GRACE_MS)));
            self.flush(index);
        }
        // Dropped at the end of this pass if the exec fails; gone with the
        // old image if it succeeds.
        for conn in &mut self.conns { conn.dead = true; }
        self.table.listener = self.listener.try_clone().ok().map(OwnedFd::from);
        crate::exec::exec(&self.table, self.build, path);
        self.table.listener = None;
    }
}

/// Bind PATH and make a fresh table around the socket.
///
/// # Errors
/// The system's refusal to bind or listen.
pub fn fresh(path: &std::path::Path) -> io::Result<Table> {
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    let mut entropy = [0u8; 8];
    let _ = rustix::rand::getrandom(&mut entropy, rustix::rand::GetRandomFlags::empty());
    Ok(Table { epoch: u64::from_ne_bytes(entropy) | 1, listener: Some(listener.into()), socket_path: path.as_os_str().as_encoded_bytes().to_vec(),
               idle_ms: DEFAULT_IDLE_MS, ..Table::default() })
}

pub(crate) fn clear_cloexec(fds: &[BorrowedFd<'_>]) { for fd in fds { set_cloexec(*fd, false); } }
pub(crate) fn restore_cloexec(fds: &[BorrowedFd<'_>]) { for fd in fds { set_cloexec(*fd, true); } }
