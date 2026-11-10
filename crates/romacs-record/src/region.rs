//! Large payloads by shared memory.
//!
//! The writer makes one memfd, sealed against resizing, and passes its
//! descriptor once ([`crate::channel`]). After that a payload crosses as a
//! [`Span`]: the writer copies it into the region and sends the span in a
//! record; the reader copies it out and acknowledges the span, which frees
//! it. The writer never reads the region, so nothing a reader does to the
//! mapping can mislead it. A reader copies a payload out before it looks at
//! it, and keeps no pointer into the region.
//!
//! Spans are handed out in a ring. One that is not acknowledged holds back
//! the space behind it, so a reader acknowledges in order of arrival, soon.
//! When the region is full [`RegionWriter::put`] returns `None` and the
//! caller sends the payload inline or waits: nothing is overwritten.

use std::collections::VecDeque;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use crate::{Error, Reader, Writer};

/// A payload's place in a region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub offset: u64,
    pub len: u64,
}

impl Span {
    #[must_use]
    pub fn write(self, writer: Writer) -> Writer { writer.u64(self.offset).u64(self.len) }

    /// # Errors
    /// [`Error::Truncated`].
    pub fn read(reader: &mut Reader<'_>) -> Result<Self, Error> { Ok(Self { offset: reader.u64()?, len: reader.u64()? }) }
}

const ALIGN: u64 = 8;
const SEALS: libc::c_int = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;

/// Which bytes of a region of CAPACITY are in use: spans allocated in a
/// ring and freed in any order. Pure bookkeeping, no memory.
struct Ring {
    capacity: u64,
    /// Where the next span starts.
    head: u64,
    /// Spans from oldest to newest, with whether each was released.
    live: VecDeque<(Span, bool)>,
}

impl Ring {
    fn allocate(&mut self, len: u64) -> Option<Span> {
        let size = len.checked_next_multiple_of(ALIGN)?.max(ALIGN);
        let Some((oldest, _)) = self.live.front() else {
            self.head = 0;
            return (size <= self.capacity).then(|| self.take(len, size));
        };
        let tail = oldest.offset;
        if self.head > tail {
            // In use: tail up to head. Free: head to the end, then 0 to the tail.
            if self.capacity - self.head >= size {
                return Some(self.take(len, size));
            }
            if tail >= size {
                self.head = 0;
                return Some(self.take(len, size));
            }
            return None;
        }
        // Wrapped: free from head to the tail. Head at the tail is full,
        // since an empty ring starts again from 0.
        (tail - self.head >= size).then(|| self.take(len, size))
    }

    fn take(&mut self, len: u64, size: u64) -> Span {
        let span = Span { offset: self.head, len };
        self.head += size;
        self.live.push_back((span, false));
        span
    }

    fn release(&mut self, span: Span) -> Result<(), Error> {
        let entry = self.live.iter_mut().find(|(live, released)| *live == span && !released).ok_or(Error::Span(span))?;
        entry.1 = true;
        while self.live.front().is_some_and(|(_, released)| *released) {
            self.live.pop_front();
        }
        Ok(())
    }
}

/// A shared mapping, unmapped on drop.
struct Mapping {
    base: *mut u8,
    len: usize,
}

impl Mapping {
    fn new(fd: BorrowedFd<'_>, len: usize, protection: libc::c_int) -> io::Result<Self> {
        // SAFETY: a fresh shared mapping of LEN bytes of a descriptor the
        // caller holds, at an address the kernel picks; no existing memory
        // is touched. The file is sealed against shrinking, so every page
        // stays backed for the mapping's life.
        let base = unsafe { libc::mmap(std::ptr::null_mut(), len, protection, libc::MAP_SHARED, fd.as_raw_fd(), 0) };
        if base == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { base: base.cast(), len })
    }

    /// SPAN's offset and length when it lies inside the mapping.
    fn place(&self, span: Span) -> Option<(usize, usize)> {
        let (offset, len) = (usize::try_from(span.offset).ok()?, usize::try_from(span.len).ok()?);
        (offset.checked_add(len)? <= self.len).then_some((offset, len))
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: BASE and LEN are exactly what mmap returned for this
        // mapping, and no reference into it exists: the region's methods
        // copy bytes in and out and return none.
        unsafe { libc::munmap(self.base.cast(), self.len) };
    }
}

fn seals(fd: BorrowedFd<'_>) -> io::Result<libc::c_int> {
    // SAFETY: F_GET_SEALS reads flags of a descriptor the caller holds and
    // takes no pointer.
    let seals = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS) };
    if seals < 0 { Err(io::Error::last_os_error()) } else { Ok(seals) }
}

