//! What became of this process's loads that could run compiled Lisp
//! (`Vm::compiled_load`), for `romacs-compiled-lisp-stats`: counts by the
//! route each took, and the latest loads that read source, each with why.
//! The loader counts a load once its input is in hand (loader.rs
//! `load_input`): an artifact chosen, or its source read.
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// How many misses are kept, the latest.
const KEPT: usize = 32;

/// The route one load took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// A stored artifact whose inputs held.
    Taken,
    /// An artifact a compile process made for this load.
    Compiled,
    /// Source: this process loads no compiled Lisp (not asked, no store,
    /// or a world build).
    Off,
    /// Source, though compiled Lisp is on here.
    Miss(Miss),
}

/// Why a load with compiled Lisp on read source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Miss {
    /// No artifact held; the file is queued for a background compile.
    Deferred,
    /// No artifact held, and this process compiles nothing.
    ReadOnly,
    /// A compile ran here and gave no artifact that opened.
    Failed,
    /// The artifact holds the file as source: its compile refused it or
    /// signalled.
    NotCompiled,
    /// The file is being compiled up this compile's chain.
    Chain,
    /// The file could not be read for its artifact's key.
    Unreadable,
}

impl Miss {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Miss::Deferred => "deferred",
            Miss::ReadOnly => "read-only",
            Miss::Failed => "failed",
            Miss::NotCompiled => "not-compiled",
            Miss::Chain => "chain",
            Miss::Unreadable => "unreadable",
        }
    }
}

/// The counts and the latest misses.
#[derive(Clone, Debug, Default)]
pub(crate) struct Stats {
    pub(crate) taken: u64,
    pub(crate) compiled: u64,
    /// Read source with compiled Lisp on, in a process that compiles.
    pub(crate) source_on_miss: u64,
    /// Read source in a process that compiles nothing: compiled Lisp off,
    /// or a read-only store.
    pub(crate) source_off: u64,
    /// The latest misses, oldest first.
    pub(crate) misses: VecDeque<(PathBuf, Miss)>,
}

impl Stats {
    const fn new() -> Self {
        Stats { taken: 0, compiled: 0, source_on_miss: 0, source_off: 0, misses: VecDeque::new() }
    }

    /// These counts with a load of PATH that took ROUTE, in a process
    /// that COMPILES or not.
    fn note(&mut self, path: &Path, route: Route, compiles: bool) {
        match route {
            Route::Taken => self.taken += 1,
            Route::Compiled => self.compiled += 1,
            Route::Off => self.source_off += 1,
            Route::Miss(miss) => {
                if compiles { self.source_on_miss += 1 } else { self.source_off += 1 }
                if self.misses.len() == KEPT {
                    self.misses.pop_front();
                }
                self.misses.push_back((path.to_owned(), miss));
            }
        }
    }
}

// CELL(Plain)
static STATS: std::sync::Mutex<Stats> = std::sync::Mutex::new(Stats::new());

/// Count a load of PATH that took ROUTE, in a process that COMPILES or not.
pub(crate) fn note(path: &Path, route: Route, compiles: bool) {
    STATS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).note(path, route, compiles);
}

/// This process's counts so far.
pub(crate) fn snapshot() -> Stats {
    STATS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_miss_in_a_process_that_compiles_nothing_counts_as_off_and_is_kept() {
        let mut stats = Stats::new();
        stats.note(Path::new("/a.el"), Route::Taken, false);
        stats.note(Path::new("/b.el"), Route::Miss(Miss::ReadOnly), false);
        stats.note(Path::new("/b2.el"), Route::Miss(Miss::NotCompiled), false);
        stats.note(Path::new("/c.el"), Route::Miss(Miss::Deferred), true);
        stats.note(Path::new("/d.el"), Route::Off, false);
        assert_eq!((stats.taken, stats.compiled, stats.source_on_miss, stats.source_off), (1, 0, 1, 3));
        assert_eq!(stats.misses, [(PathBuf::from("/b.el"), Miss::ReadOnly), (PathBuf::from("/b2.el"), Miss::NotCompiled),
                                  (PathBuf::from("/c.el"), Miss::Deferred)]);
    }

    #[test]
    fn only_the_latest_misses_are_kept() {
        let mut stats = Stats::new();
        for n in 0..KEPT + 3 {
            stats.note(Path::new(&format!("/{n}.el")), Route::Miss(Miss::Failed), true);
        }
        assert_eq!(stats.misses.len(), KEPT);
        assert_eq!(stats.misses.front().map(|m| m.0.clone()), Some(PathBuf::from("/3.el")));
        assert_eq!(stats.source_on_miss, KEPT as u64 + 3);
    }
}
