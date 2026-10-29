//! The world arena: the global allocator of a binary that boots from world
//! images. Every heap allocation comes from one fixed range of address
//! space, and the allocator's own state lives at the start of that range,
//! so a world image is the range's used prefix written to a file, and
//! restore maps that file back at the same address, copy-on-write, at the
//! process's first allocation. Nothing is decoded and nothing is relocated:
//! every pointer in the image is valid again where it was.
//!
//! Code addresses in the image stay valid only when the executable and its
//! libraries load where they loaded in the builder. World processes run
//! without address randomization (`world::image`), and an image records the
//! executable mappings of the process that made it (`layout`); a process
//! whose mappings differ does not map it.
//!
//! The allocator has size classes up to 32 KiB, carved from page slabs and
//! kept on free lists, and page runs above that, kept in bins by size. One
//! spin lock serializes it. Outside world binaries nothing installs it; when
//! its range cannot be reserved it hands every request to the system
//! allocator.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

/// Where the arena starts: far from the executable, the heap, the stack and
/// the libraries, with or without randomization.
pub const BASE: usize = 0x2000_0000_0000;
/// The reserved range. Pages are committed only as they are touched.
const RESERVE: usize = 1 << 37;
const PAGE: usize = 4096;
const MAGIC: u64 = u64::from_le_bytes(*b"RWARENA1");
/// 16..=1024 by 16, then four steps per doubling to 32 KiB.
const CLASSES: usize = 64 + 20;
const SMALL_MAX: usize = 32768;
const BINS: usize = 40;
/// Freed page runs this long give their pages back.
const RELEASE_PAGES: usize = 64;

/// The allocator's state, at `BASE`. An image carries it, and the fields a
/// restore needs.
#[repr(C)]
struct Header {
    magic: u64,
    /// The first page never handed out as a run.
    top: usize,
    /// Every address at or above this one is untouched, so zero.
    zero: usize,
    free: [usize; CLASSES],
    cursor: [usize; CLASSES],
    end: [usize; CLASSES],
    runs: [usize; BINS],
    /// Set when an image is written.
    image: ImageFields,
}

/// What an image records beside the allocator's state.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ImageFields {
    /// The image's length: the used prefix of the arena.
    pub len: usize,
    /// The address of the boxed root the image was made for.
    pub root: usize,
    /// Process-global counters the image's contents depend on.
    pub counters: [u64; 4],
    /// The digest of the executable mappings of the process that made it.
    pub layout: [u8; 32],
    /// The world's key.
    pub key: [u8; 32],
}

/// The state of the process's arena.
const UNSET: u8 = 0;
const ARENA: u8 = 1;
const SYSTEM: u8 = 2;
static STATE: AtomicU8 = AtomicU8::new(UNSET);
static LOCK: AtomicBool = AtomicBool::new(false);
/// How the image this process asked for fared (`Restore`).
static RESTORE: AtomicU8 = AtomicU8::new(Restore::None as u8);
/// The layout digest, taken as the arena started.
static LAYOUT: AtomicUsize = AtomicUsize::new(0);
static mut LAYOUT_BYTES: [u8; 32] = [0; 32];

/// What became of the image named by `ROMACS_WORLD_IMAGE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Restore {
    /// None was named.
    None = 0,
    /// It is mapped: the arena is the image.
    Mapped,
    /// It could not be opened or read.
    Unreadable,
    /// It is no arena image of this format.
    Format,
    /// The process's executable mappings differ from the builder's.
    Layout,
    /// Its range could not be mapped.
    Map,
}

pub struct Arena;

fn header() -> *mut Header {
    BASE as *mut Header
}

fn lock() {
    let mut spins = 0u32;
    while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        spins += 1;
        if spins < 64 { std::hint::spin_loop() } else { unsafe { libc::sched_yield(); } }
    }
}

fn unlock() {
    LOCK.store(false, Ordering::Release);
}

/// The class of a request of SIZE bytes aligned to ALIGN, or None for a run.
fn class_of(size: usize, align: usize) -> Option<usize> {
    if align > PAGE {
        return None;
    }
    let size = size.max(1).next_multiple_of(align.max(16));
    let mut class = if size <= 1024 {
        size / 16 - 1
    } else if size <= SMALL_MAX {
        let e = (usize::BITS - 1 - (size - 1).leading_zeros()) as usize;
        let step = 1 << (e - 2);
        64 + (e - 10) * 4 + (size - (1 << e)).div_ceil(step) - 1
    } else {
        return None;
    };
    while class < CLASSES && lowest_bit(class_size(class)) < align {
        class += 1;
    }
    (class < CLASSES).then_some(class)
}

fn class_size(class: usize) -> usize {
    if class < 64 {
        (class + 1) * 16
    } else {
        let (e, k) = (10 + (class - 64) / 4, (class - 64) % 4);
        (1 << e) + (k + 1) * (1 << (e - 2))
    }
}

