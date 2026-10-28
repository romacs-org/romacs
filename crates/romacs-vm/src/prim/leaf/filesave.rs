use super::*;

impl Leaf<'_> {
    pub fn filesave_mkdir(&self, path: &str) -> std::io::Result<()> {
        crate::host::create_dir(path, 0o777)
    }
    pub fn filesave_unlink(&self, path: &str) -> std::io::Result<()> {
        crate::host::remove_file(path)
    }
    pub fn filesave_access(&self, path: &str, mode: i32) -> std::io::Result<()> {
        let path = std::ffi::CString::new(path).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        crate::host::access(&path, mode)
    }
    pub fn filesave_chmod(&self, path: &str, mode: u32, nofollow: bool) -> std::io::Result<()> {
        let path = std::ffi::CString::new(path).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        if unsafe { libc::fchmodat(libc::AT_FDCWD, path.as_ptr(), mode as libc::mode_t, if nofollow { libc::AT_SYMLINK_NOFOLLOW } else { 0 }) } == 0 { Ok(()) }
        else { Err(std::io::Error::last_os_error()) }
    }
    pub fn filesave_rename(&self, from: &str, to: &str, replace: bool) -> std::io::Result<()> {
        if replace { return crate::host::rename(from, to) }
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
}
