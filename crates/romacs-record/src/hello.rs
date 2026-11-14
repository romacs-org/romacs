//! The hello: the first record each side sends, before reading anything.
//!
//! Its head is frozen for good: the 16-byte header (class 0, kind 0), then
//! [`MAGIC`] and the framing hash. Everything after the head may grow
//! behind a new framing. A peer whose head differs is refused whole, with
//! both hashes in the error.
//!
//! The rest lists the classes the sender speaks. A class is a family of
//! record kinds with one owner (terminals, pipes, a display ring). Each
//! carries the lowest and highest version the sender speaks; two peers use
//! the highest version both speak ([`agree`]).

use crate::{Error, Reader, Record, Writer};

/// The class and kind of a hello, reserved in every protocol on this crate.
pub const HELLO: (u16, u16) = (0, 0);

pub const MAGIC: [u8; 8] = *b"ROMACSR\n";

/// This crate's wire layout, in words. Edit it when the layout changes:
/// its hash is what peers compare.
pub const FRAMING: &str = "romacs-record 1\n\
header: seq u64le, class u16le, kind u16le, len u32le\n\
fields: integers le; bytes and str behind a u32le length\n\
hello: class 0 kind 0; magic[8], framing u64le, epoch u64le, pid u32le, count u16le, \
count of (id u16le, min u16le, max u16le, name str)\n\
span: offset u64le, len u64le\n";

/// FNV-1a over [`FRAMING`].
pub const FRAMING_HASH: u64 = fnv1a(FRAMING.as_bytes());

const fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut index = 0;
    while index < bytes.len() {
        hash = (hash ^ bytes[index] as u64).wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

/// One class as one side speaks it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassRange {
    pub id: u16,
    pub name: String,
    pub min: u16,
    pub max: u16,
}

impl ClassRange {
    #[must_use]
    pub fn new(id: u16, name: &str, versions: std::ops::RangeInclusive<u16>) -> Self {
        Self { id, name: name.into(), min: *versions.start(), max: *versions.end() }
    }
}

/// What a sender says of itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    /// The incarnation of the sender's handle allocator, or 0 when it
    /// allocates none. Handles of another epoch name nothing.
    pub epoch: u64,
    pub pid: u32,
    pub classes: Vec<ClassRange>,
}

impl Hello {
    /// The hello as the record to send first.
    ///
    /// # Panics
    /// When it lists 65536 classes or more.
    #[must_use]
    pub fn record(&self) -> Record {
        let mut writer = Writer(MAGIC.to_vec()).u64(FRAMING_HASH).u64(self.epoch).u32(self.pid)
            .u16(u16::try_from(self.classes.len()).expect("under 65536 classes"));
        for class in &self.classes {
            writer = writer.u16(class.id).u16(class.min).u16(class.max).str(&class.name);
        }
        Record { seq: 0, class: HELLO.0, kind: HELLO.1, payload: writer.finish() }
    }

    /// Read the peer's first record.
    ///
    /// # Errors
    /// [`Error::NotHello`], [`Error::Framing`] with both hashes, or
    /// [`Error::Truncated`].
    pub fn read(record: &Record) -> Result<Self, Error> {
        let mut reader = Reader(&record.payload);
        if (record.class, record.kind) != HELLO || record.payload.len() < 16 || record.payload[..8] != MAGIC {
            return Err(Error::NotHello);
        }
        reader.0 = &reader.0[8..];
        let theirs = reader.u64()?;
        if theirs != FRAMING_HASH {
            return Err(Error::Framing { ours: FRAMING_HASH, theirs });
        }
        let (epoch, pid, count) = (reader.u64()?, reader.u32()?, reader.u16()?);
        let mut classes = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let (id, min, max) = (reader.u16()?, reader.u16()?, reader.u16()?);
            classes.push(ClassRange { id, min, max, name: reader.str()?.into() });
        }
        Ok(Self { epoch, pid, classes })
    }
}

/// Whether two sides can use a class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Agreement {
    /// Both speak this version, the highest they share.
    Version(u16),
    /// The peer does not list the class.
    Absent { name: String },
    /// Both list it, with no version in common.
    Refused { name: String, ours: (u16, u16), theirs: (u16, u16) },
    /// The peer's class of this id has another name: two different classes.
    Renamed { name: String, theirs: String },
}

