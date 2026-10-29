use crate::Vm;

pub(super) fn snapshot() -> Vec<Vec<u8>> {
    crate::host::env_snapshot().into_iter().map(|(name, value)| {
        let mut entry = name.into_encoded_bytes();
        entry.push(b'=');
        entry.extend(value.into_encoded_bytes());
        entry
    }).collect()
}

/// callproc.c init_callproc_1: decode_env_path ("PATH", default_PATH, false).
/// An unset or empty PATH uses the Linux default; an empty component is ".".
/// GNU appends its own libexec helper directory; Romacs installs none.
fn exec_path(path: Option<&[u8]>) -> Vec<Vec<u8>> {
    let path = path.filter(|p| !p.is_empty()).unwrap_or(b"/bin:/usr/bin");
    path.split(|&b| b == b':').map(|d| if d.is_empty() { b".".to_vec() } else { d.to_vec() }).collect()
}

pub(crate) fn multibyte(bytes: &[u8]) -> bool {
    if bytes.is_ascii() { return false; }
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let Some((code, width)) = crate::lstring::decode_char(remaining) else { return false; };
        if code > crate::lstring::MAX_5_BYTE_CHAR { return false; }
        remaining = &remaining[width..];
    }
    true
}

impl Vm {
    pub(super) fn initialize_environment_slot(&mut self, name: &str) {
        if !matches!(name, "process-environment" | "initial-environment" | "exec-path") { return; }
        let Some(entries) = self.environment_snapshot.take() else { return; };
        let path = entries.iter().rev().find_map(|entry| entry.strip_prefix(b"PATH=")).map(<[u8]>::to_vec);
        let values = entries.into_iter().rev().map(|bytes| {
            let multibyte = multibyte(&bytes);
            self.make_lisp_string(bytes, multibyte)
        }).collect::<Vec<_>>();
        let process = self.list_value(&values);
        let initial = self.list_value(&values);
        self.declare_special_raw("process-environment", process);
        self.declare_special_raw("initial-environment", initial);
        let directories = exec_path(path.as_deref()).into_iter().map(|bytes| {
            let multibyte = multibyte(&bytes);
            self.make_lisp_string(bytes, multibyte)
        }).collect::<Vec<_>>();
        let exec = self.list_value(&directories);
        self.declare_special_raw("exec-path", exec);
    }
}

/// GNU's emacs_wd, as glibc's get_current_dir_name gives it at startup:
/// PWD when it names the same file as ".", so a directory reached through a
/// symlink keeps its spelling, else getcwd. Captured once, by the first
/// `Vm::new` as the host starts; VM2 never changes the process working
/// directory. None when it cannot be determined, or names an unreachable
/// directory (a leading `(', emacs_get_current_dir_name).
pub(crate) fn startup_directory() -> Option<&'static [u8]> {
    use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
    static DIRECTORY: std::sync::OnceLock<Option<Vec<u8>>> = std::sync::OnceLock::new();
    DIRECTORY.get_or_init(|| {
        let dot = crate::host::metadata(".").ok();
        if let (Some(pwd), Some(dot)) = (crate::host::getenv("PWD"), dot.as_ref())
            && let Ok(meta) = crate::host::metadata(&pwd)
            && meta.dev() == dot.dev() && meta.ino() == dot.ino()
        {
            return Some(pwd.as_bytes().to_vec());
        }
        crate::host::current_dir().ok().map(|dir| dir.as_os_str().as_bytes().to_vec())
    }).as_deref().filter(|dir| dir.first() != Some(&b'('))
}
