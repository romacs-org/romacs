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
//! found it so (`map_failure`). Everything else a world process maps stays
//! randomized, the executable too, and an image holds no address of the
//! executable or a library: no code, no static, no read-only data. A VM at
//! rest keeps no lowered code (`world_normalize`) and no host attachment
//! (`world_quiescent`); romacs-snapshot admits no function pointer and no
//! reference to a static in the VM's types; and a capture gives every
//! empty hash table, whose control bytes would be a static of the
//! executable, an allocation of its own (`image::settle`). So a restore
//! maps the image wherever this process's executable lies, and keeps the
//! range the builder's executable lay in inaccessible when nothing of this
//! process is there (`guard_executable`): an address left by mistake then
//! faults instead of reading what lies there. A taken range is left alone.
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
use map::{ALIGN, BUILD, FIRST, WINDOW};
pub use loaded::{executable_base, find_library, libraries, Library};
use loaded::executable;
mod loaded;
mod map;
use std::alloc::{GlobalAlloc, Layout, System};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicUsize, Ordering};

/// The most an arena reserves. Pages are committed only as they are
/// touched; under an address-space limit an arena reserves a quarter of it,
/// as a process may hold two (`adopt`).
const RESERVE: usize = 1 << 37;
const PAGE: usize = 4096;
const MAGIC: u64 = u64::from_le_bytes(*b"RWARENA4");
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
/// the fields a restore needs. Plain words throughout, with no padding, so
/// an image's header is read from its bytes (`map_image_fd`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
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
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ImageFields {
    /// The image's length: the used prefix of the arena.
    pub len: usize,
    /// The address of the boxed root the image was made for.
    pub root: usize,
    /// Process-global counters the image's contents depend on.
    pub counters: [u64; 4],
    /// Where the executable of the process that made it loaded: a restore
    /// keeps that range inaccessible when it can (`guard_executable`).
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
/// What became of the builder's executable range in the adopting process
/// (`Guard`).
static GUARD: AtomicU8 = AtomicU8::new(Guard::None as u8);
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

/// An arena's range, published once: its reserve is stored before its
/// base, so a reader that sees the base (Acquire) sees the reserve.
struct Slot {
    base: AtomicUsize,
    reserve: AtomicUsize,
}

impl Slot {
    const fn new() -> Self {
        Slot { base: AtomicUsize::new(0), reserve: AtomicUsize::new(0) }
    }

    /// (base, reserve), once published.
    #[inline]
    fn get(&self) -> Option<(usize, usize)> {
        let base = self.base.load(Ordering::Acquire);
        (base != 0).then(|| (base, self.reserve.load(Ordering::Relaxed)))
    }

    /// Publish the arena at BASE, RESERVE bytes, which is mapped read-write
    /// and stays mapped for the process's life. Once, under the lock.
    fn publish(&self, base: usize, reserve: usize) {
        self.reserve.store(reserve, Ordering::Relaxed);
        self.base.store(base, Ordering::Release);
    }

    /// Whether AT lies in this arena.
    #[inline]
    fn holds(&self, at: usize) -> bool {
        self.get().is_some_and(|(base, reserve)| (base..base + reserve).contains(&at))
    }

    /// The published arena, as a `Published` a lock may view.
    #[inline]
    fn published(&self) -> Option<Published> {
        self.get().map(|(base, reserve)| Published { base, reserve })
    }
}

/// An arena a slot published: BASE and RESERVE as `Slot::published`
/// read them, so a viewer of it (`Locked::heap`) need not read them again.
#[derive(Clone, Copy)]
struct Published {
    base: usize,
    reserve: usize,
}

impl Published {
    #[inline]
    fn holds(self, at: usize) -> bool {
        (self.base..self.base + self.reserve).contains(&at)
    }
}

/// The process's first arena, once it started.
static FIRST_ARENA: Slot = Slot::new();
/// The image this process adopted, once it did: the arena it allocates
/// from then on, in place of the first.
static IMAGE_ARENA: Slot = Slot::new();

/// The arena allocations come from, once one is published: the image,
/// once one is adopted.
#[inline]
fn current_arena() -> Option<Published> {
    IMAGE_ARENA.published().or_else(|| FIRST_ARENA.published())
}
/// A builder's arena is a shared mapping of this unlinked file, and an
/// image is copied from the file (`write_image`); unset elsewhere.
static BACKING: std::sync::OnceLock<OwnedFd> = std::sync::OnceLock::new();

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
    /// Its range could not be mapped.
    Map,
}

pub struct Arena;

/// The allocator's lock, held while this lives. One spin lock serializes
/// the allocator; whoever holds it may view an arena's state (`heap`).
struct Locked(());

impl Locked {
    #[inline]
    fn take() -> Self {
        let mut spins = 0u32;
        while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            spins += 1;
            if spins < 64 { std::hint::spin_loop() } else { std::thread::yield_now() }
        }
        Locked(())
    }

    /// The arena SLOT holds, once published (`view`).
    #[inline]
    fn heap(&mut self, slot: &Slot) -> Option<Heap<'_>> {
        slot.published().map(|arena| self.view(arena))
    }

    /// ARENA's header and the words of its blocks, for as long as this
    /// borrow of the lock lasts.
    #[inline]
    #[expect(clippy::unused_self, reason = "the borrow of the lock is what bounds the view")]
    fn view(&mut self, arena: Published) -> Heap<'_> {
        // UNSAFE(an arena is memory this module maps at an address it chooses; no type owns it)
        // SAFETY: a slot publishes only an arena mapped read-write for
        // RESERVE bytes from BASE, which stays mapped for the process's life,
        // with its header at BASE, initialized when it started or by the
        // image it maps; `Published` is made only from a slot. The lock,
        // which SELF holds and lends only through this borrow, makes this
        // the header's one reference.
        let header = unsafe { &mut *(arena.base as *mut Header) };
        Heap { header, start: arena.base, span: arena.reserve }
    }
}

