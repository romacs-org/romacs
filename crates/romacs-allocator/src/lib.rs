//! The production allocator for Romacs: jemalloc, with its statistics.

pub use tikv_jemallocator::Jemalloc as PlatformAllocator;

pub const PLATFORM_ALLOCATOR: PlatformAllocator = PlatformAllocator;

/// Read the `usize` jemalloc statistic NAME, or None when jemalloc refuses.
fn read_usize(name: &std::ffi::CStr) -> Option<usize> {
    let mut value = 0usize;
    let mut len = size_of::<usize>();
    // SAFETY: NAME is NUL-terminated; `value` and `len` are live locals and
    // `len` is `value`'s size, the size jemalloc's `size_t` statistics have.
    // No new value is written (null, 0).
    let rc = unsafe {
        tikv_jemalloc_sys::mallctl(name.as_ptr(), (&raw mut value).cast(), &mut len, std::ptr::null_mut(), 0)
    };
    (rc == 0).then_some(value)
}

/// The bytes the allocator's blocks hold and the bytes it keeps
/// resident, as of now.
pub fn stats() -> Option<(usize, usize)> {
    let mut epoch = 1u64;
    let mut len = size_of::<u64>();
    // SAFETY: "epoch" reads and writes a `uint64_t`; both pointers address
    // the live local `epoch`, and `len` is its size.
    unsafe {
        tikv_jemalloc_sys::mallctl(c"epoch".as_ptr(), (&raw mut epoch).cast(), &mut len, (&raw mut epoch).cast(), len);
    }
    Some((read_usize(c"stats.allocated")?, read_usize(c"stats.resident")?))
}

/// Give the pages of every free extent back to the system now,
/// instead of after jemalloc's decay delay.
pub fn release_free() {
    // MALLCTL_ARENAS_ALL: every arena.
    let name = c"arena.4096.purge";
    // SAFETY: "arena.<i>.purge" takes and returns no value, so every
    // pointer is null and the length 0.
    unsafe {
        tikv_jemalloc_sys::mallctl(name.as_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0);
    }
}
