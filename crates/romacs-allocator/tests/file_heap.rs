//! The file heap through its interface. The heap is process-wide and its
//! scenarios fork, so the one test re-executes this binary and they run
//! there, in order.
use romacs_allocator::file_heap::{self, Fold, Refused};
use std::ffi::{c_int, c_ulong, c_void};
use std::io::{BufRead, BufReader, PipeReader, PipeWriter, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicUsize, Ordering};

#[global_allocator]
static ALLOCATOR: romacs_allocator::PlatformAllocator = romacs_allocator::PLATFORM_ALLOCATOR;

const OWN_PROCESS: &str = "ROMACS_FILE_HEAP_SCENARIOS";
const MB: usize = 1 << 20;
const LEN: usize = 16 << 30;

#[test]
fn the_file_heap_serves_a_writer_and_its_forks() {
    if std::env::var_os(OWN_PROCESS).is_some() {
        return scenarios();
    }
    let run = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["the_file_heap_serves_a_writer_and_its_forks", "--exact", "--nocapture", "--test-threads=1"])
        .env(OWN_PROCESS, "1")
        .output()
        .unwrap();
    assert!(run.status.success(), "the scenarios failed in their own process: {}\n{}", run.status, String::from_utf8_lossy(&run.stderr));
}

/// While set, registering fork handlers fails as the C library does when it is out of memory.
static REFUSE_HANDLERS: AtomicBool = AtomicBool::new(false);

/// A fork handler, as the C library takes one.
type Handler = Option<unsafe extern "C" fn()>;

// SAFETY: both are the C library's and the linker's own, declared as they
// declare them: the function `pthread_atfork` is defined in terms of, and
// the handle every program has.
unsafe extern "C" {
    fn __register_atfork(prepare: Handler, parent: Handler, child: Handler, dso: *const u8) -> c_int;
    static __dso_handle: u8;
}

/// The C library's `pthread_atfork`, defined here as the library itself
/// defines it, in the program, so that a test can make it fail.
// SAFETY: the name and signature are the C library's own; the definition
// it replaces is that library's weak one, which makes this same call.
#[unsafe(no_mangle)]
extern "C" fn pthread_atfork(prepare: Handler, parent: Handler, child: Handler) -> c_int {
    if REFUSE_HANDLERS.load(Ordering::Relaxed) {
        return libc::ENOMEM;
    }
    // SAFETY: the handlers are the caller's, and `__dso_handle` is this program's.
    unsafe { __register_atfork(prepare, parent, child, &raw const __dso_handle) }
}

/// While set, the kernel's refusal to watch a mapping for writes is played.
static REFUSE_WATCH: AtomicBool = AtomicBool::new(false);
const UFFDIO_REGISTER: c_ulong = 0xc020_aa00;

/// The C library's `ioctl`, defined in the program so that a test can make
/// one request fail; every other request goes to the kernel as it was.
// SAFETY: the name is the C library's own and the three fixed arguments are
// how every caller here passes its variadic ones; the system call is the
// one the library's function makes.
#[unsafe(no_mangle)]
extern "C" fn ioctl(fd: c_int, request: c_ulong, arg: *mut c_void) -> c_int {
    if request == UFFDIO_REGISTER && REFUSE_WATCH.load(Ordering::Relaxed) {
        // SAFETY: the calling thread's own errno.
        unsafe { *libc::__errno_location() = libc::ENOMEM };
        return -1;
    }
    // SAFETY: the caller's request, passed on unchanged.
    c_int::try_from(unsafe { libc::syscall(libc::SYS_ioctl, fd, request, arg) }).unwrap_or(-1)
}

