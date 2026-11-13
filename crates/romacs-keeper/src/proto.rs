//! The keeper's records: class [`CLASS`], versions [`VERSIONS`].
//!
//! A session sends requests; each gets one [`REPLY`] carrying the request's
//! sequence number, a status byte (0 ok, 1 refused with a message), and the
//! body. Descriptors ride with the reply that names them. The keeper sends
//! [`EVENT`]s unasked: a child's change of state, and its own exec.

use romacs_record::{ClassRange, Error, Handle, Reader, Writer};

use crate::spawn::Status;

pub const CLASS: u16 = 1;
pub const NAME: &str = "keeper";
pub const VERSIONS: std::ops::RangeInclusive<u16> = 1..=1;

/// A count as records carry it.
pub(crate) fn count(items: usize) -> u32 { u32::try_from(items).expect("fewer than 2^32 items") }

#[must_use]
pub fn class() -> ClassRange { ClassRange::new(CLASS, NAME, VERSIONS) }

/// Request kinds. Each line: the body, then the reply's body.
pub mod kind {
    /// tag str, version u16, metadata bytes, kept u8, spec. Reply: the row,
    /// with the session's duplicates of its descriptors.
    pub const SPAWN: u16 = 1;
    /// tag str, version u16, metadata bytes, kept u8, with descriptors.
    /// Reply: the row. The keeper keeps the descriptors it was sent.
    pub const STORE: u16 = 2;
    /// Reply: count u32, rows.
    pub const LIST: u16 = 3;
    /// handle u64, supersedes u32 (a pid whose session this one replaces,
    /// or 0). Reply: the row, with duplicates of its open descriptors.
    pub const CLAIM: u16 = 4;
    /// handle u64, slot u32: close the keeper's copy of one descriptor.
    pub const CLOSE_SLOT: u16 = 5;
    /// handle u64, signal u32 (0 for none): close every descriptor, signal
    /// the child, forget the handle once the child is reaped.
    pub const CLOSE: u16 = 6;
    /// handle u64, signal u32, group u8.
    pub const SIGNAL: u16 = 7;
    /// handle u64, what u8 (0 metadata bytes, 1 kept u8, 2 deadline u64 as
    /// milliseconds since the epoch, 0 for none).
    pub const SET: u16 = 8;
    /// idle exit u64, milliseconds.
    pub const CONFIGURE: u16 = 9;
    /// quit u8: 1 closes this session's handles that are not kept.
    pub const DETACH: u16 = 10;
    /// Close every handle whose owner is gone and that is not kept.
    /// Reply: count u32.
    pub const CLOSE_ORPHANS: u16 = 11;
    /// path bytes: replace the keeper by this executable, in place.
    pub const EXEC: u16 = 12;
    /// Close everything and exit.
    pub const SHUTDOWN: u16 = 13;
    /// Reply: pid u32, executable hash u64, idle exit u64, handles u64.
    pub const INFO: u16 = 14;
    /// in reply to u64, status u8, then the body or a message str.
    pub const REPLY: u16 = 100;
    /// what u8: 0 a row whose child changed state; 1 the keeper is about to
    /// exec (reconnect; handles and duplicates stay good).
    pub const EVENT: u16 = 101;
}

/// One handle as a session sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub handle: Handle,
    /// What kind of resource the owner says it is; opaque to the keeper.
    pub tag: String,
    pub version: u16,
    pub metadata: Vec<u8>,
    pub kept: bool,
    /// Milliseconds since the epoch after which an unowned handle is
    /// closed; 0 for never.
    pub deadline: u64,
    /// The child's pid, or 0 for a handle without one.
    pub pid: u32,
    pub status: Status,
    /// The owning session's pid, or 0.
    pub owner: u32,
    pub owner_alive: bool,
    /// Which descriptor slots are still open, lowest bit first.
    pub open: u64,
}

impl Row {
    #[must_use]
    pub fn write(&self, w: Writer) -> Writer {
        let w = w.u64(self.handle.raw()).str(&self.tag).u16(self.version).bytes(&self.metadata).u8(u8::from(self.kept))
            .u64(self.deadline).u32(self.pid);
        self.status.write(w).u32(self.owner).u8(u8::from(self.owner_alive)).u64(self.open)
    }

    /// # Errors
    /// [`Error::Truncated`] or [`Error::Utf8`].
    pub fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        Ok(Self {
            handle: Handle::from_raw(r.u64()?).ok_or(Error::Truncated)?,
            tag: r.str()?.into(),
            version: r.u16()?,
            metadata: r.bytes()?.to_vec(),
            kept: r.u8()? != 0,
            deadline: r.u64()?,
            pid: r.u32()?,
            status: Status::read(r)?,
            owner: r.u32()?,
            owner_alive: r.u8()? != 0,
            open: r.u64()?,
        })
    }
}
