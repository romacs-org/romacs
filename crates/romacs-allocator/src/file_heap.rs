//! The file heap: allocator memory that is a private mapping of a memory
//! file, so that the heap at one instant is a file other processes can map.
//!
//! One thread, the heap's writer, the one that calls `start`, allocates
//! from a jemalloc arena of its own. That arena takes its extents from here:
//! spans of one address range reserved by `start`, each a `MAP_PRIVATE`
//! mapping of the file at the span's offset in the range. A write to such a
//! page is copied on write by the kernel and stays this process's; the file
//! keeps what it held. A **fold** has the kernel copy the pages written
//! since the last fold into the file (`fold`), after which the file is the
//! heap, byte for byte. The written pages are read from page-table bits: the range is
//! registered for userfaultfd's asynchronous write-protect, and one
//! `PAGEMAP_SCAN` lists the written pages and protects them again. A page
//! never touched is neither listed nor protected, so a reservation costs
//! nothing until used.
//!
//! **Every fork is the kernel's own unless its caller asks otherwise.** The
//! range is inherited like any memory, so a fork from any thread, a spawn
//! and a raw clone all give the child what they always gave it. (A raw
//! clone runs no fork handler, so its child is not counted a reader: a page
//! the parent has not written since a fold, it reads from the file, which
//! the next fold may change. It must exec first, as `Command`'s does while
//! its parent waits.)
//!
//! **A lazy fork** is one the writer asks for (`lazily`). It folds first;
//! the range is left out of that one fork, which then copies none of its
//! page-table entries; and the child maps the file where the range was,
//! before anything touches it. Its entries fill as it reads. What the
//! writer itself wrote, such a child sees exactly as it was at the fork.
//! What another thread was writing into one of the writer's blocks during
//! the call, it may see as of an earlier instant in the call, and not one
//! instant per page as a real fork shows it: a child that depends on other
//! threads' state must not be forked lazily. (Other threads do write the
//! writer's blocks, a shared count dropped elsewhere for one, and can come
//! to own blocks of this arena: a block one of them frees stays in its
//! cache, and may serve its next request. So the fold's copy is the
//! kernel's, which may read memory that is changing; no code here reads a
//! page of the heap.) A fork handler registered before `start`
//! runs after the fold in the parent and before the mapping in the child:
//! it must not touch the writer's blocks. A fork is lazy only with no reader
//! alive; otherwise, or when the kernel refuses the scan, it is the kernel's
//! own, and says so (`Stats`).
//!
//! A child, lazy or not, is a **reader** of the file, and so is every
//! process forked from it, until it exits or execs: what it has not touched
//! it still reads from the file. While a reader lives the file does not
//! change: no fold runs, so no fork is lazy; and a purge drops this
//! process's copy of the pages at once but punches the file only with no
//! reader left, and tells the allocator which happened, since only a
//! punched range reads back as zeros. The kernel keeps the count of
//! readers: each fork leaves the child the write end of a pipe, closed at
//! its exit or exec and inherited by its own children, and the read end
//! here shows when the last holder is gone. A child never writes or punches
//! the file, and takes its own fresh extents from anonymous memory.
//!
//! Any thread may free the writer's blocks, as with any jemalloc arena, and
//! a purge may run on any thread: it never waits for this module's lock.
//! Other threads allocate from the default arenas, anonymous as before. A
//! scan the kernel refuses, or a span it will not watch, ends folding for
//! good (`Stats::failed_folds`): the walk has protected pages it never
//! listed, so the file can no longer be made the heap, and from then on no
//! fork is lazy. The range is all the writer's arena has: spent, its
//! allocations fail. Off (`start` never called, or refused) none of this
//! exists and every path is the old one.
use std::cell::Cell;
use std::ffi::c_void;
use std::fs::File;
use std::io::{PipeReader, PipeWriter};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use rustix::fs::{FallocateFlags, MemfdFlags};
use tikv_jemalloc_sys as jemalloc;

const PAGE: usize = 4096;
/// Readers tracked at once; past that, one more makes the file fixed for good.
const CHILDREN: usize = 256;

