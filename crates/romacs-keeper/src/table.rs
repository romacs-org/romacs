//! The handle table: everything the keeper knows, as plain data.
//!
//! No state lives anywhere else (no thread, no closure), so an exec in
//! place carries the keeper whole: [`Table::export`] writes the table to a
//! flat buffer, descriptors as their numbers, and [`Table::import`] reads
//! it back in the new program, which inherited those descriptors.
//!
//! The buffer is versioned. Each entry is length-prefixed, so a build reads
//! the fields it knows of an entry a newer build of the same version line
//! wrote; a build refuses a table newer than [`TABLE_VERSION`].

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

use romacs_record::{Error, Handle, HandleAllocator, Reader, Writer};

use crate::proto::{count, Row};
use crate::spawn::Status;

pub const TABLE_MAGIC: &[u8; 8] = b"ROMACSKT";
pub const TABLE_VERSION: u16 = 1;

/// A child the keeper is the parent of.
pub struct Child {
    pub pid: i32,
    pub pidfd: OwnedFd,
    pub status: Status,
    /// Whether it leads a session, so signals may go to its group.
    pub leader: bool,
    /// Whether its owner has not been sent the current status.
    pub unreported: bool,
}

pub struct Entry {
    pub tag: String,
    pub version: u16,
    pub metadata: Vec<u8>,
    pub kept: bool,
    pub deadline: u64,
    /// The kept descriptors; a closed slot keeps its place.
    pub slots: Vec<Option<OwnedFd>>,
    pub child: Option<Child>,
    /// The owning attachment, or 0.
    pub owner: u64,
    /// Closed, waiting only for its child to be reaped; then forgotten.
    pub closing: bool,
}

/// A session process that attached. It is the session while its process
/// lives, whatever happens to its connection.
pub struct Attachment {
    pub pid: i32,
    /// `None` once the process has exited.
    pub pidfd: Option<OwnedFd>,
    /// How many handles name it as owner.
    pub owned: usize,
}

#[derive(Default)]
pub struct Table {
    /// This keeper's incarnation: kept across an exec, new in a new keeper.
    pub epoch: u64,
    pub allocator: HandleAllocator,
    pub handles: BTreeMap<Handle, Entry>,
    /// Whose child a pid is.
    pub pids: HashMap<i32, Handle>,
    pub attachments: BTreeMap<u64, Attachment>,
    pub last_attachment: u64,
    pub idle_ms: u64,
    /// Deadlines and kill times, as (milliseconds since the epoch, handle).
    pub timers: BTreeSet<(u64, Handle)>,
    /// The listening socket, kept across an exec so no connection is refused.
    pub listener: Option<OwnedFd>,
    pub socket_path: Vec<u8>,
    /// The lock that makes this the one keeper of its socket, held for
    /// the keeper's life.
    pub lock: Option<OwnedFd>,
}

fn fd_number(fd: Option<&OwnedFd>) -> i64 { fd.map_or(-1, |fd| i64::from(fd.as_raw_fd())) }

/// # Safety
/// NUMBER, when not negative, must be an open descriptor nothing else owns.
unsafe fn fd_owned(number: i64) -> Option<OwnedFd> {
    // SAFETY: the caller's contract.
    RawFd::try_from(number).ok().filter(|fd| *fd >= 0).map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
}

impl Table {
    #[must_use]
    pub fn owner_alive(&self, owner: u64) -> bool {
        self.attachments.get(&owner).is_some_and(|a| a.pidfd.is_some())
    }

    #[must_use]
    pub fn row(&self, handle: Handle) -> Option<Row> {
        let entry = self.handles.get(&handle)?;
        let owner = self.attachments.get(&entry.owner);
        Some(Row {
            handle,
            tag: entry.tag.clone(),
            version: entry.version,
            metadata: entry.metadata.clone(),
            kept: entry.kept,
            deadline: entry.deadline,
            pid: entry.child.as_ref().map_or(0, |c| c.pid.cast_unsigned()),
            status: entry.child.as_ref().map_or(Status::Running, |c| c.status),
            owner: owner.map_or(0, |a| a.pid.cast_unsigned()),
            owner_alive: owner.is_some_and(|a| a.pidfd.is_some()),
            open: entry.slots.iter().enumerate().fold(0, |bits, (i, slot)| bits | u64::from(slot.is_some() && i < 64) << i),
        })
    }

    /// Make ATTACHMENT (or nobody, 0) the owner of HANDLE.
    pub fn set_owner(&mut self, handle: Handle, attachment: u64) {
        let Some(entry) = self.handles.get_mut(&handle) else { return };
        let previous = std::mem::replace(&mut entry.owner, attachment);
        if let Some(a) = self.attachments.get_mut(&attachment) { a.owned += 1; }
        if let Some(a) = self.attachments.get_mut(&previous) {
            a.owned -= 1;
            if a.owned == 0 && a.pidfd.is_none() { self.attachments.remove(&previous); }
        }
    }

    pub fn remove(&mut self, handle: Handle) {
        self.set_owner(handle, 0);
        if let Some(child) = self.handles.remove(&handle).and_then(|entry| entry.child) { self.pids.remove(&child.pid); }
    }

