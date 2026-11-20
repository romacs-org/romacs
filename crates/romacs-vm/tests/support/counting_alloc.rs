//! A global allocator that counts, for a test that holds an exact number of
//! allocations: what a piece of code allocates on its thread and in the
//! whole process, and the bytes it leaves live (`measure`). A test file
//! installs it as its executable's `#[global_allocator]`, so it is a test
//! target of its own (`tests/all/standalone.txt`).
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

pub struct Counting;

/// Allocations and reallocations made, by every thread.
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
/// Bytes live.
static LIVE: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static TRACKED: Cell<usize> = const { Cell::new(0) };
}

/// What F returns; the allocations and reallocations it made on this
/// thread, and in the whole process; and how many more bytes are live
/// after it than before (negative when fewer).
pub fn measure<T>(f: impl FnOnce() -> T) -> (T, usize, usize, isize) {
    let (allocs, live) = (ALLOCS.load(Relaxed), LIVE.load(Relaxed));
    TRACKED.set(0);
    TRACKING.set(true);
    let result = f();
    TRACKING.set(false);
    let grown = LIVE.load(Relaxed).wrapping_sub(live).cast_signed();
    (result, TRACKED.get(), ALLOCS.load(Relaxed) - allocs, grown)
}

/// Count one allocation or reallocation that made GREW bytes live and
/// SHRANK bytes free.
fn counted(grew: usize, shrank: usize) {
    ALLOCS.fetch_add(1, Relaxed);
    LIVE.fetch_add(grew, Relaxed);
    LIVE.fetch_sub(shrank, Relaxed);
    // A thread's locals may be gone as it exits: then it counts nothing.
    let _ = TRACKING.try_with(|on| if on.get() { let _ = TRACKED.try_with(|n| n.set(n.get() + 1)); });
}

// UNSAFE(GlobalAlloc is an unsafe trait: an allocator promises valid, disjoint blocks)
// SAFETY: every block comes from the system allocator and goes back to it
// unchanged; this only counts.
unsafe impl GlobalAlloc for Counting {
    // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
    // SAFETY: the caller's contract is GlobalAlloc::alloc's.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
        // SAFETY: LAYOUT is the caller's, passed on unchanged.
        let at = unsafe { System.alloc(layout) };
        if !at.is_null() {
            counted(layout.size(), 0);
        }
        at
    }

    // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
    // SAFETY: the caller's contract is GlobalAlloc::dealloc's.
    unsafe fn dealloc(&self, at: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
        // SAFETY: AT and LAYOUT are the caller's, a block the system allocator made.
        unsafe { System.dealloc(at, layout) }
    }

    // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
    // SAFETY: the caller's contract is GlobalAlloc::realloc's.
    unsafe fn realloc(&self, at: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // UNSAFE(GlobalAlloc's methods are unsafe fns by the trait)
        // SAFETY: the arguments are the caller's, a block the system allocator made.
        let moved = unsafe { System.realloc(at, layout, size) };
        if !moved.is_null() {
            counted(size, layout.size());
        }
        moved
    }
}
