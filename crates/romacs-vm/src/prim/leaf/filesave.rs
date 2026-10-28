use super::*;

impl Leaf<'_> {
    pub fn filesave_mkdir(&self, path: &str) -> std::io::Result<()> {
        crate::host::create_dir(path, 0o777)
    }
    pub fn filesave_unlink(&self, path: &str) -> std::io::Result<()> {
        crate::host::remove_file(path)
    }
    pub fn filesave_access(&self, path: &str, mode: i32) -> std::io::Result<()> {
        crate::host::access(&c_path(path)?, mode, crate::host::Ids::Effective)
    }
    pub fn filesave_chmod(&self, path: &str, mode: u32, nofollow: bool) -> std::io::Result<()> {
        crate::host::chmod(&c_path(path)?, mode, nofollow)
    }
    pub fn filesave_rename(&self, from: &str, to: &str, replace: bool) -> std::io::Result<()> {
        if replace { return crate::host::rename(from, to) }
        crate::host::rename_noreplace(&c_path(from)?, &c_path(to)?)
    }
    /// utimensat: both times set to TIME (seconds, nanoseconds), or now.
    pub fn filesave_utimens(&self, path: &str, time: Option<(i64, i64)>, nofollow: bool) -> std::io::Result<()> {
        crate::host::set_times(&c_path(path)?, time, nofollow)
    }
}

fn c_path(path: &str) -> std::io::Result<std::ffi::CString> {
    std::ffi::CString::new(path).map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))
}
