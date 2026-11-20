//! The arena's mappings: the window a process reserves at its first
//! allocation (`reserve_window`), and the fixed ranges this module maps in
//! it and beside it. This is the only code that maps or unmaps memory for
//! the arena, so the rule that keeps it sound is kept here: a range is
//! mapped over what is already there only where that is this process's
//! reservation and no published arena (`place`); anywhere else it is
//! mapped only where nothing is (`MAP_FIXED_NOREPLACE`), which replaces
//! nothing.
use super::{FIRST_ARENA, IMAGE_ARENA, RESERVE};
use rustix::mm::{MapFlags, ProtFlags};
use std::os::fd::BorrowedFd;
use std::sync::atomic::{AtomicU64, Ordering};

/// Where an arena may start: a GiB-aligned address in this window, far from
/// the executable, its heap, the libraries and the stack. Builders' arenas,
/// so images, start in BUILD; a process's first arena starts in FIRST, the
/// window's top eighth. Both are above the window's lower part, where a
/// process with an unlimited stack maps its libraries (bottom-up from
/// `0x1554_0000_0000` or so, a sixth of the address space, plus up to a TiB
/// of randomization; see the module documentation). The window is
/// reserved a CHUNK at a time.
pub(super) const WINDOW: (usize, usize) = (0x1000_0000_0000, 0x5000_0000_0000);
pub(super) const BUILD: (usize, usize) = (0x2800_0000_0000, 0x4800_0000_0000);
pub(super) const FIRST: (usize, usize) = (0x4800_0000_0000, WINDOW.1);
const CHUNK: usize = 1 << 40;
pub(super) const ALIGN: usize = 1 << 30;

/// The chunks of the window this process reserved at its first allocation
/// (`reserve_window`): bit N for the Nth CHUNK from the window's start.
static RESERVED: AtomicU64 = AtomicU64::new(0);

/// Reserve the window from BUILD up, inaccessible, a chunk at a time:
/// nothing the process maps after its first allocation lands in a reserved
/// chunk, and the ranges this module maps there replace the reservation
/// (`place`). A chunk something is mapped in already stays unreserved.
/// A reservation counts against an address-space limit, inaccessible as
/// it is: under one that holds less than the arenas themselves nothing is
/// tried, and the chunks stop where the limit is reached.
pub(super) fn reserve_window() {
    if super::reserve() != Some(RESERVE) {
        return;
    }
    let mut bits = 0u64;
    let mut at = BUILD.0;
    while at < WINDOW.1 {
        match map(&place(at, CHUNK), at, CHUNK, &Of::Nothing) {
            Ok(mapping) => {
                mapping.keep();
                bits |= 1 << ((at - WINDOW.0) / CHUNK);
            }
            Err(Refused { errno, .. }) if errno == libc::ENOMEM => break,
            Err(_) => {}
        }
        at += CHUNK;
    }
    RESERVED.store(bits, Ordering::Relaxed);
}

/// The chunks of the window the range at AT, LEN bytes, spans, as bits of
/// RESERVED; None for a range not within the window.
fn chunks(at: usize, len: usize) -> Option<u64> {
    if !(WINDOW.0..WINDOW.1).contains(&at) || len == 0 || len > WINDOW.1 - at {
        return None;
    }
    let (first, last) = ((at - WINDOW.0) / CHUNK, (at + len - 1 - WINDOW.0) / CHUNK);
    u64::try_from((1u128 << (last + 1)) - (1u128 << first)).ok()
}

/// Whether the range at AT, LEN bytes, is within this process's
/// reservation of the window.
pub(super) fn within_reservation(at: usize, len: usize) -> bool {
    chunks(at, len).is_some_and(|mask| RESERVED.load(Ordering::Relaxed) & mask == mask)
}