impl Drop for Locked {
    #[inline]
    fn drop(&mut self) {
        LOCK.store(false, Ordering::Release);
    }
}

/// FORK, a fork(2), with the allocator's lock held, and the lock released
/// after it in each process it returns in: a child has the forking thread
/// alone, so a lock another thread held at the fork would stay held there.
/// FORK allocates nothing.
pub fn across_fork<T>(fork: impl FnOnce() -> T) -> T {
    let locked = Locked::take();
    let forked = fork();
    drop(locked);
    forked
}

/// The bytes of an arena its header's pages take.
const HEADER_BYTES: usize = size_of::<Header>().next_multiple_of(PAGE);

/// One arena's state, under the lock (`Locked::heap`): its header, and the
/// words this module keeps in its free blocks and free runs.
///
/// The words are written only by the free-list code below, which writes a
/// free block's first word and a free run's head and nothing a caller holds:
/// within this module, it keeps that invariant. Every access is checked to
/// lie in the arena past its header, and the process aborts otherwise: a
/// panic would allocate under the lock.
struct Heap<'l> {
    header: &'l mut Header,
    /// Where the arena starts, as its slot published it.
    start: usize,
    /// How many bytes it holds from there.
    span: usize,
}

impl std::ops::Deref for Heap<'_> {
    type Target = Header;
    fn deref(&self) -> &Header {
        self.header
    }
}

impl std::ops::DerefMut for Heap<'_> {
    fn deref_mut(&mut self) -> &mut Header {
        self.header
    }
}

impl Heap<'_> {
    /// Abort unless LEN bytes from AT lie in the arena past its header. AT
    /// below the header's end wraps to an offset past the room.
    fn check(&self, at: usize, len: usize) {
        let room = self.span.saturating_sub(HEADER_BYTES);
        let offset = at.wrapping_sub(self.start + HEADER_BYTES);
        if offset > room || len > room - offset {
            std::process::abort();
        }
    }

    /// Abort unless the word at AT lies in the arena past its header.
    fn check_word(&self, at: usize) {
        self.check(at, size_of::<usize>());
    }

    /// The word at AT, unchecked: the allocator's own path (`take_small`,
    /// `give_small`, `take_pages`, `push_run`). Checked, it cost 1.6% more
    /// instructions:u on a boot from source (17.12 G against 17.39 G, the
    /// binaries built from one checkout) and 1.2% more on a restored batch
    /// session, measured 2026-11-17.
    ///
    /// # Safety
    /// AT is a word this module keeps in the arena past its header: a free
    /// block's link or a word of a free run's head.
    // UNSAFE(a checked free-list read costs 1.6% instructions:u on a source boot, measured; see above)
    #[inline]
    #[expect(clippy::unused_self, reason = "a heap's borrow is what holds the lock across the read")]
    unsafe fn link(&self, at: usize) -> usize {
        // UNSAFE(the arena's free-list words are memory no type owns; see `Heap`)
        // SAFETY: by the contract, AT's word lies in the arena, mapped
        // read-write, and this module wrote it. Read unaligned, so no
        // alignment is assumed.
        unsafe { (at as *const usize).read_unaligned() }
    }

    /// Store VALUE at AT, unchecked, as `link` reads.
    ///
    /// # Safety
    /// AT is a word of a free block or free run in the arena past its
    /// header, which no caller holds.
    // UNSAFE(a checked free-list write costs as `link`'s read does, measured; see `link`)
    #[inline]
    #[expect(clippy::unused_self, reason = "a heap's borrow is what holds the lock across the write")]
    unsafe fn set_link(&mut self, at: usize, value: usize) {
        // UNSAFE(the arena's free-list words are memory no type owns; see `Heap`)
        // SAFETY: by the contract, AT's word lies in the arena, mapped
        // read-write, in a free block or run no caller holds. Written
        // unaligned, so no alignment is assumed.
        unsafe { (at as *mut usize).write_unaligned(value) }
    }

    /// The word at AT, which this module stored, checked: what a census,
    /// a trim or a scrub reads.
    fn word(&self, at: usize) -> usize {
        self.check_word(at);
        // UNSAFE(the arena's free-list words are memory no type owns; see `Heap`)
        // SAFETY: AT's word lies in the arena past its header (`check_word`).
        unsafe { self.link(at) }
    }

    /// Store VALUE at AT, checked, as `word` reads.
    fn set_word(&mut self, at: usize, value: usize) {
        self.check_word(at);
        // UNSAFE(the arena's free-list words are memory no type owns; see `Heap`)
        // SAFETY: AT's word lies in the arena past its header (`check_word`).
        unsafe { self.set_link(at, value) }
    }

    /// Zero LEN bytes from AT, in a free block or run.
    fn zero(&mut self, at: usize, len: usize) {
        self.check(at, len);
        // UNSAFE(the arena's free blocks are memory no type owns; see `Heap`)
        // SAFETY: the LEN bytes from AT lie in the arena, mapped read-write
        // (`check`), in a free block or run no caller holds.
        unsafe { std::ptr::write_bytes(at as *mut u8, 0, len) }
    }

    /// The free run whose head is at AT, checked.
    fn run(&self, at: usize) -> Run {
        Run { pages: self.word(at), next: self.word(at + 8), released: self.word(at + 16) != 0 }
    }

    /// Write RUN's head at AT, checked.
    fn set_run(&mut self, at: usize, run: Run) {
        self.set_word(at, run.pages);
        self.set_word(at + 8, run.next);
        self.set_word(at + 16, usize::from(run.released));
    }

    /// The free run whose head is at AT, unchecked, as `link` reads.
    ///
    /// # Safety
    /// AT is the head of a free run in the arena past its header.
    // UNSAFE(a checked read costs as `link`'s does, measured; see `link`)
    #[inline]
    unsafe fn run_at(&self, at: usize) -> Run {
        // UNSAFE(the arena's free-list words are memory no type owns; see `Heap`)
        // SAFETY: by the contract, AT and the two words after it are a free
        // run's head.
        unsafe { Run { pages: self.link(at), next: self.link(at + 8), released: self.link(at + 16) != 0 } }
    }

}

