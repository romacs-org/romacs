//! The world arena: the global allocator of a binary that boots from world
//! images. Every heap allocation comes from one fixed range of address
//! space, and the allocator's own state lives at the start of that range,
//! so a world image is the range's used prefix written to a file, and
//! restore maps that file back at the same address, copy-on-write
//! (`adopt`). Nothing is decoded and nothing is relocated: every pointer
//! in the image is valid again where it was. From then on the process
//! allocates from the production allocator (`romacs_allocator`), and the
//! image and its first arena only take back the blocks they hold: only a
//! builder captures an image, so nothing needs a restored process's later
//! blocks in the range, and the production allocator returns what a peak
//! freed (`trim`), where size classes would keep it resident for good. A
//! builder's arena is a
//! shared mapping of an unlinked memory file, and its image is copied from
//! that file by the kernel, so capture never reads heap bytes (`write_image`).
//!
//! The range's address is random, chosen by each process that starts a
//! fresh arena, so each world has its own; and the part of the window every
//! range is chosen from is reserved, inaccessible, a TiB at a time, at the
//! process's first allocation, before anything the process maps later could
//! land in it (`reserve_window`). A builder's range and an image's are in
//! BUILD, a process's first arena in FIRST above it, so an image's range
//! overlaps nothing of its restorer's: restore maps it over the
//! reservation, which cannot find the range taken. Both parts lie above
//! where a process with an unlimited stack, whose kernel maps bottom-up
//! from a sixth of the address space, has its libraries and later
//! mappings. A TiB something of the process holds before its first
//! allocation stays unreserved, and under an address-space limit nothing
//! is reserved: a range there is mapped only where nothing is, and a taken
//! range refuses the image (`Restore::Map`), with the step and error that
//! found it so (`map_failure`). Everything else a
//! world process maps stays randomized, the executable too. What an image holds of the
//! executable is read-only data: string literals, constant tables and the
//! empty hash table group in its first segment, vtables and `&'static`
//! tables in its RELRO segment. A process that maps an image maps a copy of
//! those two segments where the builder's executable had them, and nothing
//! else there (`shadow_executable`). An image holds no code or writable
//! static address: a VM at rest keeps no lowered code (`world_normalize`)
//! and no host attachment (`world_quiescent`), and nothing in the VM's
//! types holds a function pointer or a reference to a writable static.
//! Should one appear, the rest of the builder's executable range is mapped
//! inaccessible, so following it faults rather than running whatever now
//! lies there. The libraries hold nothing of an image.
//!
//! The allocator has size classes up to 32 KiB, carved from page slabs and
//! kept on free lists, and page runs above that, kept in bins by size. One
//! spin lock serializes it. A run freed at the top, or freed long enough
//! anywhere, gives its pages back, and so does every free run after a
//! collection (`trim`). Outside world binaries nothing installs it; when
//! its range cannot be reserved it hands every request to the system
//! allocator.
//!
//! With `ROMACS_HEAP_FILE=1`, the thread that adopts an image, which then
//! runs the VM, takes its later blocks from the production allocator's file
//! heap instead (`romacs_allocator::file_heap`), in address space of its
//! own, outside both arenas; a fork that thread asks to be lazy copies no
//! page-table entry of them (`start_heap_file`).
use romacs_allocator::file_heap;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU64, AtomicUsize, Ordering};

/// Where an arena may start: a GiB-aligned address in this window, far from
/// the executable, its heap, the libraries and the stack. Builders' arenas,
/// so images, start in BUILD; a process's first arena starts in FIRST, the
/// window's top eighth. Both are above the window's lower part, where a
/// process with an unlimited stack maps its libraries (bottom-up from
/// 0x1554_0000_0000 or so, a sixth of the address space, plus up to a TiB
/// of randomization; see the module documentation). The window is
/// reserved a CHUNK at a time.
const WINDOW: (usize, usize) = (0x1000_0000_0000, 0x5000_0000_0000);
const BUILD: (usize, usize) = (0x2800_0000_0000, 0x4800_0000_0000);
const FIRST: (usize, usize) = (0x4800_0000_0000, WINDOW.1);
const CHUNK: usize = 1 << 40;
const ALIGN: usize = 1 << 30;
/// The most an arena reserves. Pages are committed only as they are
/// touched; under an address-space limit an arena reserves a quarter of it,
/// as a process may hold two (`adopt`).
const RESERVE: usize = 1 << 37;
const PAGE: usize = 4096;
const MAGIC: u64 = u64::from_le_bytes(*b"RWARENA3");
/// 16..=1024 by 16, then four steps per doubling to 32 KiB.
const CLASSES: usize = 64 + 20;
const SMALL_MAX: usize = 32768;
const BINS: usize = 40;
/// Freed page runs this long give their pages back.
const RELEASE_PAGES: usize = 64;
/// When the top falls this far below the highest page touched since the
/// last release, the pages above it go back but for TRIM_KEEP: a peak's
/// pages do not stay resident, and a block freed and allocated again at
/// the top costs no system call.
const TRIM_SLACK: usize = 16 << 20;
const TRIM_KEEP: usize = 4 << 20;

/// The allocator's state, at the arena's start. An image carries it, and
/// the fields a restore needs.
#[repr(C)]
struct Header {
    magic: u64,
    /// Where the arena starts.
    base: usize,
    /// How much address space it holds from there.
    reserve: usize,
    /// The first page never handed out as a run.
    top: usize,
    /// Every address at or above this one is untouched, so zero.
    zero: usize,
    /// Pages above the top and below this address may be resident
    /// (`trim_top`).
    dirty: usize,
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
    /// Where the executable of the process that made it loaded.
    pub executable: usize,
    /// The world's key.
    pub key: [u8; 32],
}

/// The state of the process's arena.
const UNSET: u8 = 0;
const ARENA: u8 = 1;
const SYSTEM: u8 = 2;
static STATE: AtomicU8 = AtomicU8::new(UNSET);
static LOCK: AtomicBool = AtomicBool::new(false);
/// Whether freed blocks are zeroed: in a builder, so that no freed byte
/// reaches an image.
static SCRUB: AtomicBool = AtomicBool::new(false);
/// How the image this process asked for fared (`Restore`).
static RESTORE: AtomicU8 = AtomicU8::new(Restore::None as u8);
/// The chunks of the window this process reserved at its first allocation
/// (`reserve_window`): bit N for the Nth CHUNK from the window's start.
static RESERVED: AtomicU64 = AtomicU64::new(0);
/// What refused the image's range (`Restore::Map`): the step (an index in
/// STEPS) and the error; and whether the range was within the reservation.
static MAP_STEP: AtomicU8 = AtomicU8::new(0);
static MAP_ERRNO: AtomicI32 = AtomicI32::new(0);
static MAP_RESERVED: AtomicBool = AtomicBool::new(false);
const STEPS: [&str; 6] = ["", "no arena", "adopted already", "address-space limit", "image", "rest of the range"];
/// Set once an image is adopted: new blocks come from the production
/// allocator (`PLATFORM`) from then on, and the arenas only take back their own.
static ADOPTED: AtomicBool = AtomicBool::new(false);
/// A restored process's allocator for every block outside the arenas.
const PLATFORM: romacs_allocator::PlatformAllocator = romacs_allocator::PLATFORM_ALLOCATOR;
/// Whether the file heap this process asked for started (`start_heap_file`); unset when it asked for none.
static HEAP_FILE: std::sync::OnceLock<Result<(), file_heap::Refused>> = std::sync::OnceLock::new();

