//! Romacs Icons, the mode line's icon font (DIVERGENCES.md VM2-MODE-LINE),
//! as build.rs builds it from assets/icons: one glyph a drawing, each one
//! text column wide, at the codepoints assets/icons/codepoints.txt gives.

use std::path::{Path, PathBuf};

/// The font file's bytes.
pub const TTF: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/RomacsIcons.ttf"));

/// The calls of `file_in` in this process, which name their temporary files.
static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The font as a file in DIR, for a catalog that reads fonts from files:
/// named by its contents (FNV-1a), so binaries with other drawings keep
/// their own, and written once, through a temporary name of this call's
/// own, so no reader, and no other writer, finds part of it.
///
/// # Errors
///
/// When DIR cannot be made or written.
pub fn file_in(dir: &Path) -> std::io::Result<PathBuf> {
    let digest = TTF.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3));
    let path = dir.join(format!("RomacsIcons-{digest:016x}.ttf"));
    if std::fs::read(&path).is_ok_and(|bytes| bytes == TTF) {
        return Ok(path);
    }
    std::fs::create_dir_all(dir)?;
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let partial = dir.join(format!(".RomacsIcons-{}-{call}.partial", std::process::id()));
    std::fs::write(&partial, TTF)?;
    std::fs::rename(&partial, &path)?;
    Ok(path)
}
