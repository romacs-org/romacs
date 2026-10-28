use super::*;

impl Leaf<'_> {
    pub fn filesave_mkdir(&self, path: &str) -> std::io::Result<()> {
        std::fs::create_dir(path)
    }
    pub fn filesave_unlink(&self, path: &str) -> std::io::Result<()> {
        std::fs::remove_file(path)
    }
    pub fn filesave_access(&self, path: &str, mode: i32) -> std::io::Result<()> {
        let path = std::ffi::CString::new(path).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        if unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), mode, libc::AT_EACCESS) } == 0 { Ok(()) }
        else { Err(std::io::Error::last_os_error()) }
    }
    pub fn filesave_chmod(&self, path: &str, mode: u32, nofollow: bool) -> std::io::Result<()> {
        let path = std::ffi::CString::new(path).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        if unsafe { libc::fchmodat(libc::AT_FDCWD, path.as_ptr(), mode as libc::mode_t, if nofollow { libc::AT_SYMLINK_NOFOLLOW } else { 0 }) } == 0 { Ok(()) }
        else { Err(std::io::Error::last_os_error()) }
    }
    /// umask: replace the process's file mode creation mask.
    pub fn filesave_set_umask(&self, mask: u32) {
        unsafe { libc::umask(mask as libc::mode_t) };
    }
    pub fn filesave_rename(&self, from: &str, to: &str, replace: bool) -> std::io::Result<()> {
        if replace { return std::fs::rename(from, to) }
        let from = std::ffi::CString::new(from).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        let to = std::ffi::CString::new(to).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        if unsafe { libc::renameat2(libc::AT_FDCWD, from.as_ptr(), libc::AT_FDCWD, to.as_ptr(), libc::RENAME_NOREPLACE) } == 0 { Ok(()) }
        else { Err(std::io::Error::last_os_error()) }
    }
    /// utimensat: both times set to TIME (seconds, nanoseconds), or now.
    pub fn filesave_utimens(&self, path: &str, time: Option<(i64, i64)>, nofollow: bool) -> std::io::Result<()> {
        let path = std::ffi::CString::new(path).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        let stamp = match time {
            Some((seconds, nanos)) => libc::timespec { tv_sec: seconds as libc::time_t, tv_nsec: nanos as _ },
            None => libc::timespec { tv_sec: 0, tv_nsec: libc::UTIME_NOW },
        };
        let flag = if nofollow { libc::AT_SYMLINK_NOFOLLOW } else { 0 };
        if unsafe { libc::utimensat(libc::AT_FDCWD, path.as_ptr(), [stamp, stamp].as_ptr(), flag) } == 0 { Ok(()) }
        else { Err(std::io::Error::last_os_error()) }
    }
    /// GNU copy-file's fast paths: share SOURCE's extents with DEST, else
    /// copy_file_range until it stops. False leaves the rest to read+write.
    pub fn filesave_clone(&self, dest: &std::fs::File, source: &std::fs::File) -> bool {
        use std::os::fd::AsRawFd as _;
        if unsafe { libc::ioctl(dest.as_raw_fd(), libc::FICLONE, source.as_raw_fd()) } == 0 {
            return true;
        }
        const COPY_MAX: usize = (isize::MAX as usize) >> 30 << 30;
        while unsafe { libc::copy_file_range(source.as_raw_fd(), std::ptr::null_mut(), dest.as_raw_fd(), std::ptr::null_mut(), COPY_MAX, 0) } > 0 {}
        false
    }
    /// Whether FILE carries an extended (POSIX or NFSv4) ACL.
    pub fn filesave_has_acl(&self, file: &std::fs::File) -> bool {
        use std::os::fd::AsRawFd as _;
        [c"system.posix_acl_access", c"system.nfs4_acl"].iter().any(|attribute| {
            let size = unsafe { libc::fgetxattr(file.as_raw_fd(), attribute.as_ptr(), std::ptr::null_mut(), 0) };
            size >= 0
        })
    }
}
