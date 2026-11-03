//! Target-aware production allocator selection for Romacs.

cfg_select! {
    target_os = "linux" => {
        pub use tikv_jemallocator::Jemalloc as PlatformAllocator;

        pub const PLATFORM_ALLOCATOR: PlatformAllocator = PlatformAllocator;

        /// The bytes the allocator's blocks hold and the bytes it keeps
        /// resident, as of now.
        pub fn stats() -> Option<(usize, usize)> {
            let read = |name: &std::ffi::CStr| {
                let mut value = 0usize;
                let mut len = size_of::<usize>();
                let rc = unsafe {
                    tikv_jemalloc_sys::mallctl(name.as_ptr(), (&raw mut value).cast(), &mut len, std::ptr::null_mut(), 0)
                };
                (rc == 0).then_some(value)
            };
            let mut epoch = 1u64;
            let mut len = size_of::<u64>();
            unsafe {
                tikv_jemalloc_sys::mallctl(c"epoch".as_ptr(), (&raw mut epoch).cast(), &mut len, (&raw mut epoch).cast(), len);
            }
            Some((read(c"stats.allocated")?, read(c"stats.resident")?))
        }

        /// Give the pages of every free extent back to the system now,
        /// instead of after jemalloc's decay delay.
        pub fn release_free() {
            // MALLCTL_ARENAS_ALL: every arena.
            let name = c"arena.4096.purge";
            unsafe {
                tikv_jemalloc_sys::mallctl(name.as_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0);
            }
        }
    }
    _ => {
        pub use mimalloc::MiMalloc as PlatformAllocator;

        pub const PLATFORM_ALLOCATOR: PlatformAllocator = PlatformAllocator;

        /// mimalloc returns free pages on its own schedule.
        pub fn release_free() {}

        pub fn stats() -> Option<(usize, usize)> {
            None
        }
    }
}