fn lowest_bit(n: usize) -> usize {
    1 << n.trailing_zeros()
}

fn pages(size: usize) -> usize {
    size.max(1).div_ceil(PAGE)
}

fn bin(pages: usize) -> usize {
    ((usize::BITS - 1 - pages.leading_zeros()) as usize).min(BINS - 1)
}

/// A free run's head: its length in pages and the next run of its bin.
#[repr(C)]
struct Run {
    pages: usize,
    next: usize,
}

unsafe fn push_run(h: &mut Header, at: usize, pages: usize) {
    let run = at as *mut Run;
    unsafe { *run = Run { pages, next: h.runs[bin(pages)] } };
    h.runs[bin(pages)] = at;
}

/// PAGES pages aligned to ALIGN (a multiple of the page), from a free run
/// or the top. Null when the range is spent.
unsafe fn take_pages(h: &mut Header, pages: usize, align: usize) -> usize {
    if align <= PAGE {
        for b in bin(pages)..BINS {
            let mut link: *mut usize = &mut h.runs[b];
            let mut walked = 0;
            while unsafe { *link } != 0 && walked < 32 {
                let at = unsafe { *link };
                let run = unsafe { &mut *(at as *mut Run) };
                if run.pages >= pages {
                    unsafe { *link = run.next };
                    let rest = run.pages - pages;
                    if rest > 0 {
                        unsafe { push_run(h, at + pages * PAGE, rest) };
                    }
                    return at;
                }
                link = &mut run.next;
                walked += 1;
            }
        }
    }
    let at = h.top.next_multiple_of(align.max(PAGE));
    let end = at + pages * PAGE;
    if end > BASE + RESERVE {
        return 0;
    }
    if at > h.top {
        unsafe { push_run(h, h.top, (at - h.top) / PAGE) };
    }
    h.top = end;
    at
}

unsafe fn give_pages(h: &mut Header, at: usize, pages: usize) {
    if at + pages * PAGE == h.top {
        h.top = at;
        return;
    }
    if pages >= RELEASE_PAGES {
        unsafe { libc::madvise((at + PAGE) as *mut libc::c_void, (pages - 1) * PAGE, libc::MADV_DONTNEED) };
    }
    unsafe { push_run(h, at, pages) };
}

unsafe fn take_small(h: &mut Header, class: usize) -> usize {
    let head = h.free[class];
    if head != 0 {
        h.free[class] = unsafe { *(head as *const usize) };
        return head;
    }
    let size = class_size(class);
    if h.cursor[class] + size > h.end[class] {
        let slab = (64 * 1024).max(4 * size);
        let at = unsafe { take_pages(h, pages(slab), PAGE) };
        if at == 0 {
            return 0;
        }
        h.cursor[class] = at;
        h.end[class] = at + pages(slab) * PAGE;
    }
    let at = h.cursor[class];
    h.cursor[class] += size;
    at
}

unsafe fn give_small(h: &mut Header, at: usize, class: usize) {
    unsafe { *(at as *mut usize) = h.free[class] };
    h.free[class] = at;
}

fn in_arena(at: *mut u8) -> bool {
    (BASE..BASE + RESERVE).contains(&(at as usize))
}

fn state() -> u8 {
    match STATE.load(Ordering::Acquire) {
        UNSET => {
            lock();
            if STATE.load(Ordering::Relaxed) == UNSET {
                let state = unsafe { start() };
                STATE.store(state, Ordering::Release);
            }
            unlock();
            STATE.load(Ordering::Acquire)
        }
        state => state,
    }
}

/// Map the image `ROMACS_WORLD_IMAGE` names, or reserve a fresh range.
unsafe fn start() -> u8 {
    let layout = unsafe { layout_digest() };
    unsafe { LAYOUT_BYTES = layout };
    LAYOUT.store(1, Ordering::Release);
    let wanted = unsafe { libc::getenv(c"ROMACS_WORLD_IMAGE".as_ptr()) };
    if !wanted.is_null() {
        let outcome = unsafe { map_image(wanted, &layout) };
        RESTORE.store(outcome as u8, Ordering::Release);
        if outcome == Restore::Mapped {
            return ARENA;
        }
    }
    let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE | libc::MAP_FIXED_NOREPLACE;
    let at = unsafe { libc::mmap(BASE as *mut libc::c_void, RESERVE, libc::PROT_READ | libc::PROT_WRITE, flags, -1, 0) };
    if at != BASE as *mut libc::c_void {
        if at != libc::MAP_FAILED {
            unsafe { libc::munmap(at, RESERVE) };
        }
        return SYSTEM;
    }
    let h = unsafe { &mut *header() };
    h.magic = MAGIC;
    h.top = BASE + size_of::<Header>().next_multiple_of(PAGE);
    h.zero = h.top;
    ARENA
}