/// What the probe's fork handlers do at the next fork.
const QUIET: u8 = 0;
/// In the prepare step: have the writer allocate a fresh block, which the
/// child then probes.
const ALLOCATE: u8 = 1;
/// In the prepare step: store 2 in the cell at `PROBE_AT`.
const STORE: u8 = 2;
/// In the prepare step, which runs with the heap's lock held: have another
/// thread free a block and purge, and end the process if it has not within
/// ten seconds (a purge that waited for the lock never would).
const PURGE: u8 = 3;
static MODE: AtomicU8 = AtomicU8::new(QUIET);
/// Where the probe looks, and what it found in a child before the heap's
/// own handler ran there: 1 when that page was mapped, 2 when not.
static PROBE_AT: AtomicUsize = AtomicUsize::new(0);
static PROBED: AtomicU8 = AtomicU8::new(0);
/// The pipes between a forking thread's prepare step and the writer (`ALLOCATE`).
static ASK: AtomicI32 = AtomicI32::new(-1);
static ANSWER: AtomicI32 = AtomicI32::new(-1);

/// Registered before the heap starts, the probe's prepare step runs after
/// the heap's, and its child step before the heap's.
extern "C" fn probe_prepare() {
    match MODE.swap(QUIET, Ordering::Relaxed) {
        ALLOCATE => {
            // SAFETY: two pipe descriptors the test opened and keeps open; one byte each way.
            unsafe {
                libc::write(ASK.load(Ordering::Relaxed), [1u8].as_ptr().cast(), 1);
                libc::read(ANSWER.load(Ordering::Relaxed), [0u8].as_mut_ptr().cast(), 1);
            }
        }
        // SAFETY: the cell is the test's live `AtomicUsize`, read only through atomics.
        STORE => unsafe { (*(PROBE_AT.load(Ordering::Relaxed) as *const AtomicUsize)).store(2, Ordering::Relaxed) },
        PURGE => {
            let mut answered = libc::pollfd { fd: ANSWER.load(Ordering::Relaxed), events: libc::POLLIN, revents: 0 };
            // SAFETY: a pipe descriptor the test keeps open, one live `pollfd`, and `_exit` with a status.
            unsafe {
                libc::write(ASK.load(Ordering::Relaxed), [1u8].as_ptr().cast(), 1);
                if libc::poll(&raw mut answered, 1, 10_000) != 1 {
                    libc::_exit(3);
                }
            }
        }
        _ => {}
    }
}

/// Asks the kernel whether the fork gave the child the probed page; touches no memory of the heap.
extern "C" fn probe_child() {
    let mut resident = 0u8;
    // SAFETY: asks about one page by address; `resident` is a live byte for the answer.
    let mapped = unsafe { libc::mincore((PROBE_AT.load(Ordering::Relaxed) & !4095) as *mut c_void, 4096, &raw mut resident) } == 0;
    PROBED.store(if mapped { 1 } else { 2 }, Ordering::Relaxed);
}

/// Whether the fork that made this process gave it the probed page.
fn inherited() -> bool {
    PROBED.load(Ordering::Relaxed) == 1
}

/// A forked child running a body. The body gets a function that blocks
/// until the parent says `go`; the child exits 0 when the body returns true.
struct Child {
    pid: rustix::process::Pid,
    go: PipeWriter,
}

fn read_byte(mut from: &PipeReader) {
    let _ = from.read(&mut [0]);
}

fn write_byte(mut to: &PipeWriter) {
    let _ = to.write(&[1]);
}

/// Reap the child PID, or any child: whether it exited with status 0.
fn reaped(pid: Option<rustix::process::Pid>) -> bool {
    let status = rustix::process::waitpid(pid, rustix::process::WaitOptions::empty()).unwrap().unwrap().1;
    status.exit_status() == Some(0)
}

fn fork(body: impl FnOnce(&dyn Fn()) -> bool) -> Child {
    let (wait, go) = std::io::pipe().unwrap();
    // SAFETY: fork takes no argument; the child leaves by `_exit` below.
    let pid = unsafe { libc::fork() };
    if pid == 0 {
        // The child holds no way to say `go`: should the parent die, the wait ends too.
        drop(go);
        let held = || read_byte(&wait);
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body(&held))).unwrap_or(false);
        // SAFETY: `_exit` takes a status.
        unsafe { libc::_exit(i32::from(!ok)) };
    }
    Child { pid: rustix::process::Pid::from_raw(pid).expect("a fork"), go }
}