impl Agreement {
    /// The agreed version.
    ///
    /// # Errors
    /// Why there is none, naming the class.
    pub fn version(&self) -> Result<u16, String> {
        match self {
            Agreement::Version(version) => Ok(*version),
            Agreement::Absent { name } => Err(format!("the peer has no class \"{name}\"")),
            Agreement::Refused { name, ours, theirs } => Err(format!(
                "class \"{name}\" has no version in common: ours {}-{}, the peer's {}-{}",
                ours.0, ours.1, theirs.0, theirs.1
            )),
            Agreement::Renamed { name, theirs } => Err(format!("class \"{name}\" is \"{theirs}\" to the peer")),
        }
    }
}

/// How OURS fares against the classes a peer listed.
#[must_use]
pub fn agree(ours: &ClassRange, theirs: &[ClassRange]) -> Agreement {
    let name = ours.name.clone();
    let Some(peer) = theirs.iter().find(|class| class.id == ours.id) else {
        return Agreement::Absent { name };
    };
    if peer.name != ours.name {
        return Agreement::Renamed { name, theirs: peer.name.clone() };
    }
    let (low, high) = (ours.min.max(peer.min), ours.max.min(peer.max));
    if low > high {
        return Agreement::Refused { name, ours: (ours.min, ours.max), theirs: (peer.min, peer.max) };
    }
    Agreement::Version(high)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello() -> Hello {
        Hello { epoch: 7, pid: 42, classes: vec![ClassRange::new(0, "core", 1..=1), ClassRange::new(3, "terminal", 2..=5)] }
    }

    #[test]
    fn a_hello_reads_back() {
        assert_eq!(Hello::read(&hello().record()), Ok(hello()));
    }

    /// The head no build may move: a peer of any age must find the magic
    /// and the hash where it looks.
    #[test]
    fn the_head_is_frozen() {
        let mut wire = Vec::new();
        hello().record().encode_into(&mut wire);
        assert_eq!(&wire[8..12], [0, 0, 0, 0], "class 0, kind 0");
        assert_eq!(&wire[16..24], b"ROMACSR\n");
        assert_eq!(wire[24..32], FRAMING_HASH.to_le_bytes());
    }

    /// Editing the layout's description is a deliberate, incompatible act.
    #[test]
    fn the_framing_hash_is_pinned() {
        assert_eq!(FRAMING_HASH, fnv1a(FRAMING.as_bytes()));
        assert_eq!(format!("{FRAMING_HASH:016x}"), PINNED, "the wire layout changed: every older peer will refuse this one");
    }
    const PINNED: &str = "f7089680e3aaab94";

    #[test]
    fn another_framing_is_refused_with_both_hashes() {
        let mut record = hello().record();
        record.payload[8] ^= 1;
        let error = Hello::read(&record).unwrap_err();
        assert_eq!(error, Error::Framing { ours: FRAMING_HASH, theirs: FRAMING_HASH ^ 1 });
        assert!(error.to_string().contains(&format!("{:016x}", FRAMING_HASH ^ 1)), "{error}");
    }

    #[test]
    fn what_is_not_a_hello_is_refused() {
        let mut record = hello().record();
        record.kind = 1;
        assert_eq!(Hello::read(&record), Err(Error::NotHello));
        let record = Record { seq: 0, class: 0, kind: 0, payload: b"GET / HTTP/1.1\r\n\r\n".to_vec() };
        assert_eq!(Hello::read(&record), Err(Error::NotHello));
        let mut record = hello().record();
        record.payload.truncate(20);
        assert_eq!(Hello::read(&record), Err(Error::Truncated));
    }

    #[test]
    fn overlapping_ranges_agree_on_the_highest_shared_version() {
        let ours = ClassRange::new(3, "terminal", 2..=5);
        let peer = |versions| [ClassRange::new(3, "terminal", versions)];
        assert_eq!(agree(&ours, &peer(1..=3)), Agreement::Version(3));
        assert_eq!(agree(&ours, &peer(5..=9)), Agreement::Version(5));
        assert_eq!(agree(&ours, &peer(3..=4)), Agreement::Version(4));
        assert_eq!(agree(&ours, &peer(1..=9)), Agreement::Version(5));
    }

    #[test]
    fn a_class_without_overlap_is_refused_by_name() {
        let ours = ClassRange::new(3, "terminal", 2..=5);
        let refused = agree(&ours, &[ClassRange::new(3, "terminal", 6..=7)]);
        assert_eq!(refused.version(), Err("class \"terminal\" has no version in common: ours 2-5, the peer's 6-7".into()));
        assert_eq!(agree(&ours, &[ClassRange::new(4, "pipe", 1..=1)]).version(), Err("the peer has no class \"terminal\"".into()));
        assert_eq!(agree(&ours, &[ClassRange::new(3, "pipe", 2..=5)]).version(), Err("class \"terminal\" is \"pipe\" to the peer".into()));
    }
}