/// The class of a request of SIZE bytes aligned to ALIGN, or None for a run.
#[inline]
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

/// A free run's head, three words at its start (`Heap::run`): its length
/// in pages, the next run of its bin, and whether its pages but the
/// head's went back to the system.
#[derive(Clone, Copy)]
struct Run {
    pages: usize,
    next: usize,
    released: bool,
}

/// The bytes a run's head takes.
const RUN_HEAD: usize = 3 * size_of::<usize>();

#[inline]
fn push_run(heap: &mut Heap<'_>, at: usize, pages: usize, released: bool) {
    let next = heap.runs[bin(pages)];
    // UNSAFE(the allocator's own path takes no checked write, measured; see `Heap::link`)
    // SAFETY: AT starts PAGES free pages of the arena past its header, which
    // the caller gives up: a run's tail, a freed run or the gap at the top.
    unsafe {
        heap.set_link(at, pages);
        heap.set_link(at + 8, next);
        heap.set_link(at + 16, usize::from(released));
    }
    heap.runs[bin(pages)] = at;
}

/// PAGES pages aligned to ALIGN (a multiple of the page), from a free run
/// or the top. Null when the range is spent.
fn take_pages(heap: &mut Heap<'_>, pages: usize, align: usize) -> usize {
    if align <= PAGE {
        for b in bin(pages)..BINS {
            // The run before AT in the bin, None while AT is its first.
            let mut before = None;
            let mut at = heap.runs[b];
            let mut walked = 0;
            while at != 0 && walked < 32 {
                // UNSAFE(the allocator's own path takes no checked read, measured; see `Heap::link`)
                // SAFETY: AT is in bin B's list, which holds free run heads only.
                let run = unsafe { heap.run_at(at) };
                if run.pages >= pages {
                    match before {
                        None => heap.runs[b] = run.next,
                        // UNSAFE(the allocator's own path takes no checked write, measured; see `Heap::link`)
                        // SAFETY: BEFORE is the free run before AT in the same list.
                        Some(before) => unsafe { heap.set_link(before + 8, run.next) },
                    }
                    let rest = run.pages - pages;
                    if rest > 0 {
                        push_run(heap, at + pages * PAGE, rest, run.released);
                    }
                    return at;
                }
                before = Some(at);
                at = run.next;
                walked += 1;
            }
        }
    }
    let at = heap.top.next_multiple_of(align.max(PAGE));
    let end = at + pages * PAGE;
    if end > heap.base + heap.reserve {
        return 0;
    }
    if at > heap.top {
        let top = heap.top;
        push_run(heap, top, (at - top) / PAGE, false);
    }
    heap.top = end;
    at
}

fn give_pages(heap: &mut Heap<'_>, at: usize, pages: usize) {
    if at + pages * PAGE == heap.top {
        heap.top = at;
        trim_top(heap, TRIM_SLACK);
        return;
    }
    let released = pages >= RELEASE_PAGES;
    if released {
        release(heap, at + PAGE, (pages - 1) * PAGE);
    }
    push_run(heap, at, pages, released);
}

/// Record that the top grew: what lies below it is touched.
#[inline]
fn grew(heap: &mut Heap<'_>) {
    heap.zero = heap.zero.max(heap.top);
    heap.dirty = heap.dirty.max(heap.top);
}

/// Give back the pages above the top but for TRIM_KEEP, when more than
/// SLACK of them may be resident. The zero mark stays: a released page of
/// a restored image reads its file's bytes again, not zeros.
fn trim_top(heap: &mut Heap<'_>, slack: usize) {
    if heap.dirty > heap.top + slack {
        let keep = heap.top + TRIM_KEEP.min(slack);
        let dirty = heap.dirty;
        release(heap, keep, dirty - keep);
        heap.dirty = keep;
    }
}

#[inline]
fn take_small(heap: &mut Heap<'_>, class: usize) -> usize {
    let first = heap.free[class];
    if first != 0 {
        // UNSAFE(the allocator's own path takes no checked read, measured; see `Heap::link`)
        // SAFETY: HEAD is the first of CLASS's free blocks, whose first word
        // is the next one's address.
        heap.free[class] = unsafe { heap.link(first) };
        return first;
    }
    let size = class_size(class);
    if heap.cursor[class] + size > heap.end[class] {
        let slab = (64 * 1024).max(4 * size);
        let at = take_pages(heap, pages(slab), PAGE);
        if at == 0 {
            return 0;
        }
        heap.cursor[class] = at;
        heap.end[class] = at + pages(slab) * PAGE;
    }
    let at = heap.cursor[class];
    heap.cursor[class] += size;
    at
}

#[inline]
fn give_small(heap: &mut Heap<'_>, at: usize, class: usize) {
    let next = heap.free[class];
    // UNSAFE(the allocator's own path takes no checked write, measured; see `Heap::link`)
    // SAFETY: AT is a block of CLASS in this arena that its caller frees.
    unsafe { heap.set_link(at, next) };
    heap.free[class] = at;
}

/// The arena AT is in, or None for a block outside the arenas.
#[inline]
fn owner(at: *mut u8) -> Option<Published> {
    let at = at as usize;
    if let Some(image) = IMAGE_ARENA.published() && image.holds(at) {
        return Some(image);
    }
    FIRST_ARENA.published().filter(|first| first.holds(at))
}