/// A child of a fork asked to be lazy.
fn lazy(body: impl FnOnce(&dyn Fn()) -> bool) -> Child {
    file_heap::lazily(|| fork(body))
}

impl Child {
    /// Let the child finish and reap it: whether its body returned true.
    fn ok(self) -> bool {
        write_byte(&self.go);
        reaped(Some(self.pid))
    }
}

fn all(bytes: &[u8], byte: u8) -> bool {
    bytes.iter().all(|&b| b == byte)
}

fn scenarios() {
    // SAFETY: both handlers are `extern "C" fn()` that live for the program.
    assert_eq!(unsafe { libc::pthread_atfork(Some(probe_prepare), None, Some(probe_child)) }, 0);
    starting();
    let mut text = vec![b'a'; MB];
    folding(&mut text);
    lazy_forks(&mut text);
    racing_forks();
    unwinding();
    descendants(&mut text);
    purges();
    purge_under_the_lock();
    // Last: they end folding for the rest of the process.
    refused_scan();
    unwatched_span();
}

/// The heap starts once, with its fork handlers, and its writer is the thread that started it.
fn starting() {
    assert_eq!(file_heap::start(100), Err(Refused::Range));
    REFUSE_HANDLERS.store(true, Ordering::Relaxed);
    assert_eq!(file_heap::start(LEN), Err(Refused::System("fork handlers", libc::ENOMEM)), "no heap without its fork handlers");
    REFUSE_HANDLERS.store(false, Ordering::Relaxed);
    assert!(!file_heap::active() && file_heap::fold().is_none());
    assert_eq!(file_heap::start(LEN), Ok(()));
    assert_eq!(file_heap::start(LEN), Err(Refused::Started));
    assert!(file_heap::active());
    // Another thread's memory is not the heap's, and only the writer folds.
    assert!(std::thread::spawn(|| !file_heap::owns(vec![1u8; MB].as_ptr() as usize) && file_heap::fold().is_none()).join().unwrap());
}

/// A fold takes what was written, once; a reservation never touched is not written.
fn folding(text: &mut [u8]) {
    assert!(file_heap::owns(text.as_ptr() as usize));
    let reservation: Vec<u8> = Vec::with_capacity(256 * MB);
    assert!(file_heap::owns(reservation.as_ptr() as usize));
    let first = file_heap::fold().expect("a fold with no reader");
    assert!((MB / 4096..4 * MB / 4096).contains(&first.pages), "the megabyte written, not the reservation: {first:?}");
    assert_eq!(file_heap::fold(), Some(Fold::default()), "nothing was written since");
    text[4096 * 3] = b'a';
    assert_eq!(file_heap::fold(), Some(Fold { pages: 1, runs: 1 }), "one page was written since");
}

fn lazy_forks(text: &mut [u8]) {
    // What the fork itself gives a child of the heap is asked of the kernel
    // about a page of this block, before the heap's handler runs in the child.
    let ballast = vec![5u8; 64 * MB];
    PROBE_AT.store(ballast.as_ptr() as usize + 4096, Ordering::Relaxed);

    // A fork the writer asks to be lazy folds, and its child maps the file:
    // the fork gives it none of the heap, and it reads what the parent wrote
    // just before the fork, and goes on reading that whatever the parent
    // writes afterwards. What the child writes is its own.
    text.fill(b'b');
    let child = lazy(|held| {
        let before = all(text, b'b') && all(&ballast, 5);
        held();
        let after = all(text, b'b');
        text.fill(b'z');
        !inherited() && before && after && all(text, b'z') && file_heap::fold().is_none()
    });
    text.fill(b'c');
    let stats = file_heap::stats();
    assert_eq!((stats.lazy_forks, stats.plain_forks, stats.readers), (1, 0, 1));
    assert_eq!(file_heap::fold(), None, "the file does not change while a child reads it");

    // With that child alive a fork is the kernel's own, asked or not: its
    // child inherits the heap's pages and reads the parent's bytes of the
    // moment, and the first child is undisturbed.
    let second = lazy(|held| {
        let now = all(text, b'c');
        held();
        inherited() && now && all(text, b'c')
    });
    text.fill(b'd');
    let stats = file_heap::stats();
    assert_eq!((stats.lazy_forks, stats.plain_forks, stats.readers), (1, 1, 2));
    assert!(second.ok(), "the child of a fork with a reader alive");
    assert!(child.ok(), "the first child");
    assert!(all(text, b'd'), "no child's write reached the parent");
    assert_eq!(file_heap::stats().readers, 0);
    assert!(file_heap::fold().is_some(), "with the children gone the file follows the heap again");

    // A fork nobody asked to be lazy is the kernel's own, and so is another
    // thread's, asked or not: the range is inherited again after a lazy fork.
    assert!(fork(|_| inherited() && all(text, b'd')).ok(), "the writer's ordinary fork");
    let text = &*text;
    let other = std::thread::scope(|scope| scope.spawn(|| lazy(|_| inherited() && all(text, b'd')).ok()).join().unwrap());
    assert!(other, "the child of another thread's fork");
    let stats = file_heap::stats();
    assert_eq!((stats.lazy_forks, stats.plain_forks), (1, 3));
}