/// Why the file heap did not start. Each leaves the process as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// It has started already.
    Started,
    /// The length is not whole pages.
    Range,
    /// The fork handlers, the memory file, the page map, userfaultfd or the
    /// range: the step and errno.
    System(&'static str, i32),
    /// jemalloc would not make the arena, or bind the calling thread to it.
    Arena(i32),
}

/// What a fold copied into the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fold {
    pub pages: usize,
    pub runs: usize,
}

/// The heap's counts so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Bytes of the range handed to the arena.
    pub mapped: usize,
    pub folds: u64,
    pub folded_pages: u64,
    /// Bytes purged that left the file, and bytes purged while a reader kept them in it.
    pub punched: u64,
    pub kept: u64,
    /// Forked children, and theirs, that may still read the file;
    /// `usize::MAX` once one was forked that nothing tracks.
    pub readers: usize,
    /// Forks whose child mapped the file, and forks that were the kernel's own.
    pub lazy_forks: u64,
    pub plain_forks: u64,
    /// Scans or watches the kernel refused. After the first, no fold runs and no fork is lazy.
    pub failed_folds: u64,
}

/// What `start` made, for the process's life.
struct Heap {
    base: usize,
    limit: usize,
    file: File,
    watch: OwnedFd,
    pagemap: File,
}
static HEAP: OnceLock<Heap> = OnceLock::new();
static STARTING: AtomicBool = AtomicBool::new(false);
static HANDLERS: AtomicBool = AtomicBool::new(false);
/// The first byte of the range not yet handed out.
static TOP: AtomicUsize = AtomicUsize::new(0);
thread_local! {
    /// Whether this thread is the heap's writer.
    static WRITES: Cell<bool> = const { Cell::new(false) };
    /// Whether this thread's fork in progress was asked to be lazy (`lazily`).
    static ASKED: Cell<bool> = const { Cell::new(false) };
}
/// This process is a forked child: it reads its parent's file and never writes it.
static CHILD: AtomicBool = AtomicBool::new(false);
/// The read ends of the pipes whose write ends forked children hold.
static READERS: Mutex<[Option<PipeReader>; CHILDREN]> = Mutex::new([const { None }; CHILDREN]);
/// A child was forked that no pipe tracks: the file never changes again.
static LOST: AtomicBool = AtomicBool::new(false);
/// Held by a fold, a purge that punches, and across every fork: the file changes under it alone.
static LOCK: AtomicBool = AtomicBool::new(false);
static FOLDS: AtomicU64 = AtomicU64::new(0);
static FOLDED: AtomicU64 = AtomicU64::new(0);
static PUNCHED: AtomicU64 = AtomicU64::new(0);
static KEPT: AtomicU64 = AtomicU64::new(0);
static FAILED: AtomicU64 = AtomicU64::new(0);
static LAZY: AtomicU64 = AtomicU64::new(0);
static PLAIN: AtomicU64 = AtomicU64::new(0);
/// A scan or a watch was refused (`broken`).
static BROKEN: AtomicBool = AtomicBool::new(false);
/// The range could not be made inheritable again after a lazy fork: until
/// it can, a fork that is not lazy would give its child no heap.
static STUCK: AtomicBool = AtomicBool::new(false);
/// The fork in progress, from its first handler to its last: that there is
/// one, whether its child maps the file, whether its child has no heap and
/// must not run, its pipe, and the signals its thread had blocked, if they
/// could be read.
static FORKING: AtomicBool = AtomicBool::new(false);
static MAPS: AtomicBool = AtomicBool::new(false);
static DOOMED: AtomicBool = AtomicBool::new(false);
static PIPE: Mutex<Option<(PipeReader, PipeWriter)>> = Mutex::new(None);
static MASKED: AtomicBool = AtomicBool::new(false);
static SIGNALS: AtomicU64 = AtomicU64::new(0);

/// The arena's extent hooks. An absent hook declines: a span is never
/// unmapped, decommitted or purged lazily, so the arena keeps it for reuse,
/// and it is committed from the start.
static HOOKS: jemalloc::extent_hooks_t = jemalloc::extent_hooks_t {
    alloc: Some(extent_alloc), dalloc: None, destroy: None, commit: None, decommit: None, purge_lazy: None,
    purge_forced: Some(extent_purge), split: Some(extent_split), merge: Some(extent_merge),
};