fn state() -> u8 {
    match STATE.load(Ordering::Acquire) {
        UNSET => {
            let mut locked = Locked::take();
            if STATE.load(Ordering::Relaxed) == UNSET {
                let state = start(&mut locked);
                STATE.store(state, Ordering::Release);
            }
            drop(locked);
            STATE.load(Ordering::Acquire)
        }
        state => state,
    }
}

/// Whether the range at AT, LEN bytes, is within this process's
/// reservation of the window (`map::reserve_window`).
pub fn reserved(at: usize, len: usize) -> bool {
    state();
    map::within_reservation(at, len)
}

/// Reserve a fresh range at a random address in the window: a builder's
/// in BUILD, where images are made, any other process's first arena in
/// FIRST.
fn start(locked: &mut Locked) -> u8 {
    loaded::take_inventory();
    map::reserve_window();
    // HOST(libc::getenv): the world allocator reads whether it runs in a builder at its first
    // allocation, before host.rs can run; it zeroes freed blocks and backs its arena with a file
    // UNSAFE(std's env reads allocate, inside the allocator's first allocation: getenv is a C call)
    // SAFETY: the name is a C string literal; getenv's result is only
    // tested for null, and no thread sets the environment this early.
    let building = !unsafe { libc::getenv(c"ROMACS_WORLD_BUILD".as_ptr()) }.is_null();
    SCRUB.store(building, Ordering::Relaxed);
    let Some(reserve) = reserve() else { return SYSTEM };
    let mut backing = if building { backing_file(reserve) } else { None };
    let (low, high) = if building { BUILD } else { FIRST };
    for _ in 0..8 {
        let mut random = [0u8; size_of::<usize>()];
        // HOST(rustix::query): the world allocator picks a random arena address at its first
        // allocation; no Lisp value depends on it
        if rustix::rand::getrandom(&mut random, rustix::rand::GetRandomFlags::empty()) != Ok(random.len()) {
            break;
        }
        let random = usize::from_ne_bytes(random);
        let base = map::base_in(low, high, reserve, random);
        let of = match &backing {
            Some(fd) => map::Of::File { fd: fd.as_fd(), shared: true },
            None => map::Of::Memory,
        };
        if let Ok(mapping) = map::map(&map::place(base, reserve), base, reserve, &of) {
            mapping.keep();
            if let Some(backing) = backing.take() {
                // Set once: STATE starts the arena once, under the lock.
                let _ = BACKING.set(backing);
            }
            FIRST_ARENA.publish(base, reserve);
            let Some(mut heap) = locked.heap(&FIRST_ARENA) else { return SYSTEM };
            heap.magic = MAGIC;
            heap.base = base;
            heap.reserve = reserve;
            heap.top = base + HEADER_BYTES;
            heap.zero = heap.top;
            heap.dirty = heap.top;
            return ARENA;
        }
    }
    SYSTEM
}

/// A builder's backing file: unlinked, in memory, RESERVE bytes long and
/// sparse, so only pages the arena touched hold blocks. None when there is
/// none: a builder without one cannot publish (`write_image`). The path is
/// a C string, so the open allocates nothing.
fn backing_file(reserve: usize) -> Option<OwnedFd> {
    let flags = rustix::fs::OFlags::TMPFILE | rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC;
    // HOST(rustix::file): a builder backs its arena with an unlinked memory file at its first
    // allocation, before host.rs can run; no Lisp value depends on it
    let fd = rustix::fs::open(c"/dev/shm", flags, rustix::fs::Mode::from_raw_mode(0o600)).ok()?;
    // HOST(rustix::file): a builder backs its arena with an unlinked memory file at its first
    // allocation, before host.rs can run; no Lisp value depends on it
    rustix::fs::ftruncate(&fd, reserve as u64).ok()?;
    Some(fd)
}

/// Give the pages from AT, LEN bytes, back to the system. Their content is
/// the allocator's no longer: memory below the zero mark is zeroed when a
/// zeroed block is asked for.
fn release(heap: &Heap<'_>, at: usize, len: usize) {
    heap.check(at, len);
    if let Some(backing) = BACKING.get().filter(|_| in_backing(at)) {
        let offset = (at - heap.base) as u64;
        let punch = rustix::fs::FallocateFlags::PUNCH_HOLE | rustix::fs::FallocateFlags::KEEP_SIZE;
        let _ = rustix::fs::fallocate(backing, punch, offset, len as u64);
    } else {
        // UNSAFE(madvise on memory this module maps has no safe wrapper: rustix's is unsafe)
        // SAFETY: the LEN bytes from AT are free pages of the arena
        // (`check`, and the free-list code's invariant), which nothing reads.
        unsafe { libc::madvise(at as *mut libc::c_void, len, libc::MADV_DONTNEED) };
    }
}

/// Whether AT is in the arena the backing file maps: a builder's only one.
fn in_backing(at: usize) -> bool {
    IMAGE_ARENA.get().is_none() && FIRST_ARENA.holds(at)
}

/// How much address space an arena of this process reserves: RESERVE, or
/// a GiB-aligned quarter of the address-space limit, when that is at least
/// a GiB.
fn reserve() -> Option<usize> {
    let Ok(Some(limit)) = address_space_limit() else { return Some(RESERVE) };
    let quarter = (usize::try_from(limit).unwrap_or(usize::MAX) / 4).min(RESERVE) / ALIGN * ALIGN;
    (quarter >= ALIGN).then_some(quarter)
}