/// The side that makes a region and puts payloads in it.
pub struct RegionWriter {
    fd: OwnedFd,
    map: Mapping,
    ring: Ring,
}

impl RegionWriter {
    /// A region of CAPACITY bytes, zeroed, that can never be resized.
    ///
    /// # Errors
    /// A capacity that is zero or not a multiple of 8, or the system's
    /// refusal of the memfd, its seals or its mapping.
    pub fn create(capacity: usize) -> io::Result<Self> {
        if capacity == 0 || !(capacity as u64).is_multiple_of(ALIGN) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "region capacity must be a positive multiple of 8"));
        }
        // SAFETY: the name is a NUL-terminated literal that memfd_create
        // only reads; the flags take no pointer.
        let fd = unsafe { libc::memfd_create(c"romacs-region".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: memfd_create just returned FD, open and owned by no one else.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        std::fs::File::from(fd.try_clone()?).set_len(capacity as u64)?;
        // SAFETY: F_ADD_SEALS takes this descriptor and an integer.
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_ADD_SEALS, SEALS) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let map = Mapping::new(fd.as_fd(), capacity, libc::PROT_READ | libc::PROT_WRITE)?;
        Ok(Self { fd, map, ring: Ring { capacity: capacity as u64, head: 0, live: VecDeque::new() } })
    }

    /// The descriptor to pass to the reader, once.
    #[must_use]
    pub fn fd(&self) -> BorrowedFd<'_> { self.fd.as_fd() }

    /// Copy BYTES into the region; `None` when it has no room for them.
    pub fn put(&mut self, bytes: &[u8]) -> Option<Span> {
        let span = self.ring.allocate(bytes.len() as u64)?;
        let (offset, len) = self.map.place(span)?;
        // SAFETY: the ring gives out spans inside the mapping's capacity
        // that overlap no live span, so the destination is LEN mapped,
        // writable bytes; BYTES is ordinary memory and cannot overlap a
        // shared mapping this struct owns alone in this process. No
        // reference into the mapping is formed.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.map.base.add(offset), len) };
        Some(span)
    }

    /// The reader acknowledged SPAN: its bytes may be reused.
    ///
    /// # Errors
    /// [`Error::Span`] for a span that is not live: the reader's error.
    pub fn release(&mut self, span: Span) -> Result<(), Error> { self.ring.release(span) }

    /// Forget every span: the reader is gone and a new one starts clean.
    pub fn reset(&mut self) {
        self.ring.live.clear();
        self.ring.head = 0;
    }

    /// Spans put and not yet released.
    #[must_use]
    pub fn live(&self) -> usize { self.ring.live.iter().filter(|(_, released)| !released).count() }
}

/// The side that was passed a region's descriptor.
pub struct RegionReader {
    map: Mapping,
}