unsafe fn map_image(path: *const libc::c_char, layout: &[u8; 32]) -> Restore {
    let fd = unsafe { libc::open(path, libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Restore::Unreadable;
    }
    let outcome = unsafe { map_image_fd(fd, layout) };
    unsafe { libc::close(fd) };
    outcome
}

unsafe fn map_image_fd(fd: i32, layout: &[u8; 32]) -> Restore {
    let mut bytes = [0u8; size_of::<Header>()];
    let read = unsafe { libc::pread(fd, bytes.as_mut_ptr().cast(), bytes.len(), 0) };
    if read != bytes.len() as isize {
        return Restore::Unreadable;
    }
    let image = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<Header>()) };
    let len = image.image.len;
    if image.magic != MAGIC || len == 0 || len % PAGE != 0 || len >= RESERVE || image.top > BASE + len {
        return Restore::Format;
    }
    if image.image.layout != *layout {
        return Restore::Layout;
    }
    let file = libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE;
    let at = unsafe { libc::mmap(BASE as *mut libc::c_void, len, libc::PROT_READ | libc::PROT_WRITE, file, fd, 0) };
    if at != BASE as *mut libc::c_void {
        return Restore::Map;
    }
    let anon = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE | libc::MAP_FIXED_NOREPLACE;
    let rest = unsafe { libc::mmap((BASE + len) as *mut libc::c_void, RESERVE - len, libc::PROT_READ | libc::PROT_WRITE, anon, -1, 0) };
    if rest != (BASE + len) as *mut libc::c_void {
        unsafe { libc::munmap(BASE as *mut libc::c_void, len) };
        return Restore::Map;
    }
    // The kernel reads the file ahead while the process starts.
    unsafe { libc::madvise(BASE as *mut libc::c_void, len, libc::MADV_WILLNEED) };
    Restore::Mapped
}

