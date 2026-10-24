//! Integration proposal: install as src/prim/leaf/filelock.rs, then `mod filelock;`
//! alongside `mod fileio;` in prim.rs. Never executes Lisp or collects.
use super::*;
impl Leaf<'_> {
    pub(crate) fn filelock_process_exists(&self, pid: i32) -> bool {
        // Positive validated process ID only; signal zero cannot send a signal.
        (unsafe { libc::kill(pid, 0) } == 0)
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    pub(crate) fn filelock_login_name(&self) -> Vec<u8> {
        let mut buffer = vec![0u8; 16384];
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let status = unsafe { libc::getpwuid_r(libc::geteuid(), entry.as_mut_ptr(), buffer.as_mut_ptr().cast(), buffer.len(), &mut result) };
        if status != 0 || result.is_null() { return b"unknown".to_vec(); }
        unsafe { std::ffi::CStr::from_ptr((*result).pw_name) }.to_bytes().to_vec()
    }
    pub(crate) fn filelock_create_nonce(&self, path: &std::path::Path) -> std::io::Result<(std::path::PathBuf, std::fs::File)> {
        use std::os::{fd::FromRawFd, unix::ffi::{OsStrExt, OsStringExt}};
        let parent = path.parent().unwrap_or(std::path::Path::new("."));
        let mut template = parent.join(".#-emacsXXXXXX").as_os_str().as_bytes().to_vec();
        template.push(0);
        let fd = unsafe { libc::mkostemp(template.as_mut_ptr().cast(), libc::O_CLOEXEC) };
        if fd < 0 { return Err(std::io::Error::last_os_error()); }
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        template.pop();
        Ok((std::path::PathBuf::from(std::ffi::OsString::from_vec(template)), file))
    }
}
