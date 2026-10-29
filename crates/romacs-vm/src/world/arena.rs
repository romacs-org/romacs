//! The world arena: the global allocator of a binary that boots from world
//! images. Every heap allocation comes from one fixed range of address
//! space, and the allocator's own state lives at the start of that range,
//! so a world image is the range's used prefix written to a file, and
//! restore maps that file back at the same address, copy-on-write
//! (`adopt`). Nothing is decoded and nothing is relocated: every pointer
//! in the image is valid again where it was. The process allocates from
//! the image from then on; what it allocated before stays in its first
//! arena, which keeps serving those blocks' frees. A builder's arena is a
//! shared mapping of an unlinked memory file, and its image is copied from
//! that file by the kernel, so capture never reads heap bytes (`write_image`).
//!
//! The range's address is random, chosen by each process that starts a
//! fresh arena, so each world has its own. Everything else a world process
//! maps stays randomized, the executable too. What an image holds of the
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
//! spin lock serializes it. Outside world binaries nothing installs it; when
//! its range cannot be reserved it hands every request to the system
//! allocator.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicUsize, Ordering};

/// Where an arena may start: a GiB-aligned address in this window, far from
/// the executable, its heap, the libraries and the stack.
const WINDOW: (usize, usize) = (0x1000_0000_0000, 0x5000_0000_0000);
const ALIGN: usize = 1 << 30;
/// The most an arena reserves. Pages are committed only as they are
/// touched; under an address-space limit an arena reserves a quarter of it,
/// as a process may hold two (`adopt`).
const RESERVE: usize = 1 << 37;
const PAGE: usize = 4096;
const MAGIC: u64 = u64::from_le_bytes(*b"RWARENA2");
/// 16..=1024 by 16, then four steps per doubling to 32 KiB.
const CLASSES: usize = 64 + 20;
const SMALL_MAX: usize = 32768;
const BINS: usize = 40;
/// Freed page runs this long give their pages back.
const RELEASE_PAGES: usize = 64;

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
/// How the image this process asked for fared (`Restore`).
/// Whether freed blocks are zeroed: in a builder, so that no freed byte
/// reaches an image.
static SCRUB: AtomicBool = AtomicBool::new(false);
static RESTORE: AtomicU8 = AtomicU8::new(Restore::None as u8);
/// Where the arena allocations come from starts, once it has.
static BASE: AtomicUsize = AtomicUsize::new(0);
/// Where the process's first arena starts, once an image replaced it.
static FORMER: AtomicUsize = AtomicUsize::new(0);
/// A builder's arena is a shared mapping of this unlinked file, and an
/// image is copied from the file (`write_image`); -1 elsewhere.
static BACKING: AtomicI32 = AtomicI32::new(-1);
/// The digest of the libraries mapped as the arena started (`libraries`).
static mut LIBRARIES: Option<[u8; 32]> = None;

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
    if end > h.base + h.reserve {
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
        unsafe { release(at + PAGE, (pages - 1) * PAGE) };
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

/// The header of the arena AT is in, or None for the system allocator's.
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

/// Reserve a fresh range at a random address in the window.
unsafe fn start() -> u8 {
    unsafe { LIBRARIES = libraries_digest() };
    let building = !unsafe { libc::getenv(c"ROMACS_WORLD_BUILD".as_ptr()) }.is_null();
    SCRUB.store(building, Ordering::Relaxed);
    let Some(reserve) = (unsafe { reserve() }) else { return SYSTEM };
    let backing = if building { unsafe { backing_file(reserve) } } else { -1 };
    let flags = libc::MAP_NORESERVE | libc::MAP_FIXED_NOREPLACE
        | if backing >= 0 { libc::MAP_SHARED } else { libc::MAP_PRIVATE | libc::MAP_ANONYMOUS };
    for _ in 0..8 {
        let mut random = 0usize;
        let read = unsafe { libc::getrandom((&raw mut random).cast(), size_of::<usize>(), 0) };
        if read != size_of::<usize>() as isize {
            break;
        }
        let base = WINDOW.0 + random % ((WINDOW.1 - WINDOW.0 - reserve) / ALIGN) * ALIGN;
        let at = unsafe { libc::mmap(base as *mut libc::c_void, reserve, libc::PROT_READ | libc::PROT_WRITE, flags, backing, 0) };
        if at == base as *mut libc::c_void {
            BACKING.store(backing, Ordering::Relaxed);
            BASE.store(base, Ordering::Relaxed);
            let h = unsafe { &mut *header() };
            h.magic = MAGIC;
            h.base = base;
            h.reserve = reserve;
            h.top = base + size_of::<Header>().next_multiple_of(PAGE);
            h.zero = h.top;
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
    let fd = unsafe { libc::open(c"/dev/shm".as_ptr(), libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC, 0o600) };
    if fd < 0 {
        return -1;
    }
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
/// with what it addresses of the executable (`shadow_executable`), when it
/// is a whole image of this format and its range is free. Once per
/// process; the first arena keeps the blocks allocated before.
pub fn adopt(file: &std::fs::File) -> Restore {
    use std::os::fd::AsRawFd;
    if state() != ARENA || FORMER.load(Ordering::Acquire) != 0 {
        return Restore::Map;
    }
    let outcome = unsafe { map_image_fd(file.as_raw_fd()) };
    RESTORE.store(outcome as u8, Ordering::Release);
    outcome
}

unsafe fn map_image_fd(fd: i32) -> Restore {
    let mut bytes = [0u8; size_of::<Header>()];
    let read = unsafe { libc::pread(fd, bytes.as_mut_ptr().cast(), bytes.len(), 0) };
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
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
        _ => return Restore::Map,
    };
    let file = libc::MAP_PRIVATE | libc::MAP_FIXED_NOREPLACE;
    let at = unsafe { libc::mmap(base as *mut libc::c_void, len, libc::PROT_READ | libc::PROT_WRITE, file, fd, 0) };
    if at != base as *mut libc::c_void {
        if at != libc::MAP_FAILED {
            unsafe { libc::munmap(at, len) };
        }
        return Restore::Map;
    }
    let anon = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE | libc::MAP_FIXED_NOREPLACE;
    let rest = unsafe { libc::mmap((base + len) as *mut libc::c_void, reserve - len, libc::PROT_READ | libc::PROT_WRITE, anon, -1, 0) };
    if rest != (base + len) as *mut libc::c_void {
        if rest != libc::MAP_FAILED {
            unsafe { libc::munmap(rest, reserve - len) };
        }
        unsafe { libc::munmap(base as *mut libc::c_void, len) };
        return Restore::Map;
    }
    if !unsafe { shadow_executable(image.image.executable) } {
        unsafe { libc::munmap(base as *mut libc::c_void, reserve) };
        return Restore::Executable;
    }
    // This process's reservation, not the builder's.
    unsafe { (*(base as *mut Header)).reserve = reserve };
    // The kernel reads the file ahead while the process starts.
    unsafe { libc::madvise(base as *mut libc::c_void, len, libc::MADV_WILLNEED) };
    lock();
    FORMER.store(BASE.load(Ordering::Relaxed), Ordering::Release);
    BASE.store(base, Ordering::Release);
    unlock();
    Restore::Mapped
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

/// What identifies a loaded object's code, read from its mapping.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Identity<'a> {
    /// Its GNU build-id note.
    BuildId(&'a [u8]),
    /// With no build-id: the SHA-256 of its executable segments as mapped.
    Code([u8; 32]),
}

/// The libraries this process loaded before its first allocation, each
/// by its name and the identity its mapping gives (`identity`), read
/// through dl_iterate_phdr from the objects themselves: no path is
/// looked up, so no file another one replaced or overmounted at its path
/// can stand in for it. Read without allocating.
unsafe fn libraries_digest() -> Option<[u8; 32]> {
    struct Walk {
        entries: &'static mut [[u8; 32]; 256],
        count: usize,
        complete: bool,
    }
    static mut ENTRIES: [[u8; 32]; 256] = [[0; 32]; 256];
    unsafe extern "C" fn each(info: *mut libc::dl_phdr_info, _: libc::size_t, data: *mut libc::c_void) -> libc::c_int {
        let walk = unsafe { &mut *data.cast::<Walk>() };
        let info = unsafe { &*info };
        let name = if info.dlpi_name.is_null() { &[][..] } else { unsafe { std::ffi::CStr::from_ptr(info.dlpi_name) }.to_bytes() };
        // The executable, keyed by its file's content, and the vDSO, the
        // kernel's, which no image refers to.
        if name.is_empty() || name.starts_with(b"linux-vdso") {
            return 0;
        }
        let headers = unsafe { std::slice::from_raw_parts(info.dlpi_phdr, info.dlpi_phnum as usize) };
        let entry = unsafe { identity(info.dlpi_addr as usize, headers) }.and_then(|id| {
            (walk.count < walk.entries.len()).then(|| library_entry(name, id))
        });
        match entry {
            Some(entry) => {
                walk.entries[walk.count] = entry;
                walk.count += 1;
                0
            }
            None => {
                walk.complete = false;
                1
            }
        }
    }
    let mut walk = Walk { entries: unsafe { &mut *std::ptr::addr_of_mut!(ENTRIES) }, count: 0, complete: true };
    unsafe { libc::dl_iterate_phdr(Some(each), (&raw mut walk).cast()) };
    walk.complete.then(|| entries_digest(&mut walk.entries[..walk.count]))
}

/// The identity of the object loaded at BASE with program HEADERS: its
/// build-id note, or the digest of its executable segments as mapped.
/// None when it has neither.
unsafe fn identity(base: usize, headers: &[libc::Elf64_Phdr]) -> Option<Identity<'static>> {
    for header in headers.iter().filter(|h| h.p_type == libc::PT_NOTE) {
        let notes = unsafe { std::slice::from_raw_parts((base + header.p_vaddr as usize) as *const u8, header.p_memsz as usize) };
        if let Some(id) = build_id(notes) {
            return Some(Identity::BuildId(id));
        }
    }
    use sha2::Digest;
    let mut digest = sha2::Sha256::new();
    let mut any = false;
    for header in headers.iter().filter(|h| h.p_type == libc::PT_LOAD && h.p_flags & libc::PF_X != 0) {
        // Mapped from the file, readable and initialized: code.
        let code = unsafe { std::slice::from_raw_parts((base + header.p_vaddr as usize) as *const u8, header.p_filesz as usize) };
        digest.update(code);
        any = true;
    }
    any.then(|| Identity::Code(digest.finalize().into()))
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

/// One library's entry: its name and identity, digested.
fn library_entry(name: &[u8], id: Identity<'_>) -> [u8; 32] {
    use sha2::Digest;
    let mut digest = sha2::Sha256::new();
    digest.update(name);
    match id {
        Identity::BuildId(bytes) => {
            digest.update(b"\0build-id\0");
            digest.update(bytes);
        }
        Identity::Code(hash) => {
            digest.update(b"\0code\0");
            digest.update(hash);
        }
    }
    digest.finalize().into()
}

/// The digest of ENTRIES, in no order, since where each library loaded is
/// random, and once each.
fn entries_digest(entries: &mut [[u8; 32]]) -> [u8; 32] {
    use sha2::Digest;
    entries.sort_unstable();
    let mut digest = sha2::Sha256::new();
    for (i, entry) in entries.iter().enumerate() {
        if i == 0 || entries[i - 1] != *entry {
            digest.update(entry);
        }
    }
    digest.finalize().into()
}

/// The digest of the libraries this process mapped before its first
/// allocation: what a world's key binds of them. A library rebuilt, or
/// another one loaded, changes it. None when one could not be identified,
/// or more were loaded than the inventory holds: a world is not keyed on
/// an incomplete one (`libraries_digest`).
pub fn libraries() -> Option<[u8; 32]> {
    state();
    unsafe { LIBRARIES }
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

/// Whether this process's heap is the arena.
pub fn active() -> bool {
    state() == ARENA
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
            let run = unsafe { &*(at as *const Run) };
            unsafe { std::ptr::write_bytes((at + size_of::<Run>()) as *mut u8, 0, PAGE - size_of::<Run>()) };
            if run.pages > 1 {
                unsafe { release(at + PAGE, (run.pages - 1) * PAGE) };
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
        let Some(owner) = owner(at) else {
            return unsafe { System.dealloc(at, layout) };
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
            return unsafe { System.realloc(at, layout, new_size) };
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
                } else if start + old * PAGE == h.top && h.top + (new - old) * PAGE <= h.base + h.reserve {
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

    #[test]
    fn a_library_is_known_by_its_build_id_or_its_code_in_no_order() {
        let digest = |list: &[(&[u8], Identity)]| {
            let mut entries: Vec<[u8; 32]> = list.iter().map(|(name, id)| library_entry(name, *id)).collect();
            entries_digest(&mut entries)
        };
        let libc_id: &[u8] = &[1, 2, 3];
        let code = Identity::Code([7; 32]);
        let base = digest(&[(b"/usr/lib/libc.so.6", Identity::BuildId(libc_id)), (b"/usr/lib/libz.so.1", code)]);
        assert_eq!(digest(&[(b"/usr/lib/libz.so.1", code), (b"/usr/lib/libc.so.6", Identity::BuildId(libc_id))]), base);
        // Another build at the same path, overmounted or replaced, whatever
        // device and inode it has.
        assert_ne!(digest(&[(b"/usr/lib/libc.so.6", Identity::BuildId(&[9, 9])), (b"/usr/lib/libz.so.1", code)]), base);
        assert_ne!(digest(&[(b"/usr/lib/libc.so.6", Identity::BuildId(libc_id)), (b"/usr/lib/libz.so.1", Identity::Code([8; 32]))]), base);
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
        assert!(unsafe { libraries_digest() }.is_some());
        let mut found: Option<(String, Vec<u8>)> = None;
        unsafe extern "C" fn each(info: *mut libc::dl_phdr_info, _: libc::size_t, data: *mut libc::c_void) -> libc::c_int {
            let info = unsafe { &*info };
            let name = unsafe { std::ffi::CStr::from_ptr(info.dlpi_name) }.to_string_lossy().into_owned();
            if name.contains("libc.so") {
                let headers = unsafe { std::slice::from_raw_parts(info.dlpi_phdr, info.dlpi_phnum as usize) };
                let id = match unsafe { identity(info.dlpi_addr as usize, headers) } {
                    Some(Identity::BuildId(id)) => id.to_vec(),
                    other => panic!("libc without a build-id: {other:?}"),
                };
                unsafe { *data.cast::<Option<(String, Vec<u8>)>>() = Some((name, id)) };
            }
            0
        }
        unsafe { libc::dl_iterate_phdr(Some(each), (&raw mut found).cast()) };
        let (path, mapped) = found.expect("libc is loaded");
        assert_eq!(file_build_id(&path), Some(mapped), "{path}");
    }

    #[test]
    fn bins_order_runs_by_length() {
        assert_eq!((bin(1), bin(2), bin(3), bin(4), bin(1 << 50)), (0, 1, 1, 2, BINS - 1));
    }
}
