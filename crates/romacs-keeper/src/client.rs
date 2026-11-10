//! A session's side of its keeper.
//!
//! Blocking and small: a request is written and its reply awaited, with a
//! bound. Events that arrive meanwhile queue in [`Client::events`]. The
//! socket is also a descriptor for the session's wait loop: readable means
//! [`Client::pump`] has something.

use std::collections::VecDeque;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use romacs_record::{channel, hello, Agreement, ClassRange, Decoder, Handle, Hello, Reader, Record, Writer};

use crate::proto::{self, kind, Row};
use crate::spawn::Spec;

/// What the keeper says unasked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A handle's child changed state.
    Status(Row),
    /// The keeper is replacing itself; [`Client::reconnect`] finds it again.
    Exec,
}

pub struct Client {
    socket: UnixStream,
    decoder: Decoder,
    fds: VecDeque<OwnedFd>,
    seq: u64,
    path: PathBuf,
    /// What the keeper said of itself: its pid and its incarnation.
    pub keeper: Hello,
    /// How each class this session speaks fares with this keeper.
    pub classes: Vec<(ClassRange, Agreement)>,
    pub events: VecDeque<Event>,
}

/// What a keeper answers to [`Client::info`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub pid: u32,
    pub executable: u64,
    pub idle_ms: u64,
    pub handles: u64,
}

/// How long a request waits for its reply.
const PATIENCE: Duration = Duration::from_secs(10);

impl Client {
    /// Attach to the keeper listening at PATH, speaking CLASSES.
    ///
    /// # Errors
    /// No keeper there; a keeper of another framing, with both hashes; or
    /// no keeper-class version in common, by name.
    pub fn connect(path: &Path, classes: Vec<ClassRange>) -> Result<Self, String> {
        let socket = UnixStream::connect(path).map_err(|e| format!("no keeper at {}: {e}", path.display()))?;
        socket.set_read_timeout(Some(PATIENCE)).map_err(|e| e.to_string())?;
        let mut wire = Vec::new();
        Hello { epoch: 0, pid: std::process::id(), classes: classes.clone() }.record().encode_into(&mut wire);
        channel::send_all(&socket, &wire, &[]).map_err(|e| format!("the keeper at {} hung up: {e}", path.display()))?;
        let mut client = Self { socket, decoder: Decoder::default(), fds: VecDeque::new(), seq: 0, path: path.into(),
                                keeper: Hello { epoch: 0, pid: 0, classes: Vec::new() }, classes: Vec::new(), events: VecDeque::new() };
        let first = client.record()?;
        client.keeper = Hello::read(&first).map_err(|e| format!("the keeper at {}: {e}", path.display()))?;
        client.classes = classes.into_iter().map(|ours| { let agreed = hello::agree(&ours, &client.keeper.classes); (ours, agreed) }).collect();
        client.version(proto::CLASS)?;
        Ok(client)
    }

    /// Attach speaking this build's keeper class.
    ///
    /// # Errors
    /// As [`Self::connect`].
    pub fn attach(path: &Path) -> Result<Self, String> { Self::connect(path, vec![proto::class()]) }

    /// The version of class ID agreed with this keeper.
    ///
    /// # Errors
    /// Why there is none, naming the class.
    pub fn version(&self, id: u16) -> Result<u16, String> {
        self.classes.iter().find(|(ours, _)| ours.id == id).ok_or_else(|| format!("this session does not speak class {id}"))?.1.version()
    }

    fn gone(&self) -> String { format!("the keeper (pid {}) of this session is gone", self.keeper.pid) }