/// jemalloc's `arena_config_t`.
#[repr(C)]
struct ArenaConfig {
    extent_hooks: *mut jemalloc::extent_hooks_t,
    metadata_use_hooks: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PageRegion {
    start: u64,
    end: u64,
    categories: u64,
}
/// The kernel's `struct pm_scan_arg`.
#[repr(C)]
#[derive(Default)]
struct ScanArg {
    size: u64,
    flags: u64,
    start: u64,
    end: u64,
    walk_end: u64,
    vec: u64,
    vec_len: u64,
    max_pages: u64,
    category_inverted: u64,
    category_mask: u64,
    category_anyof_mask: u64,
    return_mask: u64,
}
const PAGEMAP_SCAN: libc::c_ulong = 0xc060_6610;
const PAGE_IS_WRITTEN: u64 = 1 << 1;
const PAGE_IS_PRESENT: u64 = 1 << 3;
const PAGE_IS_SWAPPED: u64 = 1 << 4;
const PM_SCAN_WP_MATCHING: u64 = 1 << 0;
const PM_SCAN_CHECK_WPASYNC: u64 = 1 << 1;
const UFFDIO_API: libc::c_ulong = 0xc018_aa3f;
const UFFDIO_REGISTER: libc::c_ulong = 0xc020_aa00;
const UFFD_API: u64 = 0xaa;
const UFFD_USER_MODE_ONLY: libc::c_int = 1;
const UFFD_FEATURE_WP_HUGETLBFS_SHMEM: u64 = 1 << 12;
const UFFD_FEATURE_WP_ASYNC: u64 = 1 << 15;
const UFFDIO_REGISTER_MODE_WP: u64 = 1 << 1;

fn heap() -> Option<&'static Heap> {
    HEAP.get()
}

fn lock() {
    while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
        std::hint::spin_loop();
    }
}

fn try_lock() -> bool {
    LOCK.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok()
}

fn unlock() {
    LOCK.store(false, Ordering::Release);
}

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Whether the file heap serves this process.
#[must_use]
pub fn active() -> bool {
    heap().is_some()
}

/// Whether AT lies in the range the file heap has handed out.
#[must_use]
pub fn owns(at: usize) -> bool {
    heap().is_some_and(|heap| (heap.base..TOP.load(Ordering::Relaxed)).contains(&at))
}

/// Start the file heap over LEN bytes of fresh address space, and make the
/// calling thread its writer: its allocations come from the file heap from
/// now on. Once per process.
///
/// # Errors
/// `Refused`, with the process as it was: the length is not whole pages, the
/// heap has started already, or the kernel or jemalloc refused a step
/// (a kernel before 6.7 has no asynchronous write-protect).
pub fn start(len: usize) -> Result<(), Refused> {
    if len == 0 || !len.is_multiple_of(PAGE) {
        return Err(Refused::Range);
    }
    if STARTING.swap(true, Ordering::AcqRel) {
        return Err(Refused::Started);
    }
    let heap = handlers().and_then(|()| open(len)).inspect_err(|_| STARTING.store(false, Ordering::Release))?;
    // The calling thread allocates nothing from its binding to here: its
    // arena would find no heap to take an extent from.
    let _ = HEAP.set(heap);
    Ok(())
}

/// The fork handlers, registered once: they do nothing until the heap is
/// set, and no fork may find the heap without them.
fn handlers() -> Result<(), Refused> {
    if HANDLERS.load(Ordering::Acquire) {
        return Ok(());
    }
    // SAFETY: the three handlers are `extern "C" fn()` that live for the program.
    let registered = unsafe { libc::pthread_atfork(Some(before_fork), Some(after_fork_parent), Some(after_fork_child)) };
    if registered != 0 {
        return Err(Refused::System("fork handlers", registered));
    }
    HANDLERS.store(true, Ordering::Release);
    Ok(())
}

