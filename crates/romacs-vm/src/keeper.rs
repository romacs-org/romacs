//! This session's link to its keeper (crate romacs-keeper).
//!
//! The keeper holds what must outlive the session process: descriptors,
//! and the children it is the parent of. The session holds duplicates of
//! the descriptors of the handles it owns and reads and writes them
//! directly; nothing flows through the keeper. `lisp/romacs/romacs-keeper.el` is
//! the Lisp face of this; the policy (whether to use a keeper, its idle
//! time, deadlines) lives there.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};

use romacs_keeper::proto::Row;
use romacs_keeper::{Client, Event};

pub(crate) struct Link {
    pub(crate) client: Client,
    /// This session's duplicates, by handle and slot.
    fds: HashMap<u64, Vec<Option<OwnedFd>>>,
}

impl Link {
    pub(crate) fn new(client: Client) -> Self { Self { client, fds: HashMap::new() } }

    /// Keep the duplicates the keeper handed with ROW: one per open slot,
    /// in slot order.
    pub(crate) fn hold(&mut self, row: &Row, handed: Vec<OwnedFd>) {
        let mut handed = handed.into_iter();
        let slots = (0..64).take_while(|slot| row.open >> slot != 0).map(|slot| if row.open >> slot & 1 == 1 { handed.next() } else { None });
        self.fds.insert(row.handle.raw(), slots.collect());
    }

    pub(crate) fn forget(&mut self, handle: u64) { self.fds.remove(&handle); }

    pub(crate) fn drop_slot(&mut self, handle: u64, slot: usize) {
        if let Some(fd) = self.fds.get_mut(&handle).and_then(|slots| slots.get_mut(slot)) { fd.take(); }
    }

    fn slot(&self, handle: u64, slot: usize) -> Result<&OwnedFd, String> {
        self.fds.get(&handle).and_then(|slots| slots.get(slot)).and_then(Option::as_ref)
            .ok_or_else(|| format!("This session holds no descriptor {slot} of keeper handle {handle}"))
    }

    /// A duplicate of a held descriptor, for a part of the session that
    /// talks to the resource itself (a terminal's helper).
    pub(crate) fn duplicate(&self, handle: u64, slot: usize) -> Result<OwnedFd, String> {
        self.slot(handle, slot)?.try_clone().map_err(|e| e.to_string())
    }

    /// Up to MAX bytes from a held descriptor: `None` when nothing arrives
    /// within TIMEOUT, empty at end of file.
    pub(crate) fn read(&self, handle: u64, slot: usize, max: usize, timeout: std::time::Duration) -> Result<Option<Vec<u8>>, String> {
        let fd = self.slot(handle, slot)?;
        let ready = crate::subprocess::poll(&[(fd.as_raw_fd(), libc::POLLIN)], Some(timeout)).map_err(|e| e.to_string())?;
        if ready.first().is_none_or(|events| *events == 0) { return Ok(None); }
        let mut bytes = vec![0; max.clamp(1, 1 << 20)];
        let count = std::fs::File::from(fd.try_clone().map_err(|e| e.to_string())?).read(&mut bytes).map_err(|e| e.to_string())?;
        bytes.truncate(count);
        Ok(Some(bytes))
    }

    /// Write what the descriptor takes of BYTES without waiting past
    /// TIMEOUT for room; how many bytes went. The caller sends the rest
    /// later: a child that does not read must not stop the Lisp thread.
    /// Nonblocking is set on the descriptor, which only this session uses:
    /// the keeper's copy shares the flag and is never read or written.
    pub(crate) fn write(&self, handle: u64, slot: usize, bytes: &[u8], timeout: std::time::Duration) -> Result<usize, String> {
        use std::os::fd::AsFd;
        let fd = self.slot(handle, slot)?;
        crate::host::set_nonblocking(fd.as_fd()).map_err(|e| e.to_string())?;
        let ready = crate::subprocess::poll(&[(fd.as_raw_fd(), libc::POLLOUT)], Some(timeout)).map_err(|e| e.to_string())?;
        if ready.first().is_none_or(|events| *events == 0) { return Ok(0); }
        match std::fs::File::from(fd.try_clone().map_err(|e| e.to_string())?).write(bytes) {
            Ok(count) => Ok(count),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(0),
            Err(error) => Err(error.to_string()),
        }
    }

    /// What the keeper said unasked. After its exec this finds it again:
    /// handles and duplicates stay good.
    pub(crate) fn events(&mut self) -> Result<Vec<Event>, String> {
        let events = self.client.pump()?;
        if events.contains(&Event::Exec) { crate::host::keeper_reconnect(&mut self.client)?; }
        Ok(events)
    }
}
