//! `sweep`: the attach cache kept bounded off the evaluator (attachments.md
//! §2.4).  Staging directories of processes that are gone are removed;
//! `received/`, once over its byte budget or its file count, is cut to 80%
//! of both, least recently used (oldest mtime) first.  `received/.partial/`
//! (fetches not yet published) is left alone.
use std::path::Path;

/// What `received/` holds after a sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Totals {
    pub bytes: u64,
    pub files: u64,
}

/// Whether process PID is alive (signal 0 reaches it, or exists but is
/// another user's).
fn alive(pid: i32) -> bool {
    // SAFETY: kill with signal 0 only checks that the process exists.
    let reached = unsafe { libc::kill(pid, 0) == 0 };
    reached || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// ROOT's `staging/<pid>` directories whose process is gone, removed.
fn sweep_staging(root: &Path) -> std::io::Result<()> {
    let Ok(entries) = std::fs::read_dir(root.join("staging")) else { return Ok(()) };
    for entry in entries.flatten() {
        let dead = entry.file_name().to_str().and_then(|n| n.parse::<i32>().ok()).is_some_and(|pid| pid > 0 && !alive(pid));
        if dead && entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(())
}

/// The files to remove from FILES (mtime, bytes, path), oldest first, so
/// that what stays is within 80% of BUDGET bytes and MAX_FILES files; none
/// unless one bound is passed.
#[must_use]
pub fn evictions<P: Clone>(mut files: Vec<(std::time::SystemTime, u64, P)>, budget: u64, max_files: u64) -> Vec<P> {
    let (mut bytes, mut count) = (files.iter().map(|f| f.1).sum::<u64>(), files.len() as u64);
    if bytes <= budget && count <= max_files {
        return vec![];
    }
    files.sort_by_key(|f| f.0);
    let mut out = vec![];
    for (_, size, path) in files {
        if bytes * 5 <= budget * 4 && count * 5 <= max_files * 4 {
            break;
        }
        bytes -= size;
        count -= 1;
        out.push(path);
    }
    out
}

/// Sweep ROOT (`$XDG_CACHE_HOME/romacs/attach`): its totals after.
pub fn sweep(root: &Path, budget: u64, max_files: u64) -> std::io::Result<Totals> {
    sweep_staging(root)?;
    let mut files = vec![];
    if let Ok(entries) = std::fs::read_dir(root.join("received")) {
        for entry in entries.flatten() {
            let meta = entry.metadata()?;
            if meta.is_file() {
                files.push((meta.modified()?, meta.len(), entry.path()));
            }
        }
    }
    let total = Totals { bytes: files.iter().map(|f| f.1).sum(), files: files.len() as u64 };
    let mut after = total;
    let sizes: std::collections::HashMap<_, _> = files.iter().map(|f| (f.2.clone(), f.1)).collect();
    for path in evictions(files, budget, max_files) {
        std::fs::remove_file(&path)?;
        after.bytes -= sizes[&path];
        after.files -= 1;
    }
    Ok(after)
}