/// The memory file of LEN bytes (sparse), the page map, the write watch, the
/// range, and the arena with the calling thread bound to it, in that order.
/// A step that fails closes and unmaps what the ones before made; an arena
/// made and not bound stays, empty: some kilobytes of the allocator's own.
fn open(len: usize) -> Result<Heap, Refused> {
    let file = rustix::fs::memfd_create(c"romacs-heap", MemfdFlags::CLOEXEC).map_err(|e| Refused::System("memory file", e.raw_os_error()))?;
    let file = File::from(file);
    file.set_len(len as u64).map_err(|e| Refused::System("memory file", e.raw_os_error().unwrap_or(0)))?;
    let pagemap = File::open("/proc/self/pagemap").map_err(|e| Refused::System("page map", e.raw_os_error().unwrap_or(0)))?;
    let watch = watch()?;
    // SAFETY: a new mapping wherever the kernel puts it, replacing nothing:
    // the range, inaccessible until a span is mapped over it.
    let base = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_NONE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE, -1, 0) };
    if base == libc::MAP_FAILED {
        return Err(Refused::System("range", errno()));
    }
    TOP.store(base as usize, Ordering::Relaxed);
    if let Err(refused) = arena() {
        // SAFETY: the mapping made above, which nothing has used.
        unsafe { libc::munmap(base, len) };
        return Err(refused);
    }
    WRITES.set(true);
    Ok(Heap { base: base as usize, limit: base as usize + len, file, watch, pagemap })
}

/// A userfaultfd for asynchronous write-protect.
fn watch() -> Result<OwnedFd, Refused> {
    let mut api = [UFFD_API, UFFD_FEATURE_WP_ASYNC | UFFD_FEATURE_WP_HUGETLBFS_SHMEM, 0];
    // SAFETY: the system call takes flags and returns a new descriptor,
    // which the `OwnedFd` then alone owns; `api` is the kernel's `struct
    // uffdio_api`, three u64, live for the call.
    let (watch, agreed) = unsafe {
        let Ok(fd) = i32::try_from(libc::syscall(libc::SYS_userfaultfd, libc::O_CLOEXEC | UFFD_USER_MODE_ONLY)) else { return Err(Refused::System("write watch", 0)) };
        if fd < 0 {
            return Err(Refused::System("write watch", errno()));
        }
        (OwnedFd::from_raw_fd(fd), libc::ioctl(fd, UFFDIO_API, api.as_mut_ptr()) == 0)
    };
    if !agreed {
        return Err(Refused::System("write watch", errno()));
    }
    Ok(watch)
}

/// The arena, with the calling thread bound to it. Its own structures stay
/// anonymous memory: a forked child's allocator finds them as any fork
/// leaves them, whatever the order of the fork handlers.
fn arena() -> Result<(), Refused> {
    let config = ArenaConfig { extent_hooks: (&raw const HOOKS).cast_mut(), metadata_use_hooks: false };
    let (mut index, mut index_len) = (0u32, size_of::<u32>());
    // SAFETY: the names are NUL-terminated; `index` and `index_len` are live
    // locals of the size the first control reads back, `config` a live
    // `arena_config_t` of the size passed (jemalloc only reads the hooks),
    // and `index` a live `unsigned` of the size the second takes.
    let bound = unsafe {
        match jemalloc::mallctl(c"experimental.arenas_create_ext".as_ptr(), (&raw mut index).cast(), &raw mut index_len,
            (&raw const config).cast_mut().cast(), size_of::<ArenaConfig>())
        {
            0 => jemalloc::mallctl(c"thread.arena".as_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), (&raw mut index).cast(), size_of::<u32>()),
            refused => refused,
        }
    };
    if bound != 0 {
        return Err(Refused::Arena(bound));
    }
    Ok(())
}

/// Run FORK, a fork(2) by the calling thread, and let it be a lazy one: when
/// the caller is the writer and no reader lives, the child maps the file
/// and inherits none of the range's page-table entries. For a caller whose
/// child reads, of the writer's blocks, only what the writer itself wrote
/// (see the module's comment for what such a child may otherwise see).
pub fn lazily<T>(fork: impl FnOnce() -> T) -> T {
    /// Puts back what was asked before, when FORK returns and when it unwinds.
    struct Asked(bool);
    impl Drop for Asked {
        fn drop(&mut self) {
            ASKED.set(self.0);
        }
    }
    let _asked = Asked(ASKED.replace(true));
    fork()
}

