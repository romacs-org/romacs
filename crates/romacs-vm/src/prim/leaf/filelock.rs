//! Integration proposal: install as src/prim/leaf/filelock.rs, then `mod filelock;`
//! alongside `mod fileio;` in prim.rs. Never executes Lisp or collects.
use super::*;
impl Leaf<'_> {
    pub(crate) fn filelock_process_exists(&self, pid: i32) -> bool {
        // Positive validated process ID only; signal zero cannot send a signal.
        crate::host::kill(pid, 0).map_or_else(|e| e.raw_os_error() == Some(libc::EPERM), |()| true)
    }
    pub(crate) fn filelock_login_name(&self) -> Vec<u8> {
        crate::host::user_by_uid(crate::host::euid(), 16384).map_or_else(|_| b"unknown".to_vec(), |entry| entry.name.unwrap_or_default())
    }
    pub(crate) fn filelock_create_nonce(&self, path: &std::path::Path) -> std::io::Result<(std::path::PathBuf, std::fs::File)> {
        let parent = path.parent().unwrap_or(std::path::Path::new("."));
        crate::host::temp_file(&parent.join(".#-emacsXXXXXX"))
    }
}
