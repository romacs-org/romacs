use super::*;

impl Leaf<'_> {
    pub fn filesave_mkdir(&self, path: &std::path::Path) -> std::io::Result<()> {
        crate::host::create_dir(path, 0o777)
    }
    pub fn filesave_unlink(&self, path: &std::path::Path) -> std::io::Result<()> {
        crate::host::remove_file(path)
    }
    pub fn filesave_access(&self, path: &std::ffi::OsStr, mode: i32) -> std::io::Result<()> {
        crate::host::access(&c_path(path)?, mode, crate::host::Ids::Effective)
    }
    pub fn filesave_chmod(&self, path: &std::path::Path, mode: u32, nofollow: bool) -> std::io::Result<()> {
        crate::host::chmod(&c_path(path.as_os_str())?, mode, nofollow)
    }
    /// umask: replace the process's file mode creation mask.
    pub fn filesave_set_umask(&self, mask: u32) {
        unsafe { libc::umask(mask as libc::mode_t) };
    }
    pub fn filesave_rename(&self, from: &std::path::Path, to: &std::path::Path, replace: bool) -> std::io::Result<()> {
        if replace { return crate::host::rename(from, to) }
        crate::host::rename_noreplace(&c_path(from.as_os_str())?, &c_path(to.as_os_str())?)
    }
    /// utimensat: both times set to TIME (seconds, nanoseconds), or now.
    pub fn filesave_utimens(&self, path: &std::path::Path, time: Option<(i64, i64)>, nofollow: bool) -> std::io::Result<()> {
        crate::host::set_times(&c_path(path.as_os_str())?, time, nofollow)
    }
}

fn c_path(path: &std::ffi::OsStr) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(path.as_bytes()).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))
}