    /// The table as a flat buffer, and every descriptor it names: those
    /// must stay open across the exec.
    #[must_use]
    pub fn export(&self, extra: bool) -> (Vec<u8>, Vec<RawFd>) {
        let mut fds = Vec::new();
        let mut number = |fd: Option<&OwnedFd>| { fds.extend(fd.map(AsRawFd::as_raw_fd)); fd_number(fd) };
        let version = TABLE_VERSION + u16::from(extra);
        let mut w = Writer(TABLE_MAGIC.to_vec()).u16(version).u64(self.epoch).u64(self.allocator.allocated())
            .u64(self.last_attachment).u64(self.idle_ms).i64(number(self.listener.as_ref())).bytes(&self.socket_path).i64(number(self.lock.as_ref()))
            .u32(count(self.attachments.len()));
        for (id, a) in &self.attachments {
            w = w.bytes(&Writer::default().u64(*id).u32(a.pid.cast_unsigned()).i64(number(a.pidfd.as_ref())).finish());
        }
        w = w.u32(count(self.handles.len()));
        for (handle, entry) in &self.handles {
            let mut e = Writer::default().u64(handle.raw()).str(&entry.tag).u16(entry.version).bytes(&entry.metadata)
                .u8(u8::from(entry.kept)).u64(entry.deadline).u64(entry.owner).u8(u8::from(entry.closing)).u32(count(entry.slots.len()));
            for slot in &entry.slots { e = e.i64(number(slot.as_ref())); }
            e = match &entry.child {
                None => e.u8(0),
                Some(c) => c.status.write(e.u8(1).u32(c.pid.cast_unsigned()).i64(number(Some(&c.pidfd)))).u8(u8::from(c.leader)).u8(u8::from(c.unreported)),
            };
            // What a later version adds goes here, after all a version-1 reader takes.
            if extra { e = e.u64(0x6e65_7874); }
            w = w.bytes(&e.finish());
        }
        w = w.u32(count(self.timers.len()));
        for (at, handle) in &self.timers { w = w.u64(*at).u64(handle.raw()); }
        (w.finish(), fds)
    }

    /// Check that BYTES is a table this build can resume from. Pure: it
    /// takes no descriptor, so a new binary can run it on a copy before
    /// the old one execs it.
    ///
    /// # Errors
    /// What is wrong with the table, in words.
    pub fn check(bytes: &[u8], newest: u16) -> Result<(), String> {
        Self::parse(bytes, newest, false).map(drop)
    }

    /// Read a table [`Self::export`] wrote, taking its descriptors. A table
    /// that does not read takes none: it is checked whole first.
    ///
    /// # Errors
    /// What is wrong with the table, in words.
    ///
    /// # Safety
    /// Every descriptor number in BYTES must be open in this process and
    /// owned by nothing else: true in the process that exported the table
    /// and then exec'd with them inherited.
    pub unsafe fn import(bytes: &[u8], newest: u16) -> Result<Self, String> {
        Self::check(bytes, newest)?;
        Self::parse(bytes, newest, true)
    }

    fn parse(bytes: &[u8], newest: u16, own: bool) -> Result<Self, String> {
        let truncated = |e: Error| format!("keeper table: {e}");
        if bytes.len() < 8 || &bytes[..8] != TABLE_MAGIC { return Err("not a keeper table".into()); }
        let mut r = Reader(&bytes[8..]);
        let version = r.u16().map_err(truncated)?;
        if version == 0 || version > newest {
            return Err(format!("keeper table version {version} is newer than this build reads ({newest})"));
        }
        Self::read(&mut r, own).map_err(truncated)
    }

    /// With OWN, wrap each descriptor number; without, only read the bytes.
    fn read(r: &mut Reader<'_>, own: bool) -> Result<Self, Error> {
        // SAFETY: OWN is set only by `import`, whose caller promises every
        // number in the table is an open descriptor owned by nothing else.
        let fd = |number: i64| if own { unsafe { fd_owned(number) } } else { None };
        let mut table = Table { epoch: r.u64()?, allocator: HandleAllocator::resume(r.u64()?), ..Table::default() };
        (table.last_attachment, table.idle_ms) = (r.u64()?, r.u64()?);
        table.listener = fd(r.i64()?);
        table.socket_path = r.bytes()?.to_vec();
        table.lock = fd(r.i64()?);
        for _ in 0..r.u32()? {
            let mut a = Reader(r.bytes()?);
            let (id, pid) = (a.u64()?, a.u32()?.cast_signed());
            table.attachments.insert(id, Attachment { pid, pidfd: fd(a.i64()?), owned: 0 });
        }
        for _ in 0..r.u32()? {
            let mut e = Reader(r.bytes()?);
            let handle = Handle::from_raw(e.u64()?).ok_or(Error::Truncated)?;
            let mut entry = Entry { tag: e.str()?.into(), version: e.u16()?, metadata: e.bytes()?.to_vec(), kept: e.u8()? != 0,
                                    deadline: e.u64()?, owner: e.u64()?, closing: e.u8()? != 0, slots: Vec::new(), child: None };
            for _ in 0..e.u32()? { entry.slots.push(fd(e.i64()?)); }
            if e.u8()? != 0 {
                let (pid, number) = (e.u32()?.cast_signed(), e.i64()?);
                if number < 0 { return Err(Error::Truncated); }
                let pidfd = fd(number);
                let (status, leader, unreported) = (Status::read(&mut e)?, e.u8()? != 0, e.u8()? != 0);
                table.pids.insert(pid, handle);
                entry.child = pidfd.map(|pidfd| Child { pid, pidfd, status, leader, unreported });
            }
            if let Some(a) = table.attachments.get_mut(&entry.owner) { a.owned += 1; }
            table.handles.insert(handle, entry);
        }
        for _ in 0..r.u32()? {
            let at = r.u64()?;
            table.timers.insert((at, Handle::from_raw(r.u64()?).ok_or(Error::Truncated)?));
        }
        Ok(table)
    }
}