/// The span of SIZE bytes aligned to ALIGNMENT at the top of the range,
/// with what it skips to get there: (where the claim starts, where the span
/// starts, where it ends). None when the range is spent.
fn claim(heap: &Heap, size: usize, alignment: usize) -> Option<(usize, usize, usize)> {
    let mut top = TOP.load(Ordering::Relaxed);
    loop {
        let at = top.checked_next_multiple_of(alignment.max(PAGE))?;
        let end = at.checked_add(size.checked_next_multiple_of(PAGE)?)?;
        if end > heap.limit {
            return None;
        }
        match TOP.compare_exchange_weak(top, end, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => return Some((top, at, end)),
            Err(now) => top = now,
        }
    }
}

/// Map FROM..TO, a claim made for this call alone: a private mapping of the
/// file at its offset, watched for writes; in a forked child, fresh
/// anonymous memory, since the file is its parent's. False when it cannot be
/// mapped. A span the kernel will not watch is still memory, but no fold
/// would see its writes: folding ends (`broken`).
fn map(heap: &Heap, from: usize, to: usize) -> bool {
    let child = CHILD.load(Ordering::Acquire);
    let (flags, fd) = if child { (libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1) } else { (libc::MAP_PRIVATE, heap.file.as_raw_fd()) };
    let Ok(offset) = libc::off_t::try_from(from - heap.base) else { return false };
    let mut register = [from as u64, (to - from) as u64, UFFDIO_REGISTER_MODE_WP, 0];
    // SAFETY: FROM..TO lies inside the range, which is this module's to map
    // over, and was claimed for this call alone (`claim`), so nothing uses
    // it; `register` is the kernel's `struct uffdio_register`, four u64, and
    // names the mapping just made.
    let (mapped, watched) = unsafe {
        let at = libc::mmap(from as *mut c_void, to - from, libc::PROT_READ | libc::PROT_WRITE, flags | libc::MAP_FIXED | libc::MAP_NORESERVE, fd, offset);
        let mapped = at != libc::MAP_FAILED;
        (mapped, child || !mapped || libc::ioctl(heap.watch.as_raw_fd(), UFFDIO_REGISTER, register.as_mut_ptr()) == 0)
    };
    if !watched {
        broken();
    }
    mapped
}

/// The file can no longer be made the heap: no fold runs from now on.
fn broken() {
    BROKEN.store(true, Ordering::Release);
    FAILED.fetch_add(1, Ordering::Relaxed);
}

extern "C" fn extent_alloc(_: *mut jemalloc::extent_hooks_t, wanted: *mut c_void, size: usize, alignment: usize, zero: *mut bool, commit: *mut bool, _: u32) -> *mut c_void {
    // A span is asked for by address only to grow one in place: not offered.
    let Some(heap) = heap().filter(|_| wanted.is_null()) else { return std::ptr::null_mut() };
    let Some((_, at, _)) = claim(heap, size, alignment).filter(|&(from, _, end)| map(heap, from, end)) else {
        return std::ptr::null_mut();
    };
    // SAFETY: jemalloc passes `zero` and `commit` as live out-parameters.
    // A span of the range is handed out once, so the file holds nothing
    // there: it reads as zeros.
    unsafe {
        *zero = true;
        *commit = true;
    }
    at as *mut c_void
}

/// jemalloc's forced purge: give back the whole pages of the span. This
/// process's copies go at once, and with no reader left the file's go too.
/// False says the pages now read as zeros; true that they hold bytes of no
/// meaning (the file's, kept for a reader, a child's view of its parent's
/// file, or pages the kernel would not drop), which jemalloc then zeroes
/// itself where it promises zeros. It never waits for this module's lock:
/// a fold or a fork holds it, and the purging thread holds the allocator's
/// own locks; held, the file's pages stay, as for a reader.
extern "C" fn extent_purge(_: *mut jemalloc::extent_hooks_t, at: *mut c_void, _: usize, offset: usize, length: usize, _: u32) -> bool {
    let (start, end) = ((at as usize + offset).next_multiple_of(PAGE), (at as usize + offset + length) / PAGE * PAGE);
    let Some(heap) = heap().filter(|_| end > start) else { return true };
    // SAFETY: jemalloc purges part of a span of this arena that it holds
    // nothing in: nothing reads or holds these pages.
    let dropped = unsafe { libc::madvise(start as *mut c_void, end - start, libc::MADV_DONTNEED) } == 0;
    let punched = dropped && !CHILD.load(Ordering::Acquire) && try_lock() && {
        let punched = readers() == 0
            && rustix::fs::fallocate(&heap.file, FallocateFlags::PUNCH_HOLE | FallocateFlags::KEEP_SIZE, (start - heap.base) as u64, (end - start) as u64).is_ok();
        unlock();
        punched
    };
    (if punched { &PUNCHED } else { &KEPT }).fetch_add((end - start) as u64, Ordering::Relaxed);
    !punched
}