/// The SHA-256 of the process's executable mappings, as /proc/self/maps
/// lists them: where the executable, the libraries and the vDSO are.
/// Read without allocating, since the allocator is starting.
unsafe fn layout_digest() -> [u8; 32] {
    use sha2::Digest;
    static mut MAPS: [u8; 65536] = [0; 65536];
    let buffer = unsafe { &mut *std::ptr::addr_of_mut!(MAPS) };
    let fd = unsafe { libc::open(c"/proc/self/maps".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    let mut len = 0;
    if fd >= 0 {
        while len < buffer.len() {
            let n = unsafe { libc::read(fd, buffer[len..].as_mut_ptr().cast(), buffer.len() - len) };
            if n <= 0 { break; }
            len += n as usize;
        }
        unsafe { libc::close(fd) };
    }
    let mut digest = sha2::Sha256::new();
    for line in buffer[..len].split(|&b| b == b'\n') {
        // start-end perms offset dev inode path
        if line.split(|&b| b == b' ').nth(1).is_some_and(|perms| perms.get(2) == Some(&b'x')) {
            digest.update(line);
            digest.update(b"\n");
        }
    }
    digest.finalize().into()
}

/// How the image this process asked for fared.
pub fn restore_outcome() -> Restore {
    state();
    match RESTORE.load(Ordering::Acquire) {
        1 => Restore::Mapped,
        2 => Restore::Unreadable,
        3 => Restore::Format,
        4 => Restore::Layout,
        5 => Restore::Map,
        _ => Restore::None,
    }
}

/// Whether this process's heap is the arena.
pub fn active() -> bool {
    state() == ARENA
}

/// The fields of the image this process mapped.
pub fn mapped_image() -> Option<ImageFields> {
    (restore_outcome() == Restore::Mapped).then(|| unsafe { (*header()).image })
}

/// The digest of this process's executable mappings.
pub fn layout() -> [u8; 32] {
    state();
    unsafe { LAYOUT_BYTES }
}

/// Write the arena to FILE as an image with FIELDS (its length is set
/// here), leaving holes for untouched and zero pages. The caller allocates
/// nothing meanwhile on any thread: the allocator is locked throughout.
///
/// # Safety
/// FIELDS.root must be the address of a live allocation the image's reader
/// takes ownership of.
pub unsafe fn write_image(file: &std::fs::File, mut fields: ImageFields) -> std::io::Result<usize> {
    use std::os::fd::AsRawFd;
    if !active() {
        return Err(std::io::Error::other("the arena is not this process's heap"));
    }
    lock();
    let h = unsafe { &mut *header() };
    fields.len = h.top - BASE;
    let saved = h.image;
    h.image = fields;
    let result = unsafe { write_pages(file.as_raw_fd(), fields.len) };
    h.image = saved;
    unlock();
    result.map(|()| fields.len)
}

unsafe fn write_pages(fd: i32, len: usize) -> std::io::Result<()> {
    let zero = unsafe { (*header()).zero }.min(BASE + len);
    let mut at = BASE;
    while at < BASE + len {
        // Untouched and all-zero pages stay holes.
        if at >= zero || unsafe { std::slice::from_raw_parts(at as *const u8, PAGE) }.iter().all(|&b| b == 0) {
            at += PAGE;
            continue;
        }
        let mut end = at + PAGE;
        while end < zero.min(BASE + len) && end - at < (1 << 24)
            && unsafe { std::slice::from_raw_parts(end as *const u8, PAGE) }.iter().any(|&b| b != 0)
        {
            end += PAGE;
        }
        let mut done = 0;
        while at + done < end {
            let n = unsafe { libc::pwrite(fd, (at + done) as *const libc::c_void, end - at - done, (at + done - BASE) as i64) };
            if n < 0 {
                return Err(std::io::Error::last_os_error());
            }
            done += n as usize;
        }
        at = end;
    }
    if unsafe { libc::ftruncate(fd, len as i64) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

unsafe impl GlobalAlloc for Arena {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if state() != ARENA {
            return unsafe { System.alloc(layout) };
        }
        lock();
        let h = unsafe { &mut *header() };
        let at = match class_of(layout.size(), layout.align()) {
            Some(class) => unsafe { take_small(h, class) },
            None => unsafe { take_pages(h, pages(layout.size()), layout.align().max(PAGE)) },
        };
        h.zero = h.zero.max(h.top);
        unlock();
        at as *mut u8
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if state() != ARENA {
            return unsafe { System.alloc_zeroed(layout) };
        }
        lock();
        let h = unsafe { &mut *header() };
        let zero = h.zero;
        let at = match class_of(layout.size(), layout.align()) {
            Some(class) => unsafe { take_small(h, class) },
            None => unsafe { take_pages(h, pages(layout.size()), layout.align().max(PAGE)) },
        };
        h.zero = h.zero.max(h.top);
        unlock();
        // Memory at or above the old zero mark was never handed out.
        if at != 0 && at < zero {
            unsafe { std::ptr::write_bytes(at as *mut u8, 0, layout.size().min(zero - at)) };
        }
        at as *mut u8
    }

    unsafe fn dealloc(&self, at: *mut u8, layout: Layout) {
        if !in_arena(at) {
            return unsafe { System.dealloc(at, layout) };
        }
        lock();
        let h = unsafe { &mut *header() };
        match class_of(layout.size(), layout.align()) {
            Some(class) => unsafe { give_small(h, at as usize, class) },
            None => unsafe { give_pages(h, at as usize, pages(layout.size())) },
        }
        unlock();
    }

    unsafe fn realloc(&self, at: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if !in_arena(at) {
            return unsafe { System.realloc(at, layout, new_size) };
        }
        let align = layout.align();
        match (class_of(layout.size(), align), class_of(new_size, align)) {
            (Some(old), Some(new)) if old == new => return at,
            (None, None) if align <= PAGE => {
                let (old, new) = (pages(layout.size()), pages(new_size));
                lock();
                let h = unsafe { &mut *header() };
                let start = at as usize;
                let grown = if new <= old {
                    if new < old {
                        unsafe { give_pages(h, start + new * PAGE, old - new) };
                    }
                    true
                } else if start + old * PAGE == h.top && h.top + (new - old) * PAGE <= BASE + RESERVE {
                    h.top += (new - old) * PAGE;
                    h.zero = h.zero.max(h.top);
                    true
                } else {
                    false
                };
                unlock();
                if grown {
                    return at;
                }
            }
            _ => {}
        }
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, align) };
        let moved = unsafe { self.alloc(new_layout) };
        if !moved.is_null() {
            unsafe { std::ptr::copy_nonoverlapping(at, moved, layout.size().min(new_size)) };
            unsafe { self.dealloc(at, layout) };
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_cover_every_small_size_and_alignment() {
        for size in 1..=SMALL_MAX {
            for align in [1, 8, 16, 32, 64, 256, 4096] {
                if let Some(class) = class_of(size, align) {
                    let block = class_size(class);
                    assert!(block >= size && block % align == 0, "{size} {align} -> {block}");
                    if class > 0 && align <= 16 {
                        assert!(class_size(class - 1) < size.next_multiple_of(16), "{size}: {block} is not the least");
                    }
                }
            }
        }
        assert_eq!(class_of(SMALL_MAX + 1, 8), None);
        assert_eq!(class_of(64, 8192), None);
        assert_eq!((class_size(63), class_size(64), class_size(CLASSES - 1)), (1024, 1280, SMALL_MAX));
    }

    #[test]
    fn bins_order_runs_by_length() {
        assert_eq!((bin(1), bin(2), bin(3), bin(4), bin(1 << 50)), (0, 1, 1, 2, BINS - 1));
    }
}