/// Where the arena allocations come from starts, once it has.
static BASE: AtomicUsize = AtomicUsize::new(0);
/// Where the process's first arena starts, once an image replaced it.
static FORMER: AtomicUsize = AtomicUsize::new(0);
/// A builder's arena is a shared mapping of this unlinked file, and an
/// image is copied from the file (`write_image`); -1 elsewhere.
static BACKING: AtomicI32 = AtomicI32::new(-1);
/// The libraries loaded as the arena started (`libraries`): for each, its
/// name, where it loaded and its program headers, the loader's own, which
/// stay for the process's life; and whether they all fit.
static mut LOADED: [(usize, usize, usize, usize); 256] = [(0, 0, 0, 0); 256];
static mut LOADED_COUNT: usize = 0;
static mut LOADED_COMPLETE: bool = false;

/// What became of the image this process adopted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Restore {
    /// None was adopted.
    None = 0,
    /// It is mapped: the arena is the image.
    Mapped,
    /// It could not be opened or read.
    Unreadable,
    /// It is no arena image of this format.
    Format,
    /// Where the builder's executable loaded is taken here.
    Executable,
    /// Its range could not be mapped.
    Map,
}

pub struct Arena;

fn base() -> usize {
    BASE.load(Ordering::Relaxed)
}