    /// The next record, waiting for it.
    fn record(&mut self) -> Result<Record, String> {
        let mut buffer = [0u8; 16384];
        loop {
            if let Some(record) = self.decoder.pop().map_err(|e| e.to_string())? { return Ok(record); }
            let mut fds = Vec::new();
            match channel::receive(&self.socket, &mut buffer, &mut fds) {
                Ok(0) => return Err(self.gone()),
                Ok(count) => { self.decoder.push(&buffer[..count]); self.fds.extend(fds); }
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) =>
                    return Err(format!("the keeper (pid {}) did not answer", self.keeper.pid)),
                Err(_) => return Err(self.gone()),
            }
        }
    }

    fn event(&mut self, record: &Record) -> Result<(), String> {
        let mut r = Reader(&record.payload);
        let event = match r.u8().map_err(|e| e.to_string())? {
            0 => Event::Status(Row::read(&mut r).map_err(|e| e.to_string())?),
            _ => Event::Exec,
        };
        self.events.push_back(event);
        Ok(())
    }

    /// Send a request and await its reply's body and descriptors.
    ///
    /// # Errors
    /// The keeper's refusal, in its words, or its death, by pid.
    pub fn request(&mut self, kind: u16, payload: &[u8], fds: &[BorrowedFd<'_>]) -> Result<(Vec<u8>, Vec<OwnedFd>), String> {
        self.seq += 1;
        let mut wire = Vec::new();
        romacs_record::record::encode(self.seq, proto::CLASS, kind, payload, &mut wire);
        channel::send_all(&self.socket, &wire, fds).map_err(|_| self.gone())?;
        loop {
            let record = self.record()?;
            if record.kind == kind::EVENT { self.event(&record)?; continue; }
            let mut r = Reader(&record.payload);
            let (seq, refused) = (r.u64().map_err(|e| e.to_string())?, r.u8().map_err(|e| e.to_string())? != 0);
            if seq != self.seq { continue; }
            if refused { return Err(r.str().map_err(|e| e.to_string())?.into()); }
            return Ok((r.0.to_vec(), self.fds.drain(..).collect()));
        }
    }

    /// Take what arrived unasked, without waiting.
    ///
    /// # Errors
    /// The keeper's death, by pid.
    pub fn pump(&mut self) -> Result<Vec<Event>, String> {
        self.socket.set_nonblocking(true).map_err(|e| e.to_string())?;
        let result = loop {
            match self.record() {
                Ok(record) if record.kind == kind::EVENT => self.event(&record)?,
                Ok(_) => {}
                Err(e) if e.ends_with("did not answer") => break Ok(()),
                Err(e) => break Err(e),
            }
        };
        self.socket.set_nonblocking(false).map_err(|e| e.to_string())?;
        // An exec's notice is the last thing an old keeper sends.
        if self.events.back() == Some(&Event::Exec) { return Ok(self.events.drain(..).collect()); }
        result.map(|()| self.events.drain(..).collect())
    }

    /// Find the keeper again after its exec: the same incarnation, so every
    /// handle and every duplicate this session holds is still good.
    ///
    /// # Errors
    /// No keeper answers, or another incarnation does.
    pub fn reconnect(&mut self) -> Result<(), String> {
        let deadline = std::time::Instant::now() + PATIENCE;
        let classes: Vec<ClassRange> = self.classes.iter().map(|(ours, _)| ours.clone()).collect();
        let fresh = loop {
            match Self::connect(&self.path, classes.clone()) {
                Ok(client) => break client,
                Err(e) if std::time::Instant::now() > deadline => return Err(e),
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        };
        if fresh.keeper.epoch != self.keeper.epoch {
            return Err(format!("another keeper (pid {}) answers where pid {} was: its handles are not this session's", fresh.keeper.pid, self.keeper.pid));
        }
        let events = std::mem::take(&mut self.events);
        *self = fresh;
        self.events = events;
        Ok(())
    }

    fn handed(&mut self, kind: u16, payload: &[u8], fds: &[BorrowedFd<'_>]) -> Result<(Row, Vec<OwnedFd>), String> {
        let (body, fds) = self.request(kind, payload, fds)?;
        Ok((Row::read(&mut Reader(&body)).map_err(|e| e.to_string())?, fds))
    }

    fn head(tag: &str, version: u16, metadata: &[u8], kept: bool) -> Writer {
        Writer::default().str(tag).u16(version).bytes(metadata).u8(u8::from(kept))
    }

    /// Have the keeper spawn SPEC's child and keep its descriptors. Returns
    /// the handle's row and this session's duplicates.
    ///
    /// # Errors
    /// As [`Self::request`].
    pub fn spawn(&mut self, tag: &str, version: u16, metadata: &[u8], kept: bool, spec: &Spec) -> Result<(Row, Vec<OwnedFd>), String> {
        self.handed(kind::SPAWN, &spec.write(Self::head(tag, version, metadata, kept)).finish(), &[])
    }

    /// Give the keeper FDS to keep as a new handle with no child.
    ///
    /// # Errors
    /// As [`Self::request`].
    pub fn store(&mut self, tag: &str, version: u16, metadata: &[u8], kept: bool, fds: &[BorrowedFd<'_>]) -> Result<Row, String> {
        Ok(self.handed(kind::STORE, &Self::head(tag, version, metadata, kept).finish(), fds)?.0)
    }

    /// # Errors
    /// As [`Self::request`].
    pub fn list(&mut self) -> Result<Vec<Row>, String> {
        let (body, _) = self.request(kind::LIST, &[], &[])?;
        let mut r = Reader(&body);
        (0..r.u32().map_err(|e| e.to_string())?).map(|_| Row::read(&mut r).map_err(|e| e.to_string())).collect()
    }

    /// Become HANDLE's owner and be handed duplicates of its descriptors.
    /// SUPERSEDES is the pid of the session this one replaces, which the
    /// keeper kills first if it still lives, or 0.
    ///
    /// # Errors
    /// Another live session owns the handle, by pid; or as [`Self::request`].
    pub fn claim(&mut self, handle: Handle, supersedes: u32) -> Result<(Row, Vec<OwnedFd>), String> {
        self.handed(kind::CLAIM, &Writer::default().u64(handle.raw()).u32(supersedes).finish(), &[])
    }

    fn plain(&mut self, kind: u16, payload: Writer) -> Result<Vec<u8>, String> { Ok(self.request(kind, &payload.finish(), &[])?.0) }

    /// # Errors
    /// As [`Self::request`].
    pub fn close_slot(&mut self, handle: Handle, slot: u32) -> Result<(), String> {
        self.plain(kind::CLOSE_SLOT, Writer::default().u64(handle.raw()).u32(slot)).map(drop)
    }

    /// # Errors
    /// As [`Self::request`].
    pub fn close(&mut self, handle: Handle, signal: i32) -> Result<(), String> {
        self.plain(kind::CLOSE, Writer::default().u64(handle.raw()).u32(signal.cast_unsigned())).map(drop)
    }

    /// # Errors
    /// The child has exited, so the signal reaches nothing; or as [`Self::request`].
    pub fn signal(&mut self, handle: Handle, signal: i32, group: bool) -> Result<(), String> {
        self.plain(kind::SIGNAL, Writer::default().u64(handle.raw()).u32(signal.cast_unsigned()).u8(u8::from(group))).map(drop)
    }

    /// # Errors
    /// As [`Self::request`].
    pub fn set_metadata(&mut self, handle: Handle, metadata: &[u8]) -> Result<(), String> {
        self.plain(kind::SET, Writer::default().u64(handle.raw()).u8(0).bytes(metadata)).map(drop)
    }

    /// # Errors
    /// As [`Self::request`].
    pub fn set_kept(&mut self, handle: Handle, kept: bool) -> Result<(), String> {
        self.plain(kind::SET, Writer::default().u64(handle.raw()).u8(1).u8(u8::from(kept))).map(drop)
    }

    /// # Errors
    /// As [`Self::request`].
    pub fn set_deadline(&mut self, handle: Handle, unix_ms: u64) -> Result<(), String> {
        self.plain(kind::SET, Writer::default().u64(handle.raw()).u8(2).u64(unix_ms)).map(drop)
    }

    /// # Errors
    /// As [`Self::request`].
    pub fn configure(&mut self, idle_ms: u64) -> Result<(), String> { self.plain(kind::CONFIGURE, Writer::default().u64(idle_ms)).map(drop) }

    /// Detach by intent. With QUIT the keeper closes this session's handles
    /// that are not kept; without, they stay for a successor.
    ///
    /// # Errors
    /// As [`Self::request`].
    pub fn detach(&mut self, quit: bool) -> Result<(), String> { self.plain(kind::DETACH, Writer::default().u8(u8::from(quit))).map(drop) }

    /// # Errors
    /// As [`Self::request`].
    pub fn close_orphans(&mut self) -> Result<u32, String> {
        let body = self.plain(kind::CLOSE_ORPHANS, Writer::default())?;
        Reader(&body).u32().map_err(|e| e.to_string())
    }

    /// Have the keeper replace itself by the executable at PATH, then find
    /// it again.
    ///
    /// # Errors
    /// The new binary's refusal of the table, in which case nothing
    /// changed; or as [`Self::reconnect`].
    pub fn exec(&mut self, path: &Path) -> Result<(), String> {
        self.plain(kind::EXEC, Writer::default().bytes(path.as_os_str().as_encoded_bytes()))?;
        self.reconnect()
    }

    /// # Errors
    /// As [`Self::request`].
    pub fn shutdown(&mut self) -> Result<(), String> { self.plain(kind::SHUTDOWN, Writer::default()).map(drop) }

    /// # Errors
    /// As [`Self::request`].
    pub fn info(&mut self) -> Result<Info, String> {
        let body = self.plain(kind::INFO, Writer::default())?;
        let mut r = Reader(&body);
        let bad = |e: romacs_record::Error| e.to_string();
        Ok(Info { pid: r.u32().map_err(bad)?, executable: r.u64().map_err(bad)?, idle_ms: r.u64().map_err(bad)?, handles: r.u64().map_err(bad)? })
    }
}

impl AsFd for Client {
    fn as_fd(&self) -> BorrowedFd<'_> { self.socket.as_fd() }
}
