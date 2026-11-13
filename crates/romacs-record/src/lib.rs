//! Flat, versioned records between Romacs's processes.
//!
//! Two processes that may be different builds talk in records. This crate
//! is what both must agree on, and it depends on nothing of the VM, the
//! display or Lisp, so the long-lived side can stay small and change rarely.
//!
//! - [`record`]: the fixed header (sequence number, class and kind, length)
//!   and an incremental decoder. A record's meaning belongs to its class.
//! - [`flat`]: the fields of a payload, little-endian, length-prefixed.
//! - [`hello`]: the first record each side sends. Its head is frozen: a
//!   magic and the framing hash, which covers this crate's wire layout and
//!   nothing else. After it come the classes the sender speaks, each with a
//!   version range. A class is usable when the two ranges overlap; one that
//!   does not overlap is refused by name while the others keep working.
//! - [`handle`]: names for what the long-lived side holds. Allocated
//!   counting up, never reused; meaningful within one [`hello::Hello::epoch`].
//! - [`region`]: large payloads by shared memory. The writer makes one
//!   sealed memfd, passes it once, and names payloads by offset and length;
//!   the reader copies a payload out and acknowledges it, which frees it.
//! - [`channel`]: records over a unix socket, with descriptors beside them.
//!
//! # Adding to the protocol
//!
//! - A new record kind in a class: raise the class's highest version. A
//!   peer that agreed on a lower version never receives the kind.
//! - A new class: a new id and name. A peer without it reports it absent.
//! - A change to this crate's wire layout: edit [`hello::FRAMING`], whose
//!   hash every peer checks first. Such peers refuse each other whole, so
//!   this is the change to avoid.

pub mod channel;
pub mod flat;
pub mod handle;
pub mod hello;
pub mod record;
pub mod region;

pub use flat::{Reader, Writer};
pub use handle::{Handle, HandleAllocator};
pub use hello::{Agreement, ClassRange, Hello};
pub use record::{Decoder, Record};
pub use region::{RegionReader, RegionWriter, Span};

/// What a peer's bytes can get wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A payload ended inside a field.
    Truncated,
    /// A string field is not UTF-8.
    Utf8,
    /// A header announces more than [`record::MAX_PAYLOAD`].
    Oversized(u32),
    /// The peer's first record is not a hello of this framing.
    Framing { ours: u64, theirs: u64 },
    /// The peer's first bytes are not a hello at all.
    NotHello,
    /// A span lies outside its region, or was never allocated.
    Span(Span),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Truncated => write!(f, "record payload ends inside a field"),
            Error::Utf8 => write!(f, "record string is not UTF-8"),
            Error::Oversized(len) => write!(f, "record of {len} bytes exceeds the {} byte limit", record::MAX_PAYLOAD),
            Error::Framing { ours, theirs } => write!(f, "protocol framing differs: ours {ours:016x}, the peer's {theirs:016x}"),
            Error::NotHello => write!(f, "the peer did not begin with a hello record"),
            Error::Span(span) => write!(f, "span {}+{} is not in the region", span.offset, span.len),
        }
    }
}

impl std::error::Error for Error {}
