//! A content-addressed directory: files named `<sha256>.<ext>`, written to
//! a temporary file beside their name and renamed into place, so a reader
//! never sees half a file and two identical writes keep one.
use sha2::{Digest, Sha256};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// A file in the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
}

/// The lowercase hex SHA-256 of BYTES.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// DIR, made (mode 0700) when missing.
fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

/// A temporary name in DIR no other writer uses.
fn temporary(dir: &Path) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    dir.join(format!(".tmp-{}-{n}", std::process::id()))
}

/// Rename TEMPORARY to DIR's name for SHA256 and EXT, unless an identical
/// file is there already (then TEMPORARY goes).
fn publish(dir: &Path, temporary: &Path, sha256: String, ext: &str, bytes: u64) -> std::io::Result<Stored> {
    let path = dir.join(format!("{sha256}.{ext}"));
    if path.exists() {
        std::fs::remove_file(temporary)?;
    } else {
        std::fs::rename(temporary, &path)?;
    }
    Ok(Stored { path, sha256, bytes })
}

/// BYTES in DIR as `<sha256>.<ext>`, mode 0600.
pub fn store(dir: &Path, bytes: &[u8], ext: &str) -> std::io::Result<Stored> {
    ensure_dir(dir)?;
    let temporary = temporary(dir);
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    publish(dir, &temporary, sha256_hex(bytes), ext, bytes.len() as u64)
}

/// The file SRC, whose contents hash to SHA256, copied unchanged into DIR
/// as `<sha256>.<ext>`.  The copy is `copy_file_range`'s, which shares the
/// extents (a reflink) on a filesystem that can, so a large file costs no
/// copy on btrfs.  A source changed since it was hashed is refused.
pub fn store_copy(dir: &Path, src: &Path, sha256: String, ext: &str) -> std::io::Result<Stored> {
    ensure_dir(dir)?;
    let temporary = temporary(dir);
    let bytes = std::fs::copy(src, &temporary)?;
    if sha256_hex(&std::fs::read(&temporary)?) != sha256 {
        std::fs::remove_file(&temporary)?;
        return Err(std::io::Error::other(format!("{} changed while it was copied", src.display())));
    }
    std::fs::set_permissions(&temporary, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    publish(dir, &temporary, sha256, ext, bytes)
}
