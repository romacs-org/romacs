use sha2::{Digest, Sha256};
use std::{ffi::OsString, fmt::Write, io};

pub const DIRECTORY: &str = "etc/romacs-runtime-epoch.d";

pub fn digest(names: impl Iterator<Item = io::Result<OsString>>) -> io::Result<String> {
    let mut names = names
        .map(|name| name.map(OsString::into_encoded_bytes))
        .collect::<io::Result<Vec<_>>>()?;
    if names.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "no epoch fragments"));
    }
    names.sort_unstable();
    let mut hash = Sha256::new();
    for name in names {
        hash.update(name);
        hash.update([0]);
    }
    let bytes = hash.finalize();
    let mut digest = String::with_capacity(2 * bytes.len());
    for byte in bytes {
        write!(digest, "{byte:02x}").map_err(io::Error::other)?;
    }
    Ok(digest)
}