/// A GiB-aligned base for a range of RESERVE bytes in the part LOW..HIGH
/// of the window, from RANDOM: in one of the part's reserved chunks when
/// it has any, anywhere in the part otherwise.
pub(super) fn base_in(low: usize, high: usize, reserve: usize, random: usize) -> usize {
    let mask = chunks(low, high - low).unwrap_or(0) & RESERVED.load(Ordering::Relaxed);
    if mask == 0 {
        return low + random % ((high - low - reserve) / ALIGN) * ALIGN;
    }
    let nth = random % mask.count_ones() as usize;
    let chunk = (0..64).filter(|bit| mask & (1 << bit) != 0).nth(nth).unwrap_or(0);
    WINDOW.0 + chunk * CHUNK + (random >> 8) % ((CHUNK - reserve) / ALIGN) * ALIGN
}

/// How the ranges of SPAN are mapped (`place`): over the reservation, or
/// only where nothing is. Made only by `place`, so it holds for its span.
#[derive(Clone, Copy)]
pub(super) struct Place {
    span: (usize, usize),
    over: bool,
}

/// How to map within the span at AT, LEN bytes: over the reservation where
/// the span is within it and reaches no published arena, else only where
/// nothing is mapped.
pub(super) fn place(at: usize, len: usize) -> Place {
    let arena = |(base, reserve): (usize, usize)| at < base + reserve && base < at + len;
    let live = [&FIRST_ARENA, &IMAGE_ARENA].iter().any(|slot| slot.get().is_some_and(arena));
    Place { span: (at, len), over: within_reservation(at, len) && !live }
}

/// What a mapping holds.
pub(super) enum Of<'a> {
    /// Nothing: inaccessible, a reservation.
    Nothing,
    /// Fresh memory, read-write.
    Memory,
    /// FD's bytes from offset 0, read-write: SHARED with the file, or
    /// private copy-on-write.
    File { fd: BorrowedFd<'a>, shared: bool },
}

/// Why a range could not be mapped at its address: the error, 0 when the
/// kernel mapped it elsewhere (which is undone), and whether it was to
/// replace the reservation.
pub(super) struct Refused {
    pub(super) errno: i32,
    pub(super) over_reservation: bool,
}

/// A range this module mapped. It stays mapped once kept; undone, it goes
/// back to the reservation it replaced, or is unmapped.
#[must_use]
pub(super) struct Mapping {
    at: usize,
    len: usize,
    over: bool,
}

impl Mapping {
    /// Keep it mapped for the process's life.
    #[expect(clippy::unused_self, reason = "keeping consumes the mapping, so it cannot be undone after")]
    pub(super) fn keep(self) {}

    /// Ask the kernel to read it ahead.
    pub(super) fn will_need(&self) {
        // UNSAFE(madvise on a mapping has no safe wrapper: rustix's is unsafe)
        // SAFETY: the range is this mapping, which WILLNEED only reads ahead.
        let _ = unsafe { rustix::mm::madvise(self.at as *mut _, self.len, rustix::mm::Advice::WillNeed) };
    }

    /// Give it up: nothing refers to it yet.
    pub(super) fn undo(self) {
        unmap_or_reserve(self.at, self.len, self.over);
    }
}