/// Forks that meet the heap mid-change are the kernel's own, and their
/// children see the heap at the fork itself.
fn racing_forks() {
    // Another thread forks while the writer maps a fresh span of the range,
    // after the heap's prepare step has run: the child has the span, as it
    // has any memory mapped before the fork.
    let (ask_read, ask_write) = std::io::pipe().unwrap();
    let (answer_read, answer_write) = std::io::pipe().unwrap();
    ASK.store(ask_write.as_raw_fd(), Ordering::Relaxed);
    ANSWER.store(answer_read.as_raw_fd(), Ordering::Relaxed);
    let (fresh, child_ok) = std::thread::scope(|scope| {
        let forker = scope.spawn(|| {
            MODE.store(ALLOCATE, Ordering::Relaxed);
            fork(|_| {
                let at = PROBE_AT.load(Ordering::Relaxed);
                // SAFETY: the writer's live block of 64 MB at AT, as inherited.
                inherited() && all(unsafe { std::slice::from_raw_parts(at as *const u8, 64 * MB) }, 0x42)
            }).ok()
        });
        read_byte(&ask_read);
        let fresh = vec![0x42u8; 64 * MB];
        PROBE_AT.store(fresh.as_ptr() as usize, Ordering::Relaxed);
        write_byte(&answer_write);
        let child_ok = forker.join().unwrap();
        (fresh, child_ok)
    });
    assert!(file_heap::owns(fresh.as_ptr() as usize), "the block is the writer's");
    assert!(child_ok, "the child of a fork made while the writer mapped a fresh span");

    // The writer's fork that nobody asked to be lazy shows its child the heap
    // as the fork makes it, writes made while it was being prepared included.
    let cell = Box::new(AtomicUsize::new(1));
    assert!(file_heap::fold().is_some());
    PROBE_AT.store(&raw const *cell as usize, Ordering::Relaxed);
    MODE.store(STORE, Ordering::Relaxed);
    assert!(fork(|_| cell.load(Ordering::Relaxed) == 2).ok(), "the child saw the write made after the heap's prepare step");
}

/// A request for a lazy fork ends with the call that made it, however that
/// call ends: the next fork is the kernel's own.
fn unwinding() {
    let before = file_heap::stats();
    let unwound = std::panic::catch_unwind(|| file_heap::lazily(|| std::panic::resume_unwind(Box::new(()))));
    assert!(unwound.is_err());
    assert!(fork(|_| inherited()).ok(), "the fork after an unwound request");
    let after = file_heap::stats();
    assert_eq!((after.lazy_forks, after.plain_forks), (before.lazy_forks, before.plain_forks + 1));
}