/// Offsets in the file follow addresses, so any two neighbours are one span and any span divides.
extern "C" fn extent_split(_: *mut jemalloc::extent_hooks_t, _: *mut c_void, _: usize, _: usize, _: usize, _: bool, _: u32) -> bool {
    false
}

extern "C" fn extent_merge(_: *mut jemalloc::extent_hooks_t, _: *mut c_void, _: usize, _: *mut c_void, _: usize, _: bool, _: u32) -> bool {
    false
}

/// Copy every page written since the last fold into the file. For the
/// writer; a page another thread writes meanwhile is copied as some of its
/// bytes then stood, and again at the next fold. Allocates nothing. None
/// when the file was left as it is: the heap is off, the caller is not its
/// writer, a reader exists, or the kernel refused a scan or a copy
/// (`Stats::failed_folds`).
#[must_use]
pub fn fold() -> Option<Fold> {
    let heap = heap().filter(|_| WRITES.get() && !CHILD.load(Ordering::Acquire))?;
    lock();
    let folded = if readers() == 0 { fold_locked(heap) } else { None };
    unlock();
    folded
}

/// How many forked children may still read the file. Those that are gone
/// are forgotten. Under the lock; allocates nothing, as a purge calls it
/// from inside the allocator.
fn readers() -> usize {
    let mut readers = READERS.lock().unwrap_or_else(PoisonError::into_inner);
    let mut polls = [libc::pollfd { fd: -1, events: 0, revents: 0 }; CHILDREN];
    for (poll, reader) in polls.iter_mut().zip(readers.iter()) {
        poll.fd = reader.as_ref().map_or(-1, AsRawFd::as_raw_fd);
    }
    if polls.iter().any(|poll| poll.fd >= 0) {
        // SAFETY: `polls` is CHILDREN live entries; the kernel skips a negative descriptor.
        unsafe { libc::poll(polls.as_mut_ptr(), CHILDREN as libc::nfds_t, 0) };
    }
    let mut alive = 0;
    for (poll, reader) in polls.iter().zip(readers.iter_mut()).filter(|(poll, _)| poll.fd >= 0) {
        // A read end hangs up when the last holder of its write end is gone.
        if poll.revents & libc::POLLHUP == 0 {
            alive += 1;
        } else {
            *reader = None;
        }
    }
    if LOST.load(Ordering::Relaxed) { usize::MAX } else { alive }
}

/// The pages of FROM..TO written since they were last listed, into REGIONS,
/// each write-protected again; and where the walk stopped. None when the
/// kernel refuses the scan.
fn written(heap: &Heap, from: u64, to: u64, regions: &mut [PageRegion]) -> Option<(usize, u64)> {
    let mut arg = ScanArg {
        size: size_of::<ScanArg>() as u64, flags: PM_SCAN_WP_MATCHING | PM_SCAN_CHECK_WPASYNC, start: from, end: to,
        vec: regions.as_mut_ptr() as u64, vec_len: regions.len() as u64, category_mask: PAGE_IS_WRITTEN,
        // A page never touched is not listed, so it is never protected and
        // a reservation grows no page table.
        category_anyof_mask: PAGE_IS_PRESENT | PAGE_IS_SWAPPED, return_mask: PAGE_IS_WRITTEN, ..ScanArg::default()
    };
    // SAFETY: `arg` is the kernel's `struct pm_scan_arg` and names `regions`, both live for the call.
    let found = unsafe { libc::ioctl(heap.pagemap.as_raw_fd(), PAGEMAP_SCAN, &raw mut arg) };
    usize::try_from(found).ok().map(|found| (found, arg.walk_end))
}