fn header() -> *mut Header {
    base() as *mut Header
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

/// FORK, a fork(2), with the allocator's lock held, and the lock released
/// after it in each process it returns in: a child has the forking thread
/// alone, so a lock another thread held at the fork would stay held there.
/// FORK allocates nothing.
pub fn across_fork<T>(fork: impl FnOnce() -> T) -> T {
    lock();
    let forked = fork();
    unlock();
    forked
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

/// A free run's head: its length in pages, the next run of its bin, and
/// whether its pages but the head's went back to the system.
#[repr(C)]
struct Run {
    pages: usize,
    next: usize,
    released: bool,
}

unsafe fn push_run(h: &mut Header, at: usize, pages: usize, released: bool) {
    let run = at as *mut Run;
    unsafe { *run = Run { pages, next: h.runs[bin(pages)], released } };
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
                        unsafe { push_run(h, at + pages * PAGE, rest, run.released) };
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
    if end > h.base + h.reserve {
        return 0;
    }
    if at > h.top {
        unsafe { push_run(h, h.top, (at - h.top) / PAGE, false) };
    }
    h.top = end;
    at
}

unsafe fn give_pages(h: &mut Header, at: usize, pages: usize) {
    if at + pages * PAGE == h.top {
        h.top = at;
        unsafe { trim_top(h, TRIM_SLACK) };
        return;
    }
    let released = pages >= RELEASE_PAGES;
    if released {
        unsafe { release(at + PAGE, (pages - 1) * PAGE) };
    }
    unsafe { push_run(h, at, pages, released) };
}

/// Record that the top grew: what lies below it is touched.
fn grew(h: &mut Header) {
    h.zero = h.zero.max(h.top);
    h.dirty = h.dirty.max(h.top);
}

/// Give back the pages above the top but for TRIM_KEEP, when more than
/// SLACK of them may be resident. The zero mark stays: a released page of
/// a restored image reads its file's bytes again, not zeros.
unsafe fn trim_top(h: &mut Header, slack: usize) {
    if h.dirty > h.top + slack {
        let keep = h.top + TRIM_KEEP.min(slack);
        unsafe { release(keep, h.dirty - keep) };
        h.dirty = keep;
    }
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

/// The header of the arena AT is in, or None for a block outside the arenas.
fn owner(at: *mut u8) -> Option<*mut Header> {
    let at = at as usize;
    // An adoption publishes FORMER before BASE: a block of the first arena
    // finds its header whichever of the two this reads.
    [BASE.load(Ordering::Acquire), FORMER.load(Ordering::Acquire)].into_iter()
        .find(|&base| base != 0 && (base..base + unsafe { (*(base as *const Header)).reserve }).contains(&at))
        .map(|base| base as *mut Header)
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

/// Reserve the window from BUILD up, inaccessible, a chunk at a time:
/// nothing the process maps after its first allocation lands in a reserved
/// chunk, and the ranges this module maps there replace the reservation
/// (`place`). A chunk something is mapped in already stays unreserved.
/// A reservation counts against an address-space limit, inaccessible as
/// it is: under one that holds less than the arenas themselves nothing is
/// tried, and the chunks stop where the limit is reached.
unsafe fn reserve_window() {
    if unsafe { reserve() } != Some(RESERVE) {
        return;
    }
    let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE | libc::MAP_FIXED_NOREPLACE;
    let mut bits = 0u64;
    let mut at = BUILD.0;
    while at < WINDOW.1 {
        let mapped = unsafe { libc::mmap(at as *mut libc::c_void, CHUNK, libc::PROT_NONE, flags, -1, 0) };
        if mapped == at as *mut libc::c_void {
            bits |= 1 << ((at - WINDOW.0) / CHUNK);
        } else if mapped != libc::MAP_FAILED {
            unsafe { libc::munmap(mapped, CHUNK) };
        } else if errno() == libc::ENOMEM {
            break;
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
    Some(((1u128 << (last + 1)) - (1u128 << first)) as u64)
}

/// Whether the range at AT, LEN bytes, is within this process's
/// reservation of the window (`reserve_window`).
pub fn reserved(at: usize, len: usize) -> bool {
    state();
    chunks(at, len).is_some_and(|mask| RESERVED.load(Ordering::Relaxed) & mask == mask)
}

/// The flags that map a range of this process at AT, LEN bytes: over the
/// reservation where the range is within it, else only where nothing is
/// mapped.
fn place(at: usize, len: usize) -> i32 {
    let within = chunks(at, len).is_some_and(|mask| RESERVED.load(Ordering::Relaxed) & mask == mask);
    if within { libc::MAP_FIXED } else { libc::MAP_FIXED_NOREPLACE }
}

/// A GiB-aligned base for a range of RESERVE bytes in the part LOW..HIGH
/// of the window, from RANDOM: in one of the part's reserved chunks when
/// it has any, anywhere in the part otherwise.
fn base_in(low: usize, high: usize, reserve: usize, random: usize) -> usize {
    let mask = chunks(low, high - low).unwrap_or(0) & RESERVED.load(Ordering::Relaxed);
    if mask == 0 {
        return low + random % ((high - low - reserve) / ALIGN) * ALIGN;
    }
    let nth = random % mask.count_ones() as usize;
    let chunk = (0..64).filter(|bit| mask & (1 << bit) != 0).nth(nth).unwrap();
    WINDOW.0 + chunk * CHUNK + (random >> 8) % ((CHUNK - reserve) / ALIGN) * ALIGN
}

/// Give up the range at AT, LEN bytes, this module's mapping placed with
/// PLACED (`place`): back to the reservation where it was mapped over it,
/// else unmapped.
unsafe fn unplace(at: usize, len: usize, placed: i32) {
    if placed == libc::MAP_FIXED {
        let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE | libc::MAP_FIXED;
        unsafe { libc::mmap(at as *mut libc::c_void, len, libc::PROT_NONE, flags, -1, 0) };
    } else {
        unsafe { libc::munmap(at as *mut libc::c_void, len) };
    }
}

/// Reserve a fresh range at a random address in the window: a builder's
/// in BUILD, where images are made, any other process's first arena in
/// FIRST.
unsafe fn start() -> u8 {
    unsafe { take_inventory() };
    unsafe { reserve_window() };
    // HOST(libc::getenv): the world allocator reads whether it runs in a builder at its first
    // allocation, before host.rs can run; it zeroes freed blocks and backs its arena with a file
    let building = !unsafe { libc::getenv(c"ROMACS_WORLD_BUILD".as_ptr()) }.is_null();
    SCRUB.store(building, Ordering::Relaxed);
    let Some(reserve) = (unsafe { reserve() }) else { return SYSTEM };
    let backing = if building { unsafe { backing_file(reserve) } } else { -1 };
    let flags = libc::MAP_NORESERVE | if backing >= 0 { libc::MAP_SHARED } else { libc::MAP_PRIVATE | libc::MAP_ANONYMOUS };
    let (low, high) = if building { BUILD } else { FIRST };
    for _ in 0..8 {
        let mut random = 0usize;
        // HOST(libc::getrandom): the world allocator picks a random arena address at its first
        // allocation; no Lisp value depends on it
        let read = unsafe { libc::getrandom((&raw mut random).cast(), size_of::<usize>(), 0) };
        if read != size_of::<usize>() as isize {
            break;
        }
        let base = base_in(low, high, reserve, random);
        let at = unsafe { libc::mmap(base as *mut libc::c_void, reserve, libc::PROT_READ | libc::PROT_WRITE, flags | place(base, reserve), backing, 0) };
        if at == base as *mut libc::c_void {
            BACKING.store(backing, Ordering::Relaxed);
            BASE.store(base, Ordering::Relaxed);
            let h = unsafe { &mut *header() };
            h.magic = MAGIC;
            h.base = base;
            h.reserve = reserve;
            h.top = base + size_of::<Header>().next_multiple_of(PAGE);
            h.zero = h.top;
            h.dirty = h.top;
            return ARENA;
        }
        if at != libc::MAP_FAILED {
            unsafe { libc::munmap(at, reserve) };
        }
    }
    SYSTEM
}

/// A builder's backing file: unlinked, in memory, RESERVE bytes long and
/// sparse, so only pages the arena touched hold blocks. -1 when there is
/// none: a builder without one cannot publish (`write_image`).
unsafe fn backing_file(reserve: usize) -> i32 {
    // HOST(libc::file): a builder backs its arena with an unlinked memory file at its first
    // allocation, before host.rs can run; no Lisp value depends on it
    let fd = unsafe { libc::open(c"/dev/shm".as_ptr(), libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC, 0o600) };
    if fd < 0 {
        return -1;
    }
    // HOST(libc::file): a builder backs its arena with an unlinked memory file at its first
    // allocation, before host.rs can run; no Lisp value depends on it
    if unsafe { libc::ftruncate(fd, reserve as i64) } != 0 {
        unsafe { libc::close(fd) };
        return -1;
    }
    fd
}

/// Give the pages from AT, LEN bytes, back to the system. Their content is
/// the allocator's no longer: memory below the zero mark is zeroed when a
/// zeroed block is asked for.
unsafe fn release(at: usize, len: usize) {
    let backing = BACKING.load(Ordering::Relaxed);
    if backing >= 0 && in_backing(at) {
        let offset = (at - BASE.load(Ordering::Relaxed)) as i64;
        unsafe { libc::fallocate(backing, libc::FALLOC_FL_PUNCH_HOLE | libc::FALLOC_FL_KEEP_SIZE, offset, len as i64) };
    } else {
        unsafe { libc::madvise(at as *mut libc::c_void, len, libc::MADV_DONTNEED) };
    }
}

/// Whether AT is in the arena the backing file maps: a builder's only one.
fn in_backing(at: usize) -> bool {
    let base = BASE.load(Ordering::Relaxed);
    FORMER.load(Ordering::Relaxed) == 0 && base != 0 && (base..base + unsafe { (*(base as *const Header)).reserve }).contains(&at)
}

/// How much address space an arena of this process reserves: RESERVE, or
/// a GiB-aligned quarter of the address-space limit, when that is at least
/// a GiB.
unsafe fn reserve() -> Option<usize> {
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    if unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut limit) } != 0 || limit.rlim_cur == libc::RLIM_INFINITY {
        return Some(RESERVE);
    }
    let quarter = (limit.rlim_cur as usize / 4).min(RESERVE) / ALIGN * ALIGN;
    (quarter >= ALIGN).then_some(quarter)
}

/// Allocate from the image in FILE from now on: map it where it was made,
/// over this process's reservation of the window, or where nothing is when
/// the window is not reserved (`place`), with what it addresses of the
/// executable (`shadow_executable`), when it is a whole image of this
/// format. Once per process; the first arena keeps the blocks allocated
/// before. Refused, `map_failure` says at which step and why.
pub fn adopt(file: &std::fs::File) -> Restore {
    use std::os::fd::AsRawFd;
    if state() != ARENA {
        return refuse(1, 0, false);
    }
    // The first image stays this process's: what became of it stays said.
    if FORMER.load(Ordering::Acquire) != 0 {
        return refuse(2, 0, false);
    }
    let outcome = unsafe { map_image_fd(file.as_raw_fd()) };
    RESTORE.store(outcome as u8, Ordering::Release);
    outcome
}

/// The image's range is refused at STEP (an index in STEPS) with ERRNO;
/// RESERVED says whether the range was within the reservation.
fn refuse(step: u8, errno: i32, reserved: bool) -> Restore {
    MAP_ERRNO.store(errno, Ordering::Relaxed);
    MAP_RESERVED.store(reserved, Ordering::Relaxed);
    MAP_STEP.store(step, Ordering::Release);
    Restore::Map
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// What refused the image's range, once `adopt` has (`Restore::Map`): the
/// step, the error number of the moment, 0 for none, and whether the range
/// was within this process's reservation of the window.
pub fn map_failure() -> Option<(&'static str, i32, bool)> {
    let step = MAP_STEP.load(Ordering::Acquire);
    (step != 0).then(|| (STEPS[step as usize], MAP_ERRNO.load(Ordering::Relaxed), MAP_RESERVED.load(Ordering::Relaxed)))
}

/// Whether the ranges AT, LEN bytes, and OTHER, OTHER_LEN bytes, share an address.
fn overlaps(at: usize, len: usize, other: usize, other_len: usize) -> bool {
    at < other + other_len && other < at + len
}

unsafe fn map_image_fd(fd: i32) -> Restore {
    let mut bytes = [0u8; size_of::<Header>()];
    let read = unsafe { libc::pread(fd, bytes.as_mut_ptr().cast(), bytes.len(), 0) };
    // HOST(libc::file): the world allocator checks an image is whole before it adopts it; the
    // launcher checked that image
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // HOST(libc::file): the world allocator checks an image is whole before it adopts it; the
    // launcher checked that image
    if read != bytes.len() as isize || unsafe { libc::fstat(fd, &mut stat) } != 0 {
        return Restore::Unreadable;
    }
    let image = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<Header>()) };
    let (base, len) = (image.base, image.image.len);
    // A truncated or extended file is no image: mapped, it would fault.
    if image.magic != MAGIC || len == 0 || len % PAGE != 0 || stat.st_size as usize != len
        || base % ALIGN != 0 || base < WINDOW.0 || base.saturating_add(len) > WINDOW.1 || !(base..=base + len).contains(&image.top)
    {
        return Restore::Format;
    }
    let reserve = match unsafe { reserve() } {
        Some(reserve) if len < reserve && base + reserve <= WINDOW.1 => reserve,
        _ => return refuse(3, 0, false),
    };
    // Over the reservation, unless the range reaches this process's first
    // arena (an image made under another address-space limit could): then
    // only where nothing is, which refuses it.
    let first = unsafe { &*header() };
    let placed = if overlaps(base, reserve, first.base, first.reserve) { libc::MAP_FIXED_NOREPLACE } else { place(base, reserve) };
    let file = libc::MAP_PRIVATE | placed;
    let at = unsafe { libc::mmap(base as *mut libc::c_void, len, libc::PROT_READ | libc::PROT_WRITE, file, fd, 0) };
    // What a failure leaves is given up (`unplace`): only what this
    // module mapped, never what refused it, which the launch's source boot
    // may be using.
    if at != base as *mut libc::c_void {
        let refused = refuse(4, errno(), placed == libc::MAP_FIXED);
        if at != libc::MAP_FAILED {
            unsafe { libc::munmap(at, len) };
        } else if placed == libc::MAP_FIXED {
            unsafe { unplace(base, len, placed) };
        }
        return refused;
    }
    let anon = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE | placed;
    let rest = unsafe { libc::mmap((base + len) as *mut libc::c_void, reserve - len, libc::PROT_READ | libc::PROT_WRITE, anon, -1, 0) };
    if rest != (base + len) as *mut libc::c_void {
        let refused = refuse(5, errno(), placed == libc::MAP_FIXED);
        if rest != libc::MAP_FAILED {
            unsafe { libc::munmap(rest, reserve - len) };
        }
        unsafe { unplace(base, if placed == libc::MAP_FIXED { reserve } else { len }, placed) };
        return refused;
    }
    if !unsafe { shadow_executable(image.image.executable) } {
        unsafe { unplace(base, reserve, placed) };
        return Restore::Executable;
    }
    // This process's reservation, not the builder's.
    unsafe { (*(base as *mut Header)).reserve = reserve };
    // The kernel reads the file ahead while the process starts.
    unsafe { libc::madvise(base as *mut libc::c_void, len, libc::MADV_WILLNEED) };
    lock();
    FORMER.store(BASE.load(Ordering::Relaxed), Ordering::Release);
    BASE.store(base, Ordering::Release);
    ADOPTED.store(true, Ordering::Release);
    unlock();
    start_heap_file(reserve);
    Restore::Mapped
}

/// With `ROMACS_HEAP_FILE=1`, start the file heap, LEN bytes as an arena
/// has, for the calling thread: the one that adopted the image, which runs
/// the VM. The switch is read from the environment itself, not through the
/// host, so that a process that asks for none reads nothing the host
/// records.
fn start_heap_file(len: usize) {
    if std::env::var_os("ROMACS_HEAP_FILE").is_some_and(|asked| asked == "1") {
        let _ = HEAP_FILE.set(file_heap::start(len));
    }
}

/// The file heap's counts, when this process asked for one
/// (`ROMACS_HEAP_FILE`) as it adopted its image; or why it was refused.
#[must_use]
pub fn heap_file() -> Option<Result<file_heap::Stats, file_heap::Refused>> {
    HEAP_FILE.get().map(|started| started.map(|()| file_heap::stats()))
}

/// The executable's load segments, as the kernel loaded it: where it
/// starts, and each PT_LOAD segment's (address, file offset, file size,
/// memory size, flags), and the RELRO range, all relative to the start.
struct Executable {
    base: usize,
    loads: [(usize, usize, usize, usize, u32); 8],
    count: usize,
    relro: (usize, usize),
}

/// This process's executable, from its program headers, without allocating.
unsafe fn executable() -> Option<Executable> {
    let phdr = unsafe { libc::getauxval(libc::AT_PHDR) } as usize;
    let count = unsafe { libc::getauxval(libc::AT_PHNUM) } as usize;
    if phdr == 0 {
        return None;
    }
    let headers = unsafe { std::slice::from_raw_parts(phdr as *const libc::Elf64_Phdr, count) };
    let own = headers.iter().find(|h| h.p_type == libc::PT_PHDR)?;
    let mut exe = Executable { base: phdr - own.p_vaddr as usize, loads: [(0, 0, 0, 0, 0); 8], count: 0, relro: (0, 0) };
    for h in headers {
        match h.p_type {
            libc::PT_LOAD if exe.count < exe.loads.len() => {
                exe.loads[exe.count] = (h.p_vaddr as usize, h.p_offset as usize, h.p_filesz as usize, h.p_memsz as usize, h.p_flags);
                exe.count += 1;
            }
            libc::PT_LOAD => return None,
            libc::PT_GNU_RELRO => exe.relro = (h.p_vaddr as usize, h.p_vaddr as usize + h.p_memsz as usize),
            _ => {}
        }
    }
    Some(exe)
}

/// Where this process's executable loaded.
pub fn executable_base() -> usize {
    unsafe { executable() }.map_or(0, |exe| exe.base)
}

/// Map, at OLD, where the builder's executable loaded, what an image may
/// address there: the read-only segments, from the executable's file, and
/// a read-only copy of this process's RELRO, whose pointers are this
/// process's. The rest of the builder's executable range is reserved
/// inaccessible. False when that range is taken.
unsafe fn shadow_executable(old: usize) -> bool {
    let Some(exe) = (unsafe { executable() }) else { return false };
    if old == exe.base {
        return true;
    }
    let loads = &exe.loads[..exe.count];
    let floor = |n: usize| n / PAGE * PAGE;
    let ceil = |n: usize| n.next_multiple_of(PAGE);
    let Some(low) = loads.iter().map(|l| floor(l.0)).min() else { return false };
    let high = loads.iter().map(|l| ceil(l.0 + l.3)).max().unwrap_or(low);
    let span = high - low;
    let anon = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE;
    let at = unsafe { libc::mmap((old + low) as *mut libc::c_void, span, libc::PROT_NONE, anon | libc::MAP_FIXED_NOREPLACE, -1, 0) };
    if at != (old + low) as *mut libc::c_void {
        if at != libc::MAP_FAILED {
            unsafe { libc::munmap(at, span) };
        }
        return false;
    }
    let undo = || { unsafe { libc::munmap((old + low) as *mut libc::c_void, span) }; false };
    // HOST(libc::file): the world allocator maps its own executable read-only where an image
    // expects it; no Lisp value depends on it
    let fd = unsafe { libc::open(c"/proc/self/exe".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return undo();
    }
    let mut mapped = true;
    for &(vaddr, offset, filesz, _, flags) in loads.iter().filter(|l| l.4 & (libc::PF_W | libc::PF_X) == 0) {
        let (start, end) = (floor(vaddr), ceil(vaddr + filesz));
        let at = unsafe { libc::mmap((old + start) as *mut libc::c_void, end - start, libc::PROT_READ, libc::MAP_PRIVATE | libc::MAP_FIXED, fd, floor(offset) as i64) };
        mapped &= at == (old + start) as *mut libc::c_void && flags & libc::PF_R != 0;
    }
    unsafe { libc::close(fd) };
    let (start, end) = (floor(exe.relro.0), ceil(exe.relro.1));
    if mapped && end > start {
        let at = unsafe { libc::mmap((old + start) as *mut libc::c_void, end - start, libc::PROT_READ | libc::PROT_WRITE, anon | libc::MAP_FIXED, -1, 0) };
        mapped = at == (old + start) as *mut libc::c_void;
        if mapped {
            unsafe { std::ptr::copy_nonoverlapping((exe.base + start) as *const u8, (old + start) as *mut u8, end - start) };
            mapped = unsafe { libc::mprotect(at, end - start, libc::PROT_READ) } == 0;
        }
    }
    if !mapped {
        return undo();
    }
    true
}

/// Note the libraries loaded so far (`LOADED`), through dl_iterate_phdr:
/// the objects themselves, not paths. The executable, keyed by its file's
/// content, and the vDSO, the kernel's, which no image refers to, are left
/// out. Allocates nothing.
unsafe fn take_inventory() {
    unsafe extern "C" fn each(info: *mut libc::dl_phdr_info, _: libc::size_t, _: *mut libc::c_void) -> libc::c_int {
        let info = unsafe { &*info };
        if info.dlpi_name.is_null() {
            return 0;
        }
        let name = unsafe { std::ffi::CStr::from_ptr(info.dlpi_name) }.to_bytes();
        if name.is_empty() || name.starts_with(b"linux-vdso") {
            return 0;
        }
        let count = unsafe { LOADED_COUNT };
        if count == 256 {
            unsafe { LOADED_COMPLETE = false };
            return 1;
        }
        unsafe {
            LOADED[count] = (info.dlpi_name as usize, info.dlpi_addr as usize, info.dlpi_phdr as usize, info.dlpi_phnum as usize);
            LOADED_COUNT = count + 1;
        }
        0
    }
    unsafe {
        LOADED_COMPLETE = true;
        libc::dl_iterate_phdr(Some(each), std::ptr::null_mut());
    }
}

/// A library loaded before the first allocation, as the loader mapped it.
#[derive(Clone, Copy, Debug)]
pub struct Library {
    /// Its name, as the loader found it: a path.
    pub name: &'static [u8],
    base: usize,
    headers: &'static [libc::Elf64_Phdr],
}

/// The libraries this process loaded before its first allocation, or
/// None when there were more than the inventory holds.
pub fn libraries() -> Option<Vec<Library>> {
    state();
    if !unsafe { LOADED_COMPLETE } {
        return None;
    }
    let loaded: &[(usize, usize, usize, usize); 256] = unsafe { &*std::ptr::addr_of!(LOADED) };
    let loaded = &loaded[..unsafe { LOADED_COUNT }];
    Some(loaded.iter().map(|&(name, base, phdr, phnum)| unsafe { Library::at(name as *const libc::c_char, base, phdr, phnum) }).collect())
}

impl Library {
    /// The object a dl_phdr_info describes: NAME, loaded at BASE, with
    /// PHNUM program headers at PHDR.
    ///
    /// # Safety
    /// The object stays loaded for the process's life.
    pub unsafe fn at(name: *const libc::c_char, base: usize, phdr: usize, phnum: usize) -> Library {
        Library { name: unsafe { std::ffi::CStr::from_ptr(name) }.to_bytes(), base,
                  headers: unsafe { std::slice::from_raw_parts(phdr as *const libc::Elf64_Phdr, phnum) } }
    }

    /// The GNU build-id note of its PT_NOTE segments, as mapped.
    pub fn build_id(&self) -> Option<&'static [u8]> {
        self.headers.iter().filter(|h| h.p_type == libc::PT_NOTE).find_map(|h| {
            build_id(unsafe { std::slice::from_raw_parts((self.base + h.p_vaddr as usize) as *const u8, h.p_memsz as usize) })
        })
    }

    /// Where its first segment is mapped: an address in the object.
    pub fn address(&self) -> usize {
        self.headers.iter().find(|h| h.p_type == libc::PT_LOAD).map_or(self.base, |h| self.base + h.p_vaddr as usize)
    }

    /// Whether the loader writes its read-only segments: its dynamic
    /// section has DT_TEXTREL, or DF_TEXTREL in DT_FLAGS.
    pub fn text_relocations(&self) -> bool {
        const DT_TEXTREL: i64 = 22;
        const DT_FLAGS: i64 = 30;
        const DF_TEXTREL: u64 = 0x4;
        /// Elf64_Dyn: a tag and its value.
        #[repr(C)]
        struct Dynamic { tag: i64, value: u64 }
        self.headers.iter().filter(|h| h.p_type == libc::PT_DYNAMIC).any(|h| {
            let count = h.p_memsz as usize / size_of::<Dynamic>();
            let dynamic = unsafe { std::slice::from_raw_parts((self.base + h.p_vaddr as usize) as *const Dynamic, count) };
            dynamic.iter().take_while(|d| d.tag != 0)
                .any(|d| d.tag == DT_TEXTREL || (d.tag == DT_FLAGS && d.value & DF_TEXTREL != 0))
        })
    }

    /// What the loader leaves as the file has it: each non-writable
    /// PT_LOAD segment's bytes, code and read-only data, as mapped, but for
    /// the RELRO range the loader writes, each with the file offset it maps.
    /// Position-independent code and data are not relocated there, so they
    /// are alike in every process. None when a segment is not readable.
    pub fn constant_segments(&self) -> Option<Vec<(u64, &'static [u8])>> {
        let relro = self.headers.iter().filter(|h| h.p_type == libc::PT_GNU_RELRO)
            .map(|h| (h.p_vaddr, h.p_vaddr + h.p_memsz)).collect::<Vec<_>>();
        let mut out = vec![];
        for h in self.headers.iter().filter(|h| h.p_type == libc::PT_LOAD && h.p_flags & libc::PF_W == 0) {
            if h.p_flags & libc::PF_R == 0 {
                return None;
            }
            // The segment's file bytes, less any RELRO range within them.
            let mut pieces = vec![(h.p_vaddr, h.p_vaddr + h.p_filesz)];
            for &(lo, hi) in &relro {
                pieces = pieces.into_iter().flat_map(|(a, b)| [(a, b.min(lo)), (a.max(hi), b)]).filter(|(a, b)| a < b).collect();
            }
            for (a, b) in pieces {
                let bytes = unsafe { std::slice::from_raw_parts((self.base + a as usize) as *const u8, (b - a) as usize) };
                out.push((h.p_offset + (a - h.p_vaddr), bytes));
            }
        }
        (!out.is_empty()).then_some(out)
    }
}

/// The GNU build-id in NOTES, an ELF note section.
fn build_id(notes: &[u8]) -> Option<&[u8]> {
    const NT_GNU_BUILD_ID: u32 = 3;
    let word = |at: usize| notes.get(at..at + 4).map(|b| u32::from_ne_bytes(b.try_into().unwrap()));
    let mut at = 0;
    while at + 12 <= notes.len() {
        let (name_size, desc_size, kind) = (word(at)? as usize, word(at + 4)? as usize, word(at + 8)?);
        let name = at + 12;
        let desc = name + name_size.next_multiple_of(4);
        let next = desc + desc_size.next_multiple_of(4);
        if next > notes.len() {
            return None;
        }
        if kind == NT_GNU_BUILD_ID && &notes[name..name + name_size] == b"GNU\0" && desc_size > 0 {
            return Some(&notes[desc..desc + desc_size]);
        }
        at = next;
    }
    None
}

/// How the image this process adopted fared.
pub fn restore_outcome() -> Restore {
    match RESTORE.load(Ordering::Acquire) {
        1 => Restore::Mapped,
        2 => Restore::Unreadable,
        3 => Restore::Format,
        4 => Restore::Executable,
        5 => Restore::Map,
        _ => Restore::None,
    }
}

/// Whether the heap is an arena whose used prefix will be an image: a
/// builder's, the only heap `write_image` will write. What goes into it goes
/// into the world, so nothing that depends on the moment belongs there.
pub fn heap_is_image() -> bool {
    BACKING.load(Ordering::Relaxed) >= 0 && !ADOPTED.load(Ordering::Acquire)
}

/// Whether the arena is a shared mapping (a builder's, `backing_file`): a
/// forked child then shares its blocks with the parent instead of copying them.
pub fn heap_is_shared() -> bool {
    BACKING.load(Ordering::Relaxed) >= 0
}

/// Whether a heap table may take its one large reservation (`table.rs`): not
/// where the heap is an image, nor under an address-space limit, which the
/// reservation would eat into.
pub fn may_reserve() -> bool {
    if heap_is_image() {
        return false;
    }
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: getrlimit writes only the local it is given.
    unsafe { libc::getrlimit(libc::RLIMIT_AS, &mut limit) == 0 && limit.rlim_cur == libc::RLIM_INFINITY }
}

/// Give the system the whole pages between FROM and TO bytes past BASE, the
/// spare capacity of a reserved table, whose contents are unused. SAFETY:
/// those bytes are within one allocation the caller owns, and nothing reads
/// or holds them: the pages come back zeroed, or, in the file heap
/// (`start_heap_file`), as the heap's file last held them.
pub(crate) unsafe fn release_spare(base: *mut u8, from: usize, to: usize) {
    let start = (base as usize + from).next_multiple_of(PAGE);
    let end = (base as usize + to) / PAGE * PAGE;
    if end > start {
        // SAFETY: this function's contract: these whole pages lie in the
        // caller's allocation, and nothing reads or holds them.
        unsafe { libc::madvise(start as *mut libc::c_void, end - start, libc::MADV_DONTNEED) };
    }
}

/// Whether this process's heap is the arena.
pub fn active() -> bool {
    state() == ARENA
}

/// Whether the arena already serves this process, without starting one:
/// a process whose global allocator is another never maps it.
fn started() -> bool {
    STATE.load(Ordering::Acquire) == ARENA
}

/// The range the arena's allocations come from: where it starts and how
/// much address space it holds. None when the heap is the system's.
pub fn current_range() -> Option<(usize, usize)> {
    active().then(|| { let h = unsafe { &*header() }; (h.base, h.reserve) })
}

/// Where the arena's bytes are: the span handed out from its start
/// (`used`), and of that span the free small blocks (`small_free`, by
/// class in `classes`), the uncarved rest of the current slabs
/// (`slab_rest`) and the free runs, whose pages are kept (`runs_kept`) or
/// went back to the system (`runs_released`); and the pages above the top
/// that may still be resident (`above_top`). Both arenas of a process
/// that adopted an image count.
#[derive(Clone, Debug, Default)]
pub struct Census {
    pub used: usize,
    pub small_free: usize,
    pub slab_rest: usize,
    pub runs_kept: usize,
    pub runs_released: usize,
    pub above_top: usize,
    pub classes: Vec<(usize, usize)>,
}

/// The arena's census (`Census`), or None when the heap is the system's.
/// It walks every free list under the allocator's lock.
pub fn census() -> Option<Census> {
    if !started() {
        return None;
    }
    let mut c = Census { classes: vec![(0, 0); CLASSES], ..Census::default() };
    lock();
    for base in [BASE.load(Ordering::Acquire), FORMER.load(Ordering::Acquire)] {
        if base != 0 {
            unsafe { count(&*(base as *const Header), &mut c) };
        }
    }
    unlock();
    for (class, row) in c.classes.iter_mut().enumerate() {
        row.0 = class_size(class);
    }
    c.small_free = c.classes.iter().map(|&(_, free)| free).sum();
    c.classes.retain(|&(_, free)| free > 0);
    Some(c)
}

/// Add H's free lists to C. The caller holds the lock.
unsafe fn count(h: &Header, c: &mut Census) {
    c.used += h.top - h.base;
    c.above_top += h.dirty.saturating_sub(h.top);
    for class in 0..CLASSES {
        let mut at = h.free[class];
        while at != 0 {
            c.classes[class].1 += class_size(class);
            at = unsafe { *(at as *const usize) };
        }
        c.slab_rest += h.end[class] - h.cursor[class];
    }
    for b in 0..BINS {
        let mut at = h.runs[b];
        while at != 0 {
            let run = unsafe { &*(at as *const Run) };
            if run.released {
                c.runs_released += (run.pages - 1) * PAGE;
                c.runs_kept += PAGE;
            } else {
                c.runs_kept += run.pages * PAGE;
            }
            at = run.next;
        }
    }
}

/// Give free memory back to the system after a collection: the production
/// allocator's free pages (`romacs_allocator::release_free`), and the arenas' pages above
/// their tops and in their free runs but for each run's head. A run
/// remembers its release, so a later trim skips it.
pub fn trim() {
    if !started() {
        return;
    }
    if ADOPTED.load(Ordering::Acquire) {
        romacs_allocator::release_free();
    }
    lock();
    for base in [BASE.load(Ordering::Acquire), FORMER.load(Ordering::Acquire)] {
        if base == 0 {
            continue;
        }
        let h = unsafe { &mut *(base as *mut Header) };
        unsafe { trim_top(h, 0) };
        for b in 0..BINS {
            let mut at = h.runs[b];
            while at != 0 {
                let run = unsafe { &mut *(at as *mut Run) };
                if !run.released && run.pages > 1 {
                    unsafe { release(at + PAGE, (run.pages - 1) * PAGE) };
                    run.released = true;
                }
                at = run.next;
            }
        }
    }
    unlock();
}

/// The fields of the image this process mapped.
pub fn mapped_image() -> Option<ImageFields> {
    (restore_outcome() == Restore::Mapped).then(|| unsafe { (*header()).image })
}

/// Write the arena to FILE as an image with FIELDS (its length is set
/// here), with holes where the arena never touched a page. The caller
/// allocates nothing meanwhile on any thread: the allocator is locked
/// throughout. Only a builder writes images: its arena is a shared mapping
/// of a backing file.
///
/// The image is the backing file's bytes, copied by the kernel. Nothing
/// here loads heap memory as Rust values: the heap holds padding and
/// unused capacity no Rust value initialized, and reading those bytes,
/// even to test them for zero, would be undefined behaviour. The file's
/// extents (SEEK_DATA) say which pages hold data, and copy_file_range,
/// or pread into a buffer this function owns, moves it. The allocator's
/// own loads here are of words it stored: the free-list links and run
/// heads `scrub` follows.
///
/// # Safety
/// FIELDS.root must be the address of a live allocation the image's reader
/// takes ownership of.
pub unsafe fn write_image(file: &std::fs::File, mut fields: ImageFields) -> std::io::Result<usize> {
    use std::os::fd::AsRawFd;
    if !active() {
        return Err(std::io::Error::other("the arena is not this process's heap"));
    }
    let backing = BACKING.load(Ordering::Relaxed);
    if backing < 0 || FORMER.load(Ordering::Relaxed) != 0 {
        return Err(std::io::Error::other("only a builder's file-backed arena can be written as an image"));
    }
    lock();
    let h = unsafe { &mut *header() };
    unsafe { scrub(h) };
    fields.len = h.top - base();
    let saved = h.image;
    h.image = fields;
    let result = unsafe { copy_extents(backing, file.as_raw_fd(), fields.len) };
    h.image = saved;
    unlock();
    result.map(|()| fields.len)
}

/// Zero what free blocks still hold, but for their links, and give free
/// runs' pages back: freed bytes are no part of a world, and a released
/// page is a hole in its file.
unsafe fn scrub(h: &mut Header) {
    for class in 0..CLASSES {
        let size = class_size(class);
        let mut at = h.free[class];
        while at != 0 {
            unsafe { std::ptr::write_bytes((at + 8) as *mut u8, 0, size - 8) };
            at = unsafe { *(at as *const usize) };
        }
    }
    for b in 0..BINS {
        let mut at = h.runs[b];
        while at != 0 {
            let run = unsafe { &mut *(at as *mut Run) };
            unsafe { std::ptr::write_bytes((at + size_of::<Run>()) as *mut u8, 0, PAGE - size_of::<Run>()) };
            if run.pages > 1 {
                unsafe { release(at + PAGE, (run.pages - 1) * PAGE) };
                run.released = true;
            }
            at = run.next;
        }
    }
}

/// Copy the data extents of FROM's first LEN bytes to the same offsets
/// of TO, and make TO LEN bytes long: its holes stay holes.
unsafe fn copy_extents(from: i32, to: i32, len: usize) -> std::io::Result<()> {
    let last = || std::io::Error::last_os_error();
    let mut at = 0i64;
    while (at as usize) < len {
        let data = unsafe { libc::lseek(from, at, libc::SEEK_DATA) };
        if data < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENXIO) {
                break;
            }
            return Err(last());
        }
        if data as usize >= len {
            break;
        }
        let hole = unsafe { libc::lseek(from, data, libc::SEEK_HOLE) };
        if hole < 0 {
            return Err(last());
        }
        let end = (hole as usize).min(len) as i64;
        unsafe { copy_range(from, to, data, end) }?;
        at = end;
    }
    // HOST(libc::file): copies the builder arena's backing file into the image file the builder
    // opened through host
    if unsafe { libc::ftruncate(to, len as i64) } != 0 {
        return Err(last());
    }
    Ok(())
}

/// Bytes START..END of FROM to the same offsets of TO: by the kernel, or
/// through a buffer of this module's where it cannot copy between them.
unsafe fn copy_range(from: i32, to: i32, start: i64, end: i64) -> std::io::Result<()> {
    static mut BUFFER: [u8; 1 << 20] = [0; 1 << 20];
    let mut at = start;
    while at < end {
        let (mut input, mut output) = (at, at);
        // HOST(libc::file): copies the builder arena's backing file into the image file the builder
        // opened through host
        let n = unsafe { libc::copy_file_range(from, &mut input, to, &mut output, (end - at) as usize, 0) };
        if n > 0 {
            at += n as isize as i64;
            continue;
        }
        let error = std::io::Error::last_os_error();
        if n == 0 || !matches!(error.raw_os_error(), Some(libc::EXDEV | libc::EINVAL | libc::ENOSYS | libc::EOPNOTSUPP)) {
            return Err(if n == 0 { std::io::Error::other("the backing file ended early") } else { error });
        }
        let buffer = unsafe { &mut *std::ptr::addr_of_mut!(BUFFER) };
        let want = buffer.len().min((end - at) as usize);
        let read = unsafe { libc::pread(from, buffer.as_mut_ptr().cast(), want, at) };
        if read <= 0 {
            return Err(if read == 0 { std::io::Error::other("the backing file ended early") } else { std::io::Error::last_os_error() });
        }
        let mut done = 0;
        while done < read as usize {
            let wrote = unsafe { libc::pwrite(to, buffer[done..].as_ptr().cast(), read as usize - done, at + done as i64) };
            if wrote < 0 {
                return Err(std::io::Error::last_os_error());
            }
            done += wrote as usize;
        }
        at += read as i64;
    }
    Ok(())
}

unsafe impl GlobalAlloc for Arena {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ADOPTED.load(Ordering::Relaxed) {
            return unsafe { PLATFORM.alloc(layout) };
        }
        if state() != ARENA {
            return unsafe { System.alloc(layout) };
        }
        lock();
        let h = unsafe { &mut *header() };
        let at = match class_of(layout.size(), layout.align()) {
            Some(class) => unsafe { take_small(h, class) },
            None => unsafe { take_pages(h, pages(layout.size()), layout.align().max(PAGE)) },
        };
        grew(h);
        unlock();
        at as *mut u8
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if ADOPTED.load(Ordering::Relaxed) {
            return unsafe { PLATFORM.alloc_zeroed(layout) };
        }
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
        grew(h);
        unlock();
        // Memory at or above the old zero mark was never handed out.
        if at != 0 && at < zero {
            unsafe { std::ptr::write_bytes(at as *mut u8, 0, layout.size().min(zero - at)) };
        }
        at as *mut u8
    }

    unsafe fn dealloc(&self, at: *mut u8, layout: Layout) {
        let Some(owner) = owner(at) else {
            // A block outside the arenas is the production allocator's in a
            // process that adopted an image (every earlier block is an
            // arena's), else the system's (an arena that never started).
            return unsafe {
                if ADOPTED.load(Ordering::Acquire) { PLATFORM.dealloc(at, layout) } else { System.dealloc(at, layout) }
            };
        };
        if SCRUB.load(Ordering::Relaxed) {
            unsafe { std::ptr::write_bytes(at, 0, layout.size()) };
        }
        lock();
        let h = unsafe { &mut *owner };
        match class_of(layout.size(), layout.align()) {
            Some(class) => unsafe { give_small(h, at as usize, class) },
            None => unsafe { give_pages(h, at as usize, pages(layout.size())) },
        }
        unlock();
    }

    unsafe fn realloc(&self, at: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let Some(owner) = owner(at) else {
            return unsafe {
                if ADOPTED.load(Ordering::Acquire) { PLATFORM.realloc(at, layout, new_size) } else { System.realloc(at, layout, new_size) }
            };
        };
        let align = layout.align();
        if SCRUB.load(Ordering::Relaxed) && new_size < layout.size() {
            unsafe { std::ptr::write_bytes(at.add(new_size), 0, layout.size() - new_size) };
        }
        match (class_of(layout.size(), align), class_of(new_size, align)) {
            (Some(old), Some(new)) if old == new => return at,
            (None, None) if align <= PAGE => {
                let (old, new) = (pages(layout.size()), pages(new_size));
                lock();
                let h = unsafe { &mut *owner };
                let start = at as usize;
                let grown = if new <= old {
                    if new < old {
                        unsafe { give_pages(h, start + new * PAGE, old - new) };
                    }
                    true
                } else if start + old * PAGE == h.top && h.top + (new - old) * PAGE <= h.base + h.reserve
                    && !ADOPTED.load(Ordering::Relaxed)
                {
                    h.top += (new - old) * PAGE;
                    grew(h);
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

    /// A note section of NOTES, (name, type, desc).
    fn notes(notes: &[(&[u8], u32, &[u8])]) -> Vec<u8> {
        let mut out = vec![];
        for (name, kind, desc) in notes {
            out.extend((name.len() as u32).to_ne_bytes());
            out.extend((desc.len() as u32).to_ne_bytes());
            out.extend(kind.to_ne_bytes());
            out.extend(*name);
            out.resize(out.len().next_multiple_of(4), 0);
            out.extend(*desc);
            out.resize(out.len().next_multiple_of(4), 0);
        }
        out
    }

    #[test]
    fn a_build_id_is_read_from_its_note() {
        let id = [0xab, 0xcd, 0xef, 0x01, 0x23];
        let section = notes(&[(b"GNU\0", 1, b"abi-tag!"), (b"GNU\0", 3, &id)]);
        assert_eq!(build_id(&section), Some(&id[..]));
        assert_eq!(build_id(&notes(&[(b"FDO\0", 3, &id)])), None);
        assert_eq!(build_id(&section[..section.len() - 4]), None);
    }

    /// The build-id the file at PATH carries, read from its ELF section
    /// headers: what the mapping's note must agree with.
    fn file_build_id(path: &str) -> Option<Vec<u8>> {
        let bytes = std::fs::read(path).ok()?;
        let at = |o: usize, n: usize| u64::from_le_bytes(bytes[o..o + n].iter().chain([0u8; 8].iter()).take(8).copied().collect::<Vec<_>>().try_into().unwrap()) as usize;
        let (shoff, shentsize, shnum) = (at(0x28, 8), at(0x3a, 2), at(0x3c, 2));
        (0..shnum).find_map(|i| {
            let h = shoff + i * shentsize;
            (at(h + 4, 4) == 7).then(|| build_id(&bytes[at(h + 0x18, 8)..at(h + 0x18, 8) + at(h + 0x20, 8)]).map(<[u8]>::to_vec)).flatten()
        })
    }

    #[test]
    fn this_process_knows_libc_by_the_build_id_it_mapped() {
        let libraries = libraries().expect("an inventory");
        let libc = libraries.iter().find(|l| std::str::from_utf8(l.name).is_ok_and(|n| n.contains("libc.so"))).expect("libc is loaded");
        let path = std::str::from_utf8(libc.name).unwrap();
        assert_eq!(file_build_id(path).as_deref(), libc.build_id(), "{path}");
        let segments = libc.constant_segments().expect("readable");
        // Read-only data and code, as the file has them at those offsets.
        let file = std::fs::read(path).unwrap();
        for (offset, bytes) in segments {
            assert_eq!(&file[offset as usize..offset as usize + bytes.len()], bytes);
        }
    }

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
        assert!(!reserved(WINDOW.1 + CHUNK, PAGE) && !reserved(WINDOW.0 - PAGE, PAGE));
        // The parts a range is chosen from fit whole chunks, and FIRST is above BUILD.
        assert!(BUILD.0 % CHUNK == 0 && BUILD.1 == FIRST.0 && FIRST.0 % CHUNK == 0 && FIRST.1 == WINDOW.1 && RESERVE < CHUNK);
    }

    #[test]
    fn bins_order_runs_by_length() {
        assert_eq!((bin(1), bin(2), bin(3), bin(4), bin(1 << 50)), (0, 1, 1, 2, BINS - 1));
    }
}