/// Children's children, children that exec, and children gone.
fn descendants(text: &mut [u8]) {
    // Children that are gone hold nothing: after more forks than there are
    // places to track a reader, each child reaped, the file follows the heap.
    let many = std::thread::scope(|scope| scope.spawn(|| (0..300).all(|_| fork(|_| true).ok())).join().unwrap());
    assert!(many, "three hundred children");
    assert_eq!(file_heap::stats().readers, 0);
    assert!(file_heap::fold().is_some(), "no reader is left, and none was lost count of");

    // A child's own child inherits its parent's heap, private writes and
    // all, and is a reader of the file after its parent is gone. Fresh
    // memory in a child is the child's, and zeroed.
    rustix::process::set_child_subreaper(Some(rustix::process::getpid())).unwrap();
    let (released, release) = std::io::pipe().unwrap();
    let nested = lazy(|_| {
        text.fill(b'n');
        let fresh = vec![0u8; 8 * MB];
        let text = &*text;
        fork(|_| {
            // SAFETY: closes this process's copy of the write end, as `fork` does for its own pipe.
            unsafe { libc::close(release.as_raw_fd()) };
            read_byte(&released);
            all(text, b'n')
        });
        file_heap::owns(fresh.as_ptr() as usize) && all(&fresh, 0)
    });
    assert!(nested.ok(), "a child's fresh memory");
    text.fill(b'e');
    assert_eq!(file_heap::stats().readers, 1, "the grandchild outlives its parent and still reads the file");
    assert_eq!(file_heap::fold(), None);
    write_byte(&release);
    assert!(reaped(None), "the grandchild read its parent's heap");
    assert_eq!(file_heap::stats().readers, 0);

    // A child that execs reads the file no more; its arguments, which are
    // the writer's memory, reached it as any fork's would.
    let script = format!("echo running; read line; exit 7 # {}", "x".repeat(64 << 10));
    assert!(file_heap::owns(script.as_ptr() as usize));
    let mut command = std::process::Command::new("/bin/sh");
    command.args(["-c", &script]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped());
    // SAFETY: the closure does nothing; it is there so that the child is forked, not spawned.
    unsafe { command.pre_exec(|| Ok(())) };
    let mut shell = command.spawn().expect("a shell");
    let mut line = String::new();
    BufReader::new(shell.stdout.take().unwrap()).read_line(&mut line).unwrap();
    assert_eq!((line.as_str(), file_heap::stats().readers), ("running\n", 0), "the shell runs, and is no reader");
    assert!(file_heap::fold().is_some());
    drop(shell.stdin.take());
    assert_eq!(shell.wait().unwrap().code(), Some(7), "the shell got its script");
}

/// Freed memory leaves the file only when no child reads it, and the
/// allocator's zeroed memory is zeros either way. Any thread may free it.
fn purges() {
    let len = 64 * MB;
    let small = vec![9u8; 4 * MB];
    let big = vec![9u8; len];
    let both = (len + 4 * MB) as u64;
    let reader = lazy(|held| {
        held();
        all(&big, 9) && all(&small, 9)
    });
    // Freed by another thread, as the collector's threads free the VM's blocks.
    let before = file_heap::stats();
    std::thread::spawn(move || drop((big, small))).join().unwrap();
    romacs_allocator::release_free();
    let stats = file_heap::stats();
    assert!(stats.kept >= before.kept + both && stats.punched == before.punched, "purged under a reader: {before:?} {stats:?}");
    assert!(all(&vec![0u8; len], 0) && all(&vec![0u8; 4 * MB], 0), "zeroed memory over pages the file kept");
    assert!(reader.ok(), "the reader still had the purged bytes");
    let before = file_heap::stats();
    let (big, small) = (vec![9u8; len], vec![9u8; 4 * MB]);
    std::thread::spawn(move || drop((big, small))).join().unwrap();
    romacs_allocator::release_free();
    let stats = file_heap::stats();
    assert!(stats.punched >= before.punched + both, "purged with no reader: {before:?} {stats:?}");
    assert!(all(&vec![0u8; len], 0) && all(&vec![0u8; 4 * MB], 0), "zeroed memory over punched pages");

    // A page the kernel will not drop keeps its bytes: the purge says so,
    // and zeroed memory over it is zeros all the same.
    let locked = vec![9u8; len];
    let page = (locked.as_ptr() as usize).next_multiple_of(4096);
    // SAFETY: locks one page of this function's live block.
    let pinned = unsafe { libc::mlock(page as *const c_void, 4096) } == 0;
    let before = file_heap::stats();
    drop(locked);
    romacs_allocator::release_free();
    let after = file_heap::stats();
    let again = vec![0u8; len];
    // SAFETY: unlocks the page locked above; the memory is the allocator's again.
    unsafe { libc::munlock(page as *const c_void, 4096) };
    assert!(pinned, "one page can be locked");
    assert!(after.kept > before.kept, "a purge that could not drop a page keeps it: {before:?} {after:?}");
    assert!(all(&again, 0), "zeroed memory over a page the kernel would not drop");
}