/// Have the kernel copy the LEN bytes at START, pages of the range, into the
/// file at their offset. False when it would not: those pages are protected
/// again and not in the file, so the file cannot be made the heap any more.
fn copy(heap: &Heap, start: usize, len: usize) -> bool {
    let mut done = 0;
    while done < len {
        let Ok(offset) = libc::off_t::try_from(start + done - heap.base) else { return false };
        // SAFETY: the pages are mapped, being the range's own, and the
        // kernel does the reading: other threads may be writing them (see
        // the module's comment), which a copy made by this program's own
        // code could not be allowed to race with. The kernel copies user
        // memory as it finds it, changing or not; a page caught mid-write
        // is listed again by the next scan, since the scan protected it
        // before this copy and the write unprotects it.
        let wrote = unsafe { libc::pwrite(heap.file.as_raw_fd(), (start + done) as *const c_void, len - done, offset) };
        match usize::try_from(wrote) {
            Ok(0) => return false,
            Ok(wrote) => done += wrote,
            Err(_) if errno() == libc::EINTR => {}
            Err(_) => return false,
        }
    }
    true
}

/// An address or a length the kernel gave, as this process's own: the two
/// are one width wherever `PAGEMAP_SCAN` exists.
fn native(n: u64) -> usize {
    usize::try_from(n).expect("a 64-bit address space")
}

/// The fold, under the lock. None when a scan or a copy was ever refused:
/// the file is then not the heap, and the caller must not let a child map it.
fn fold_locked(heap: &Heap) -> Option<Fold> {
    if BROKEN.load(Ordering::Acquire) {
        return None;
    }
    let top = TOP.load(Ordering::Acquire) as u64;
    let mut folded = Fold::default();
    let mut regions = [PageRegion::default(); 256];
    let mut at = heap.base as u64;
    while at < top {
        // A refused scan has protected again the pages it walked without
        // listing them: their writes are lost to every later fold.
        let Some((found, walked)) = written(heap, at, top, &mut regions) else {
            broken();
            return None;
        };
        for region in &regions[..found] {
            let (start, len) = (native(region.start), native(region.end - region.start));
            if !copy(heap, start, len) {
                broken();
                return None;
            }
            folded.pages += len / PAGE;
            folded.runs += 1;
        }
        if walked <= at {
            break;
        }
        at = walked;
    }
    FOLDS.fetch_add(1, Ordering::Relaxed);
    FOLDED.fetch_add(folded.pages as u64, Ordering::Relaxed);
    Some(folded)
}

#[must_use]
pub fn stats() -> Stats {
    let Some(heap) = heap() else { return Stats::default() };
    let readers = if CHILD.load(Ordering::Acquire) {
        0
    } else {
        lock();
        let readers = readers();
        unlock();
        readers
    };
    Stats {
        mapped: TOP.load(Ordering::Relaxed) - heap.base, folds: FOLDS.load(Ordering::Relaxed),
        folded_pages: FOLDED.load(Ordering::Relaxed), punched: PUNCHED.load(Ordering::Relaxed), kept: KEPT.load(Ordering::Relaxed),
        readers, lazy_forks: LAZY.load(Ordering::Relaxed), plain_forks: PLAIN.load(Ordering::Relaxed),
        failed_folds: FAILED.load(Ordering::Relaxed),
    }
}

/// Leave the range out of forks (OUT), or put it back. False when the kernel refused.
fn exclude(heap: &Heap, out: bool) -> bool {
    let top = TOP.load(Ordering::Acquire);
    // SAFETY: the range's mapped part, which this module owns; the advice changes only what a fork copies.
    top == heap.base || unsafe { libc::madvise(heap.base as *mut c_void, top - heap.base, if out { libc::MADV_DONTFORK } else { libc::MADV_DOFORK }) } == 0
}

/// Make MASK the calling thread's blocked signals; the set it had, when the kernel says.
fn block(mask: u64) -> Option<u64> {
    let mut old = 0u64;
    // SAFETY: both sets are live u64, the size of the kernel's signal set, which the last argument says.
    let done = unsafe { libc::syscall(libc::SYS_rt_sigprocmask, libc::SIG_SETMASK, &raw const mask, &raw mut old, size_of::<u64>()) };
    (done == 0).then_some(old)
}