/// Map LEN bytes of OF at AT, within PLACE's span, as PLACE says.
pub(super) fn map(place: &Place, at: usize, len: usize, of: &Of<'_>) -> Result<Mapping, Refused> {
    let (low, span) = place.span;
    if at < low || len > span || at - low > span - len {
        std::process::abort();
    }
    let over = place.over;
    let fixed = if over { MapFlags::FIXED } else { MapFlags::FIXED_NOREPLACE };
    let rw = ProtFlags::READ | ProtFlags::WRITE;
    // UNSAFE(no crate maps at a fixed address safely: rustix's mmap is unsafe, memmap2 has no MAP_FIXED)
    // SAFETY: with FIXED_NOREPLACE the kernel replaces nothing; FIXED is
    // used only where PLACE found this process's reservation and no
    // published arena (`place`), whose pages nothing reads.
    let mapped = unsafe {
        match of {
            Of::Nothing => rustix::mm::mmap_anonymous(at as *mut _, len, ProtFlags::empty(), MapFlags::PRIVATE | MapFlags::NORESERVE | fixed),
            Of::Memory => rustix::mm::mmap_anonymous(at as *mut _, len, rw, MapFlags::PRIVATE | MapFlags::NORESERVE | fixed),
            &Of::File { fd, shared } => {
                let sharing = if shared { MapFlags::SHARED | MapFlags::NORESERVE } else { MapFlags::PRIVATE };
                rustix::mm::mmap(at as *mut _, len, rw, sharing | fixed, fd, 0)
            }
        }
    };
    match mapped {
        Ok(got) if got as usize == at => Ok(Mapping { at, len, over }),
        Ok(got) => {
            // A kernel that ignores MAP_FIXED_NOREPLACE placed it elsewhere.
            unmap_or_reserve(got as usize, len, false);
            Err(Refused { errno: 0, over_reservation: over })
        }
        Err(errno) => {
            // A failed MAP_FIXED may have unmapped what it was to replace.
            if over {
                unmap_or_reserve(at, len, true);
            }
            Err(Refused { errno: errno.raw_os_error(), over_reservation: over })
        }
    }
}

/// Give up the range at AT, LEN bytes, which this module mapped and
/// nothing refers to: back to the reservation where it replaced it (OVER),
/// else unmapped.
fn unmap_or_reserve(at: usize, len: usize, over: bool) {
    if over {
        let flags = MapFlags::PRIVATE | MapFlags::NORESERVE | MapFlags::FIXED;
        // UNSAFE(no crate maps at a fixed address safely: rustix's mmap is unsafe)
        // SAFETY: the range lies in this process's reservation and was this
        // module's mapping, which nothing refers to; it becomes inaccessible.
        let _ = unsafe { rustix::mm::mmap_anonymous(at as *mut _, len, ProtFlags::empty(), flags) };
    } else {
        // UNSAFE(no crate unmaps a range safely: rustix's munmap is unsafe)
        // SAFETY: the range was this module's mapping, which nothing refers to.
        let _ = unsafe { rustix::mm::munmap(at as *mut _, len) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::PAGE;

    #[test]
    fn a_range_spans_the_chunks_of_the_window_it_lies_in() {
        assert_eq!(chunks(WINDOW.0, CHUNK), Some(1));
        assert_eq!(chunks(WINDOW.0 + CHUNK - PAGE, 2 * PAGE), Some(0b11));
        assert_eq!(chunks(WINDOW.1 - CHUNK, CHUNK), Some(1 << 63));
        assert_eq!(chunks(WINDOW.0, WINDOW.1 - WINDOW.0), Some(u64::MAX));
        assert_eq!(chunks(BUILD.0, RESERVE), Some(1 << ((BUILD.0 - WINDOW.0) / CHUNK)));
        assert_eq!((chunks(WINDOW.0 - PAGE, PAGE), chunks(WINDOW.1 - PAGE, 2 * PAGE), chunks(WINDOW.0, 0)), (None, None, None));
        // At and above the window's end: no chunk, so never reserved.
        assert_eq!((chunks(WINDOW.1, PAGE), chunks(WINDOW.1 + CHUNK, PAGE), chunks(usize::MAX, 1)), (None, None, None));
        assert!(!within_reservation(WINDOW.1 + CHUNK, PAGE) && !within_reservation(WINDOW.0 - PAGE, PAGE));
        // The parts a range is chosen from fit whole chunks, and FIRST is above BUILD.
        assert!(BUILD.0.is_multiple_of(CHUNK) && BUILD.1 == FIRST.0 && FIRST.0.is_multiple_of(CHUNK) && FIRST.1 == WINDOW.1 && RESERVE < CHUNK);
    }
}
