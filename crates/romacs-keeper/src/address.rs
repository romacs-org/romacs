//! The keeper's socket address, whatever the depth of its path.
//!
//! A unix socket address holds a path of 107 bytes, and a runtime
//! directory can be deeper than that. So the keeper binds, and a session
//! connects, through a descriptor of the socket's directory:
//! `/proc/self/fd/N/NAME` names the same file in under 40 bytes, and the
//! directory's own permissions still decide who may reach it. Both sides
//! name the socket by its whole path, so one path is one keeper. No
//! `chdir`: the session has other threads.

use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// Run WITH on a short name for PATH, made through a descriptor of its
/// directory that lives as long as the call.
fn through_directory<T>(path: &Path, with: impl FnOnce(PathBuf) -> io::Result<T>) -> io::Result<T> {
    let directory = path.parent().map(|d| if d.as_os_str().is_empty() { Path::new(".") } else { d });
    let (Some(directory), Some(name)) = (directory, path.file_name()) else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{} is not a socket's path", path.display())));
    };
    let directory = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_PATH | libc::O_DIRECTORY).open(directory)?;
    with(Path::new("/proc/self/fd").join(directory.as_raw_fd().to_string()).join(name))
}

/// Listen at PATH.
///
/// # Errors
/// The system's refusal to open PATH's directory, to bind or to listen.
pub fn bind(path: &Path) -> io::Result<UnixListener> { through_directory(path, UnixListener::bind) }

/// Connect to the socket at PATH.
///
/// # Errors
/// The system's refusal to open PATH's directory or to connect.
pub fn connect(path: &Path) -> io::Result<UnixStream> { through_directory(path, UnixStream::connect) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_deeper_than_an_address_binds_and_connects() {
        let base = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
        let top = base.join(format!("rk-address-{}", std::process::id()));
        let directory = top.join("d".repeat(60)).join("e".repeat(60));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("keeper.sock");
        assert!(path.as_os_str().len() > 107);
        assert!(UnixListener::bind(&path).is_err(), "too long for an address by itself");
        let listener = bind(&path).unwrap();
        assert!(path.exists(), "bound at the path itself");
        let _client = connect(&path).unwrap();
        assert!(listener.accept().is_ok());
        std::fs::remove_dir_all(&top).unwrap();
    }
}
