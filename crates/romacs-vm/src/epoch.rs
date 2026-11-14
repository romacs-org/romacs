//! The collections' epoch in this process.
//!
//! Storage that no table entry owns alone (a char-table node, a text body's
//! properties) cannot be given a colour, so it carries the epoch at which it
//! was made or last written. Every collection of any VM advances the epoch
//! and the VM remembers where its own ended: storage stamped below that was
//! last written before the collection and holds only what it kept, so a
//! minor passes over it; storage stamped at or above it is read. Another
//! VM's collections only raise later stamps: more looks written, never less.
use std::sync::atomic::{AtomicU64, Ordering};

static EPOCH: AtomicU64 = AtomicU64::new(1);

/// The stamp of storage made or written now.
pub(crate) fn born() -> u64 {
    EPOCH.load(Ordering::Relaxed)
}
/// A collection ends: the epoch it leaves, which its VM remembers.
pub(crate) fn commit_point() -> u64 {
    EPOCH.fetch_add(1, Ordering::Relaxed) + 1
}
/// A VM restored from an image remembers the epoch of the process that
/// built it: this process's is raised to it, so what the VM writes from now
/// on is stamped at or above what it remembers.
pub(crate) fn raise(to: u64) {
    EPOCH.fetch_max(to, Ordering::Relaxed);
}