impl RegionReader {
    /// Map the region FD names, read-only. A descriptor that could still be
    /// resized is refused: reading past a shrunken file would kill the reader.
    /// The mapping outlives FD, which the caller may close.
    ///
    /// # Errors
    /// `PermissionDenied` for an unsealed descriptor, `InvalidData` for an
    /// empty one, or the system's refusal to map it.
    pub fn open(fd: BorrowedFd<'_>) -> io::Result<Self> {
        let needed = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW;
        if seals(fd).map_or(true, |seals| seals & needed != needed) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "region descriptor is not sealed against resizing"));
        }
        let len = usize::try_from(std::fs::File::from(fd.try_clone_to_owned()?).metadata()?.len()).map_err(io::Error::other)?;
        if len == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "region is empty"));
        }
        Ok(Self { map: Mapping::new(fd, len, libc::PROT_READ)? })
    }

    /// The bytes of SPAN, copied out. The caller then acknowledges SPAN to
    /// the writer and validates the copy; the region may change after that.
    ///
    /// # Errors
    /// [`Error::Span`] for a span outside the region.
    pub fn copy(&self, span: Span) -> Result<Vec<u8>, Error> {
        let (offset, len) = self.map.place(span).ok_or(Error::Span(span))?;
        let mut bytes = Vec::<u8>::with_capacity(len);
        // SAFETY: SPAN was checked to lie inside the mapping, which stays
        // mapped and backed (the file cannot shrink), and the destination
        // is a fresh allocation of that capacity. The source is read
        // through a raw pointer, never a reference, since another process
        // may write it; whatever bytes arrive are plain u8.
        unsafe {
            std::ptr::copy_nonoverlapping(self.map.base.add(offset), bytes.as_mut_ptr(), len);
            bytes.set_len(len);
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(capacity: u64) -> Ring { Ring { capacity, head: 0, live: VecDeque::new() } }

    #[test]
    fn a_payload_crosses_by_span() {
        let mut writer = RegionWriter::create(4096).unwrap();
        let reader = RegionReader::open(writer.fd()).unwrap();
        let first = writer.put(b"hello").unwrap();
        let second = writer.put(&[0xab; 1000]).unwrap();
        assert_eq!(reader.copy(first).unwrap(), b"hello");
        assert_eq!(reader.copy(second).unwrap(), [0xab; 1000]);
        assert_eq!(writer.live(), 2);
        writer.release(first).unwrap();
        writer.release(second).unwrap();
        assert_eq!(writer.live(), 0);
    }

    #[test]
    fn a_span_outside_the_region_is_refused() {
        let writer = RegionWriter::create(4096).unwrap();
        let reader = RegionReader::open(writer.fd()).unwrap();
        for span in [Span { offset: 4096, len: 1 }, Span { offset: 0, len: 4097 }, Span { offset: u64::MAX, len: 2 }] {
            assert_eq!(reader.copy(span), Err(Error::Span(span)));
        }
        assert_eq!(reader.copy(Span { offset: 4096, len: 0 }), Ok(vec![]));
    }

    #[test]
    fn a_region_cannot_be_resized_by_either_side() {
        let writer = RegionWriter::create(4096).unwrap();
        let file = std::fs::File::from(writer.fd().try_clone_to_owned().unwrap());
        assert!(file.set_len(0).is_err());
        assert!(file.set_len(8192).is_err());
    }

    #[test]
    fn an_unsealed_descriptor_is_refused() {
        // SAFETY: a literal name; the result is checked and owned below.
        let fd = unsafe { libc::memfd_create(c"unsealed".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
        assert!(fd >= 0);
        // SAFETY: FD was just returned open and is owned here alone.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        std::fs::File::from(fd.try_clone().unwrap()).set_len(4096).unwrap();
        assert_eq!(RegionReader::open(fd.as_fd()).err().map(|e| e.kind()), Some(io::ErrorKind::PermissionDenied));
    }

    #[test]
    fn a_full_region_refuses_and_overwrites_nothing() {
        let mut writer = RegionWriter::create(64).unwrap();
        let reader = RegionReader::open(writer.fd()).unwrap();
        let first = writer.put(&[1; 40]).unwrap();
        assert_eq!(writer.put(&[2; 40]), None);
        assert_eq!(reader.copy(first).unwrap(), [1; 40]);
        writer.release(first).unwrap();
        let second = writer.put(&[2; 40]).unwrap();
        assert_eq!(reader.copy(second).unwrap(), [2; 40]);
        assert_eq!(writer.put(&[0; 65]), None, "larger than the region");
    }

    #[test]
    fn a_release_of_what_is_not_live_is_an_error() {
        let mut writer = RegionWriter::create(64).unwrap();
        let span = writer.put(b"x").unwrap();
        assert_eq!(writer.release(Span { offset: 8, len: 1 }), Err(Error::Span(Span { offset: 8, len: 1 })));
        writer.release(span).unwrap();
        assert_eq!(writer.release(span), Err(Error::Span(span)));
    }

    /// Spans allocated in a ring never overlap, whatever order they are
    /// released in, and space comes back.
    #[test]
    fn live_spans_never_overlap() {
        let mut ring = ring(256);
        let mut live: Vec<Span> = Vec::new();
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || { state ^= state << 13; state ^= state >> 7; state ^= state << 17; state };
        let mut allocated = 0;
        for _ in 0..20_000 {
            if next() % 3 != 0 {
                if let Some(span) = ring.allocate(next() % 70) {
                    let end = span.offset + span.len.next_multiple_of(ALIGN).max(ALIGN);
                    assert!(end <= 256, "{span:?}");
                    for other in &live {
                        let other_end = other.offset + other.len.next_multiple_of(ALIGN).max(ALIGN);
                        assert!(end <= other.offset || other_end <= span.offset, "{span:?} overlaps {other:?}");
                    }
                    live.push(span);
                    allocated += 1;
                }
            } else if !live.is_empty() {
                let span = live.swap_remove(usize::try_from(next() % live.len() as u64).unwrap());
                ring.release(span).unwrap();
            }
        }
        assert!(allocated > 5_000, "the ring kept giving space back: {allocated}");
    }

    #[test]
    fn an_emptied_ring_takes_a_span_of_its_whole_capacity() {
        let mut ring = ring(64);
        let a = ring.allocate(24).unwrap();
        let b = ring.allocate(24).unwrap();
        ring.release(b).unwrap();
        ring.release(a).unwrap();
        assert_eq!(ring.allocate(64), Some(Span { offset: 0, len: 64 }));
        assert_eq!(ring.allocate(1), None);
    }
}