/// A purge on another thread while a fork holds the heap's lock does not
/// wait for it: it keeps the file's pages, as for a reader, and the fork goes on.
fn purge_under_the_lock() {
    let (ask_read, ask_write) = std::io::pipe().unwrap();
    let (answer_read, answer_write) = std::io::pipe().unwrap();
    ASK.store(ask_write.as_raw_fd(), Ordering::Relaxed);
    ANSWER.store(answer_read.as_raw_fd(), Ordering::Relaxed);
    let block = vec![6u8; 64 * MB];
    // What the fork gives the child is asked about a block that stays.
    let anchor = vec![1u8; MB];
    assert!(file_heap::owns(anchor.as_ptr() as usize));
    PROBE_AT.store(anchor.as_ptr() as usize, Ordering::Relaxed);
    let before = file_heap::stats();
    let child_ok = std::thread::scope(|scope| {
        scope.spawn(move || {
            read_byte(&ask_read);
            drop(block);
            romacs_allocator::release_free();
            write_byte(&answer_write);
        });
        MODE.store(PURGE, Ordering::Relaxed);
        lazy(|_| !inherited() && all(&anchor, 1)).ok()
    });
    let after = file_heap::stats();
    assert!(child_ok, "the child of a fork a purge ran under");
    assert!(after.kept >= before.kept + 64 * MB as u64 && after.punched == before.punched, "purged under the lock: {before:?} {after:?}");
}

/// A scan the kernel refuses ends folding for good, and every fork after it
/// is the kernel's own, its child whole.
fn refused_scan() {
    let mut block = vec![3u8; 8 * MB];
    let page = (block.as_ptr() as usize).next_multiple_of(4096) + 4096;
    // A page of the range the kernel does not watch: its scan is refused.
    // SAFETY: one page inside this function's own block, whose bytes there it does not read again.
    let unwatched = unsafe {
        libc::mmap(page as *mut c_void, 4096, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_FIXED, -1, 0)
    };
    assert_eq!(unwatched as usize, page);
    let first = (block.as_ptr() as usize).next_multiple_of(4096) - block.as_ptr() as usize;
    block[first..first + 4096].fill(4);
    assert_eq!(file_heap::fold(), None, "a refused scan");
    block[first..first + 4096].fill(5);
    assert_eq!(file_heap::fold(), None, "and no fold after it");
    assert_eq!(file_heap::stats().failed_folds, 1);
    let lazy_before = file_heap::stats().lazy_forks;
    let page_bytes = &block[first..first + 4096];
    assert!(lazy(|_| all(page_bytes, 5)).ok(), "the child of a fork asked to be lazy, after the refusal");
    assert_eq!(file_heap::stats().lazy_forks, lazy_before, "no fork is lazy after a refused scan");
}

/// A span the kernel will not watch is memory all the same, and counted:
/// no fold would see its writes.
fn unwatched_span() {
    let before = file_heap::stats();
    REFUSE_WATCH.store(true, Ordering::Relaxed);
    let mut block = vec![0u8; 2 << 30];
    REFUSE_WATCH.store(false, Ordering::Relaxed);
    block[..4096].fill(8);
    let after = file_heap::stats();
    assert!(after.mapped > before.mapped, "the block took a fresh span: {before:?} {after:?}");
    assert_eq!(after.failed_folds, before.failed_folds + 1, "the unwatched span is counted");
    assert!(all(&block[..4096], 8) && all(&block[4096..MB], 0));
    assert_eq!(file_heap::fold(), None);
}
