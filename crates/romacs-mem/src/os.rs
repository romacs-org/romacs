//! Minimal Linux virtual-memory owner. No inherited allocator or build glue.
//! The adopted page geometry assumes a 4 KiB OS page (checked at construction).
use crate::AllocError;
use std::{
    ffi::{c_int, c_long, c_void},
    ptr::NonNull,
};
#[cfg(not(target_os = "linux"))]
compile_error!("romacs-mem S2 virtual-memory backend currently supports Linux only");
const READ_WRITE: c_int = 3;
const MAP_PRIVATE_ANONYMOUS: c_int = 0x22;
unsafe extern "C" {
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: i64,
    ) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
    fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int;
    fn madvise(addr: *mut c_void, len: usize, advice: c_int) -> c_int;
    fn sysconf(name: c_int) -> c_long;
    #[cfg(test)]
    fn mincore(addr: *mut c_void, len: usize, vec: *mut u8) -> c_int;
}
/// Owns exactly one mapped span; never copies, never crosses isolate threads.
pub(crate) struct Mapping {
    pub base: NonNull<u8>,
    pub len: usize,
}
impl Mapping {
    pub fn new(len: usize, align: usize, reserve: bool) -> Result<Self, AllocError> {
        // SAFETY: sysconf has no pointer arguments. Linux _SC_PAGESIZE = 30.
        if unsafe { sysconf(30) } != 4096 {
            return Err(AllocError::UnsupportedPageSize);
        }
        if len == 0 || !len.is_multiple_of(4096) || !align.is_power_of_two() || align < 4096 {
            return Err(AllocError::InvalidSize);
        }
        let total = len.checked_add(align).ok_or(AllocError::MemoryFull)?;
        // SAFETY: anonymous mapping, no file descriptor or borrowed memory.
        let raw = unsafe {
            mmap(
                std::ptr::null_mut(),
                total,
                if reserve { 0 } else { READ_WRITE },
                MAP_PRIVATE_ANONYMOUS,
                -1,
                0,
            )
        };
        if raw as isize == -1 {
            return Err(AllocError::MemoryFull);
        }
        let start = raw as usize;
        let aligned = (start + align - 1) & !(align - 1);
        let prefix = aligned - start;
        let suffix = total - prefix - len;
        // SAFETY: page-aligned subspans of this new mapping, never published.
        unsafe {
            if prefix != 0 {
                assert_eq!(munmap(raw, prefix), 0);
            }
            if suffix != 0 {
                assert_eq!(munmap((aligned + len) as *mut c_void, suffix), 0);
            }
        }
        Ok(Self {
            base: NonNull::new(aligned as *mut u8).ok_or(AllocError::MemoryFull)?,
            len,
        })
    }
    pub fn commit(&self, offset: usize, len: usize) -> Result<(), AllocError> {
        self.range(offset, len);
        // SAFETY: checked page-aligned span is owned by this mapping.
        if unsafe { mprotect(self.base.as_ptr().add(offset).cast(), len, READ_WRITE) } == 0 {
            Ok(())
        } else {
            Err(AllocError::MemoryFull)
        }
    }
    pub fn discard(&self, offset: usize, len: usize) -> Result<(), AllocError> {
        self.range(offset, len);
        // SAFETY: checked private anonymous subspan. Linux MADV_DONTNEED = 4.
        if unsafe { madvise(self.base.as_ptr().add(offset).cast(), len, 4) } == 0 {
            Ok(())
        } else {
            Err(AllocError::MemoryFull)
        }
    }
    #[cfg(test)]
    pub fn resident_pages(&self, offset: usize, len: usize) -> usize {
        self.range(offset, len);
        let mut residency = vec![0_u8; len / 4096];
        // SAFETY: checked mapped range; vector has one writable byte per OS page.
        assert_eq!(
            unsafe {
                mincore(
                    self.base.as_ptr().add(offset).cast(),
                    len,
                    residency.as_mut_ptr(),
                )
            },
            0
        );
        residency.iter().filter(|b| **b & 1 != 0).count()
    }
    fn range(&self, offset: usize, len: usize) {
        assert!(
            offset.is_multiple_of(4096)
                && len.is_multiple_of(4096)
                && offset.checked_add(len).is_some_and(|end| end <= self.len)
        );
    }
}
impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: this owner retained exactly this span after trimming alignment slack.
        let rc = unsafe { munmap(self.base.as_ptr().cast(), self.len) };
        debug_assert_eq!(rc, 0, "munmap failed");
    }
}