/// The heap of a process that is not a forked child of the writer's process.
fn own_heap() -> Option<&'static Heap> {
    heap().filter(|_| !CHILD.load(Ordering::Acquire))
}

/// Before any fork: the file stays as it is until the fork has returned in
/// the parent and its child is counted a reader, and no signal handler runs
/// in a lazy child before its heap is there. A fork the writer asked to be
/// lazy, with no reader alive, folds and leaves the range out: its child
/// will map the file. Any other fork is the kernel's own.
extern "C" fn before_fork() {
    let Some(heap) = own_heap() else { return };
    let signals = block(u64::MAX);
    lock();
    FORKING.store(true, Ordering::Relaxed);
    MASKED.store(signals.is_some(), Ordering::Relaxed);
    SIGNALS.store(signals.unwrap_or(0), Ordering::Relaxed);
    let alone = readers() == 0;
    let lazy = ASKED.get() && WRITES.get() && alone && fold_locked(heap).is_some();
    if lazy {
        // Refused, the fork copies entries the child then maps over: the cost, not the result.
        exclude(heap, true);
    } else if STUCK.load(Ordering::Relaxed) {
        // Part of the range is still left out of forks: this child would have no heap there.
        let back = exclude(heap, false);
        STUCK.store(!back, Ordering::Relaxed);
        DOOMED.store(!back, Ordering::Relaxed);
    }
    MAPS.store(lazy, Ordering::Relaxed);
    (if lazy { &LAZY } else { &PLAIN }).fetch_add(1, Ordering::Relaxed);
    *PIPE.lock().unwrap_or_else(PoisonError::into_inner) = std::io::pipe().ok();
}

/// In the parent, whether or not the fork made a child: the range is
/// inherited again, and the child holds the pipe's write end and reads the
/// file for as long (mapped by itself, or through the pages it inherited
/// unwritten).
extern "C" fn after_fork_parent() {
    let Some(heap) = own_heap().filter(|_| FORKING.swap(false, Ordering::Relaxed)) else { return };
    if MAPS.load(Ordering::Relaxed) {
        STUCK.store(!exclude(heap, false), Ordering::Relaxed);
    }
    DOOMED.store(false, Ordering::Relaxed);
    // This process's copy of the write end closes here; the child's stays.
    let pipe = PIPE.lock().unwrap_or_else(PoisonError::into_inner).take();
    let mut readers = READERS.lock().unwrap_or_else(PoisonError::into_inner);
    match (pipe, readers.iter_mut().find(|slot| slot.is_none())) {
        (Some((read, _)), Some(slot)) => *slot = Some(read),
        // No pipe, or more live children than slots: this child is a reader nothing tracks.
        _ => LOST.store(true, Ordering::Relaxed),
    }
    drop(readers);
    let signals = MASKED.load(Ordering::Relaxed).then(|| SIGNALS.load(Ordering::Relaxed));
    unlock();
    if let Some(mask) = signals {
        block(mask);
    }
}

/// In the child: after a lazy fork the heap is the file as the fold left
/// it, mapped where the range was; otherwise it is the copy every fork
/// makes. A child's own children find no fork in progress here and inherit
/// their parent's heap as any fork does. The pipe's write end stays open
/// here, inherited by the child's own children, until the last of them
/// exits or execs.
extern "C" fn after_fork_child() {
    let Some(heap) = heap().filter(|_| FORKING.swap(false, Ordering::Relaxed)) else { return };
    CHILD.store(true, Ordering::Release);
    LOCK.store(false, Ordering::Release);
    let top = TOP.load(Ordering::Acquire);
    // SAFETY: after a lazy fork the range was not inherited, so nothing is
    // mapped there and nothing of it is in use here yet; the file is open in
    // the child.
    let mapped = !MAPS.load(Ordering::Relaxed) || top == heap.base || unsafe {
        libc::mmap(heap.base as *mut c_void, top - heap.base, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_FIXED | libc::MAP_NORESERVE, heap.file.as_raw_fd(), 0)
    } != libc::MAP_FAILED;
    if !mapped || DOOMED.load(Ordering::Relaxed) {
        // A child without its heap can run nothing.
        std::process::abort();
    }
    if MASKED.load(Ordering::Relaxed) {
        block(SIGNALS.load(Ordering::Relaxed));
    }
}