/// The process's address-space limit (`RLIMIT_AS`), None for none; an
/// error when the query is refused (a syscall filter may refuse it).
fn address_space_limit() -> std::io::Result<Option<u64>> {
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // UNSAFE(rustix's getrlimit takes the call as infallible, and a refused one would read garbage)
    // SAFETY: getrlimit writes only the local it is given.
    if unsafe { libc::getrlimit(libc::RLIMIT_AS, &raw mut limit) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((limit.rlim_cur != libc::RLIM_INFINITY).then_some(limit.rlim_cur))
}

/// Allocate from the image in FILE from now on: map it where it was made,
/// over this process's reservation of the window, or where nothing is when
/// the window is not reserved (`place`), when it is a whole image of this
/// format. Once per process; the first arena keeps the blocks allocated
/// before. Refused, `map_failure` says at which step and why.
pub fn adopt(file: &std::fs::File) -> Restore {
    if state() != ARENA {
        return refuse(1, 0, false);
    }
    // The first image stays this process's: what became of it stays said.
    if IMAGE_ARENA.get().is_some() {
        return refuse(2, 0, false);
    }
    let Some(outcome) = map_image_fd(file.as_fd()) else { return refuse(2, 0, false) };
    // A failure is said only while no image is adopted: one adopted at the
    // same time by another thread keeps what became of it. Under the lock
    // its publication took, so the two cannot interleave.
    let locked = Locked::take();
    if outcome == Restore::Mapped || IMAGE_ARENA.get().is_none() {
        RESTORE.store(outcome as u8, Ordering::Release);
    }
    drop(locked);
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


/// What refused the image's range, once `adopt` has (`Restore::Map`): the
/// step, the error number of the moment, 0 for none, and whether the range
/// was within this process's reservation of the window.
pub fn map_failure() -> Option<(&'static str, i32, bool)> {
    let step = MAP_STEP.load(Ordering::Acquire);
    (step != 0).then(|| (STEPS[step as usize], MAP_ERRNO.load(Ordering::Relaxed), MAP_RESERVED.load(Ordering::Relaxed)))
}


/// FD's image mapped where it was made, as `adopt` says; None when another
/// thread adopted one first, whose outcome stays said.
fn map_image_fd(fd: BorrowedFd<'_>) -> Option<Restore> {
    let mut bytes = [0u8; size_of::<Header>()];
    let read = rustix::io::pread(fd, &mut bytes, 0);
    // HOST(rustix::file): the world allocator checks an image is whole before it adopts it; the
    // launcher checked that image
    let stat = rustix::fs::fstat(fd);
    let (Ok(read), Ok(stat)) = (read, stat) else { return Some(Restore::Unreadable) };
    if read != bytes.len() {
        return Some(Restore::Unreadable);
    }
    let image: Header = bytemuck::pod_read_unaligned(&bytes);
    let (base, len) = (image.base, image.image.len);
    // A truncated or extended file is no image: mapped, it would fault.
    if image.magic != MAGIC || len == 0 || len % PAGE != 0 || stat.st_size as usize != len
        || base % ALIGN != 0 || base < WINDOW.0 || base.saturating_add(len) > WINDOW.1 || !(base..=base + len).contains(&image.top)
    {
        return Some(Restore::Format);
    }
    let reserve = match reserve() {
        Some(reserve) if len < reserve && base + reserve <= WINDOW.1 => reserve,
        _ => return Some(refuse(3, 0, false)),
    };
    // From here to publication under the lock: a thread adopting at the
    // same time finds the image published, never a placement that
    // replaces it. Nothing below allocates.
    let mut locked = Locked::take();
    if IMAGE_ARENA.get().is_some() {
        return None;
    }
    // Over the reservation, unless the range reaches this process's first
    // arena (an image made under another address-space limit could): then
    // only where nothing is, which refuses it (`map::place`). What a
    // failure leaves is given up: only what this module mapped, never what
    // refused it, which the launch's source boot may be using.
    let place = map::place(base, reserve);
    let mapped = match map::map(&place, base, len, &map::Of::File { fd, shared: false }) {
        Ok(mapped) => mapped,
        Err(refused) => return Some(refuse(4, refused.errno, refused.over_reservation)),
    };
    match map::map(&place, base + len, reserve - len, &map::Of::Memory) {
        Ok(rest) => rest.keep(),
        Err(refused) => {
            mapped.undo();
            return Some(refuse(5, refused.errno, refused.over_reservation));
        }
    }
    // The kernel reads the file ahead while the process starts.
    mapped.will_need();
    mapped.keep();
    IMAGE_ARENA.publish(base, reserve);
    if let Some(mut heap) = locked.heap(&IMAGE_ARENA) {
        // This process's reservation, not the builder's.
        heap.reserve = reserve;
    }
    ADOPTED.store(true, Ordering::Release);
    drop(locked);
    GUARD.store(guard_executable(image.image.executable) as u8, Ordering::Release);
    start_heap_file(reserve);
    Some(Restore::Mapped)
}

/// With `ROMACS_HEAP_FILE=1`, start the file heap, LEN bytes as an arena
/// has, for the calling thread: the one that adopted the image, which runs
/// the VM.
fn start_heap_file(len: usize) {
    // HOST(std::env::var_os): whether the file heap is asked for (ROMACS_HEAP_FILE), read once as
    // the image is adopted; not through host.rs, so that a process that asks for none records no
    // host read; no world holds it, and Lisp sees it only through romacs--arena-census
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

/// What a restore made of the range the builder's executable lay in. An
/// image holds no address there (`image::settle`, `Schema::renew`); should
/// one be left, following it faults where the range is reserved, rather
/// than reading what this process has there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Guard {
    /// No image was adopted.
    None = 0,
    /// Reserved, inaccessible.
    Reserved,
    /// Something of this process lies there, its own executable perhaps:
    /// left alone, the image restores.
    Taken,
    /// This process's executable could not be read.
    Unknown,
}

/// What became of the builder's executable range (`Guard`).
pub fn executable_guard() -> Guard {
    match GUARD.load(Ordering::Acquire) {
        1 => Guard::Reserved,
        2 => Guard::Taken,
        3 => Guard::Unknown,
        _ => Guard::None,
    }
}

/// Reserve, inaccessible, the range the executable of the process that made
/// the image lay in at OLD (the same binary: the image's key binds it),
/// when nothing of this process lies there.
fn guard_executable(old: usize) -> Guard {
    let Some((_, span)) = executable() else { return Guard::Unknown };
    let (at, len) = (old + span.start, span.end - span.start);
    match map::map(&map::place(at, len), at, len, &map::Of::Nothing) {
        Ok(guard) => {
            guard.keep();
            Guard::Reserved
        }
        Err(_) => Guard::Taken,
    }
}

/// How the image this process adopted fared.
pub fn restore_outcome() -> Restore {
    match RESTORE.load(Ordering::Acquire) {
        1 => Restore::Mapped,
        2 => Restore::Unreadable,
        3 => Restore::Format,
        4 => Restore::Map,
        _ => Restore::None,
    }
}

/// Whether the heap is an arena whose used prefix will be an image: a
/// builder's, the only heap `write_image` will write. What goes into it goes
/// into the world, so nothing that depends on the moment belongs there.
pub fn heap_is_image() -> bool {
    BACKING.get().is_some() && !ADOPTED.load(Ordering::Acquire)
}

/// Whether the arena is a shared mapping (a builder's, `backing_file`): a
/// forked child then shares its blocks with the parent instead of copying them.
pub fn heap_is_shared() -> bool {
    BACKING.get().is_some()
}

/// Whether a heap table may take its one large reservation (`table.rs`): not
/// where the heap is an image, nor under an address-space limit, which the
/// reservation would eat into.
pub fn may_reserve() -> bool {
    if heap_is_image() {
        return false;
    }
    matches!(address_space_limit(), Ok(None))
}

/// Give the system the whole pages between FROM and TO bytes past BASE, the
/// spare capacity of a reserved table, whose contents are unused: the
/// pages come back zeroed, or, in the file heap (`start_heap_file`), as
/// the heap's file last held them.
///
/// # Safety
/// Those bytes are within one allocation the caller owns, and nothing reads
/// or holds them.
// UNSAFE(only the caller knows the pages are its own and unused)
pub(crate) unsafe fn release_spare(base: *mut u8, from: usize, to: usize) {
    let start = (base as usize + from).next_multiple_of(PAGE);
    let end = (base as usize + to) / PAGE * PAGE;
    if end > start {
        // UNSAFE(madvise on memory has no safe wrapper: rustix's is unsafe)
        // SAFETY: this function's contract: these whole pages lie in the
        // caller's allocation, and nothing reads or holds them.
        unsafe { libc::madvise(start as *mut libc::c_void, end - start, libc::MADV_DONTNEED) };
    }
}

/// Whether this process's heap is the arena.
pub fn active() -> bool {
    state() == ARENA
}

/// Whether this process's global allocator is the arena: a new block lies
/// in its range. The arena exists in any process that asks for it, whatever
/// allocator serves the heap, so `active` alone does not tell.
#[must_use]
pub fn serves_heap() -> bool {
    let block = Box::new(0_u8);
    let at = std::ptr::from_ref(&*block) as usize;
    current_range().is_some_and(|(base, len)| (base..base + len).contains(&at))
}

/// Whether the arena already serves this process, without starting one:
/// a process whose global allocator is another never maps it.
fn started() -> bool {
    STATE.load(Ordering::Acquire) == ARENA
}

/// The range the arena's allocations come from: where it starts and how
/// much address space it holds. None when the heap is the system's.
pub fn current_range() -> Option<(usize, usize)> {
    if active() { current_arena().map(|arena| (arena.base, arena.reserve)) } else { None }
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
#[must_use]
pub fn census() -> Option<Census> {
    census_of(&[&IMAGE_ARENA, &FIRST_ARENA])
}

/// The census of the image this process mapped alone, as it was published
/// and as it is now: without the arena the process started with.
#[must_use]
pub fn image_census() -> Option<Census> {
    if restore_outcome() != Restore::Mapped {
        return None;
    }
    census_of(&[&IMAGE_ARENA])
}

/// The census of the arenas ARENAS hold.
fn census_of(arenas: &[&Slot]) -> Option<Census> {
    if !started() {
        return None;
    }
    let mut c = Census { classes: vec![(0, 0); CLASSES], ..Census::default() };
    let mut locked = Locked::take();
    for arena in arenas {
        if let Some(heap) = locked.heap(arena) {
            count(&heap, &mut c);
        }
    }
    drop(locked);
    for (class, row) in c.classes.iter_mut().enumerate() {
        row.0 = class_size(class);
    }
    c.small_free = c.classes.iter().map(|&(_, free)| free).sum();
    c.classes.retain(|&(_, free)| free > 0);
    Some(c)
}

/// Add HEAP's free lists to C.
fn count(heap: &Heap<'_>, c: &mut Census) {
    c.used += heap.top - heap.base;
    c.above_top += heap.dirty.saturating_sub(heap.top);
    for class in 0..CLASSES {
        let mut at = heap.free[class];
        while at != 0 {
            c.classes[class].1 += class_size(class);
            at = heap.word(at);
        }
        c.slab_rest += heap.end[class] - heap.cursor[class];
    }
    for b in 0..BINS {
        let mut at = heap.runs[b];
        while at != 0 {
            let run = heap.run(at);
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
    let mut locked = Locked::take();
    for arena in [&IMAGE_ARENA, &FIRST_ARENA] {
        let Some(mut heap) = locked.heap(arena) else { continue };
        trim_top(&mut heap, 0);
        for b in 0..BINS {
            let mut at = heap.runs[b];
            while at != 0 {
                let run = heap.run(at);
                if !run.released && run.pages > 1 {
                    release(&heap, at + PAGE, (run.pages - 1) * PAGE);
                    heap.set_run(at, Run { released: true, ..run });
                }
                at = run.next;
            }
        }
    }
    drop(locked);
}

/// The fields of the image this process mapped.
pub fn mapped_image() -> Option<ImageFields> {
    if restore_outcome() != Restore::Mapped {
        return None;
    }
    Locked::take().heap(&IMAGE_ARENA).map(|heap| heap.image)
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
// UNSAFE(only the caller knows what FIELDS.root holds, which the reader takes)
pub unsafe fn write_image(file: &std::fs::File, mut fields: ImageFields) -> std::io::Result<usize> {
    if !active() {
        return Err(std::io::Error::other("the arena is not this process's heap"));
    }
    let Some(backing) = BACKING.get().filter(|_| IMAGE_ARENA.get().is_none()) else {
        return Err(std::io::Error::other("only a builder's file-backed arena can be written as an image"));
    };
    let mut locked = Locked::take();
    let Some(mut heap) = locked.heap(&FIRST_ARENA) else {
        return Err(std::io::Error::other("the arena is not this process's heap"));
    };
    scrub(&mut heap);
    fields.len = heap.top - heap.base;
    let saved = heap.image;
    heap.image = fields;
    let result = copy_extents(backing.as_fd(), file.as_fd(), fields.len);
    heap.image = saved;
    drop(locked);
    result.map(|()| fields.len)
}

/// Zero what free blocks still hold, but for their links, and give free
/// runs' pages back: freed bytes are no part of a world, and a released
/// page is a hole in its file.
fn scrub(heap: &mut Heap<'_>) {
    for class in 0..CLASSES {
        let size = class_size(class);
        let mut at = heap.free[class];
        while at != 0 {
            heap.zero(at + 8, size - 8);
            at = heap.word(at);
        }
    }
    for b in 0..BINS {
        let mut at = heap.runs[b];
        while at != 0 {
            let run = heap.run(at);
            heap.zero(at + RUN_HEAD, PAGE - RUN_HEAD);
            if run.pages > 1 {
                release(heap, at + PAGE, (run.pages - 1) * PAGE);
                heap.set_run(at, Run { released: true, ..run });
            }
            at = run.next;
        }
    }
}

/// Copy the data extents of FROM's first LEN bytes to the same offsets
/// of TO, and make TO LEN bytes long: its holes stay holes.
fn copy_extents(from: BorrowedFd<'_>, to: BorrowedFd<'_>, len: usize) -> std::io::Result<()> {
    let len = len as u64;
    let mut at = 0u64;
    while at < len {
        let data = match rustix::fs::seek(from, rustix::fs::SeekFrom::Data(at)) {
            Ok(data) => data,
            Err(rustix::io::Errno::NXIO) => break,
            Err(error) => return Err(error.into()),
        };
        if data >= len {
            break;
        }
        let end = rustix::fs::seek(from, rustix::fs::SeekFrom::Hole(data))?.min(len);
        copy_range(from, to, data, end)?;
        at = end;
    }
    // HOST(rustix::file): copies the builder arena's backing file into the image file the builder
    // opened through host
    Ok(rustix::fs::ftruncate(to, len)?)
}

/// Bytes START..END of FROM to the same offsets of TO: by the kernel, or
/// through a buffer of this module's where it cannot copy between them.
/// The caller holds the allocator's lock, so nothing here may allocate.
fn copy_range(from: BorrowedFd<'_>, to: BorrowedFd<'_>, start: u64, end: u64) -> std::io::Result<()> {
    static BUFFER: std::sync::Mutex<[u8; 1 << 20]> = std::sync::Mutex::new([0; 1 << 20]);
    let ended = || std::io::Error::other("the backing file ended early");
    let mut at = start;
    while at < end {
        let (mut input, mut output) = (at, at);
        let want = usize::try_from(end - at).unwrap_or(usize::MAX);
        // HOST(rustix::file): copies the builder arena's backing file into the image file the builder
        // opened through host
        match rustix::fs::copy_file_range(from, Some(&mut input), to, Some(&mut output), want) {
            Ok(0) => return Err(ended()),
            Ok(n) => {
                at += n as u64;
                continue;
            }
            Err(rustix::io::Errno::XDEV | rustix::io::Errno::INVAL | rustix::io::Errno::NOSYS | rustix::io::Errno::OPNOTSUPP) => {}
            Err(error) => return Err(error.into()),
        }
        let mut buffer = BUFFER.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let want = buffer.len().min(want);
        let read = rustix::io::pread(from, &mut buffer[..want], at)?;
        if read == 0 {
            return Err(ended());
        }
        let mut done = 0;
        while done < read {
            done += rustix::io::pwrite(to, &buffer[done..read], at + done as u64)?;
        }
        at += read as u64;
    }
    Ok(())
}

/// A block for LAYOUT from the current arena, 0 when it is spent, with the
/// zero mark as it was: memory at or above it was never handed out.
#[inline(always)]
#[expect(clippy::inline_always, reason = "every allocation runs it: plain #[inline] cost 0.95% more instructions:u on a boot from source, 17.65 G against 17.49 G")]
fn allocate(layout: Layout) -> (usize, usize) {
    let mut locked = Locked::take();
    let Some(arena) = current_arena() else { return (0, 0) };
    let mut heap = locked.view(arena);
    let zero = heap.zero;
    let at = match class_of(layout.size(), layout.align()) {
        Some(class) => take_small(&mut heap, class),
        None => take_pages(&mut heap, pages(layout.size()), layout.align().max(PAGE)),
    };
    grew(&mut heap);
    (at, zero)
}

/// Give the block at AT, of LAYOUT, back to ARENA, the arena it lies in.
#[inline(always)]
#[expect(clippy::inline_always, reason = "every allocation runs it: plain #[inline] cost 0.95% more instructions:u on a boot from source, 17.65 G against 17.49 G")]
fn free(arena: Published, at: usize, layout: Layout) {
    let mut locked = Locked::take();
    let mut heap = locked.view(arena);
    match class_of(layout.size(), layout.align()) {
        Some(class) => give_small(&mut heap, at, class),
        None => give_pages(&mut heap, at, pages(layout.size())),
    }
}

/// Resize the page run at AT, of OLD pages, in ARENA to NEW pages where it
/// lies: shrunk in place, or grown at the top. Whether it was.
fn resize_run(arena: Published, at: usize, old: usize, new: usize) -> bool {
    let mut locked = Locked::take();
    let mut heap = locked.view(arena);
    if new <= old {
        if new < old {
            give_pages(&mut heap, at + new * PAGE, old - new);
        }
        return true;
    }
    if at + old * PAGE == heap.top && heap.top + (new - old) * PAGE <= heap.base + heap.reserve
        && !ADOPTED.load(Ordering::Relaxed)
    {
        heap.top += (new - old) * PAGE;
        grew(&mut heap);
        return true;
    }
    false
}

// UNSAFE(GlobalAlloc is an unsafe trait: the allocator promises its blocks are valid and disjoint)
// SAFETY: blocks come from the free-list code (`Heap`), which hands out
// each block once, aligned and sized for its layout, and takes back only
// what a caller frees; blocks outside the arenas go to the allocator that
// made them (`PLATFORM` once an image is adopted, the system's before).
unsafe impl GlobalAlloc for Arena {
    // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
    // SAFETY: the caller's contract is GlobalAlloc::alloc's.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ADOPTED.load(Ordering::Relaxed) {
            // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
            // SAFETY: LAYOUT is the caller's, passed on unchanged.
            return unsafe { PLATFORM.alloc(layout) };
        }
        if state() != ARENA {
            // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
            // SAFETY: LAYOUT is the caller's, passed on unchanged.
            return unsafe { System.alloc(layout) };
        }
        allocate(layout).0 as *mut u8
    }

    // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
    // SAFETY: the caller's contract is GlobalAlloc::alloc_zeroed's.
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if ADOPTED.load(Ordering::Relaxed) {
            // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
            // SAFETY: LAYOUT is the caller's, passed on unchanged.
            return unsafe { PLATFORM.alloc_zeroed(layout) };
        }
        if state() != ARENA {
            // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
            // SAFETY: LAYOUT is the caller's, passed on unchanged.
            return unsafe { System.alloc_zeroed(layout) };
        }
        let (at, zero) = allocate(layout);
        // Memory at or above the old zero mark was never handed out.
        if at != 0 && at < zero {
            // UNSAFE(a new block is raw memory until it is returned; no type owns it)
            // SAFETY: AT is the block just taken for LAYOUT, LAYOUT.size()
            // bytes this call alone holds.
            unsafe { std::ptr::write_bytes(at as *mut u8, 0, layout.size().min(zero - at)) };
        }
        at as *mut u8
    }

    // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
    // SAFETY: the caller's contract is GlobalAlloc::dealloc's.
    unsafe fn dealloc(&self, at: *mut u8, layout: Layout) {
        let Some(arena) = owner(at) else {
            // A block outside the arenas is the production allocator's in a
            // process that adopted an image (every earlier block is an
            // arena's), else the system's (an arena that never started).
            // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
            // SAFETY: AT and LAYOUT are the caller's, passed to the allocator that made the block.
            return unsafe {
                if ADOPTED.load(Ordering::Acquire) { PLATFORM.dealloc(at, layout) } else { System.dealloc(at, layout) }
            };
        };
        if SCRUB.load(Ordering::Relaxed) {
            // UNSAFE(a freed block is the caller's memory until this returns; no type owns it)
            // SAFETY: by dealloc's contract, AT holds LAYOUT.size() bytes the caller gives up.
            unsafe { std::ptr::write_bytes(at, 0, layout.size()) };
        }
        free(arena, at as usize, layout);
    }

    // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
    // SAFETY: the caller's contract is GlobalAlloc::realloc's.
    unsafe fn realloc(&self, at: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let Some(arena) = owner(at) else {
            // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
            // SAFETY: the arguments are the caller's, passed to the allocator that made the block.
            return unsafe {
                if ADOPTED.load(Ordering::Acquire) { PLATFORM.realloc(at, layout, new_size) } else { System.realloc(at, layout, new_size) }
            };
        };
        let align = layout.align();
        if SCRUB.load(Ordering::Relaxed) && new_size < layout.size() {
            // UNSAFE(a block's tail is the caller's memory; no type owns it)
            // SAFETY: by realloc's contract, AT holds LAYOUT.size() bytes,
            // and the bytes past NEW_SIZE are given up.
            unsafe { std::ptr::write_bytes(at.add(new_size), 0, layout.size() - new_size) };
        }
        match (class_of(layout.size(), align), class_of(new_size, align)) {
            (Some(old), Some(new)) if old == new => return at,
            (None, None) if align <= PAGE && resize_run(arena, at as usize, pages(layout.size()), pages(new_size)) => return at,
            _ => {}
        }
        let Ok(new_layout) = Layout::from_size_align(new_size, align) else { return std::ptr::null_mut() };
        // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
        // SAFETY: NEW_LAYOUT has a nonzero size, as realloc's contract gives NEW_SIZE.
        let moved = unsafe { self.alloc(new_layout) };
        if !moved.is_null() {
            // UNSAFE(copying between two raw blocks has no safe form)
            // SAFETY: AT holds LAYOUT.size() bytes, MOVED holds NEW_SIZE,
            // and the two are different blocks.
            unsafe { std::ptr::copy_nonoverlapping(at, moved, layout.size().min(new_size)) };
            // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
            // SAFETY: AT was allocated with LAYOUT, and is now moved.
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
