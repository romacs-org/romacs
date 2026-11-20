//! What became of this process's loads that could run compiled Lisp
//! (`Vm::compiled_load`), for `romacs-compiled-lisp-stats`: counts by
//! outcome, and the latest loads that read source, each with why.
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// How many misses are kept, the latest.
const KEPT: usize = 32;

/// What one load took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
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
    /// No artifact held, and a background compile will make one.
    Deferred,
    /// No artifact held, and this process compiles nothing.
    ReadOnly,
    /// A compile ran and published no artifact.
    Failed,
    /// The artifact holds the file as source: its compile refused it or
    /// signalled.
    NotCompiled,
    /// The file is being compiled up this compile's chain.
    Chain,
    /// The file could not be read.
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
    /// Read source where a compile was due: every miss but `ReadOnly`.
    pub(crate) source_on_miss: u64,
    /// Read source where this process compiles nothing: `Off` and
    /// `ReadOnly`.
    pub(crate) source_off: u64,
    /// The latest misses, oldest first.
    pub(crate) misses: VecDeque<(PathBuf, Miss)>,
}

impl Stats {
    const fn new() -> Self {
        Stats { taken: 0, compiled: 0, source_on_miss: 0, source_off: 0, misses: VecDeque::new() }
    }

    /// These counts with a load of PATH that took OUTCOME.
    fn note(&mut self, path: &Path, outcome: Outcome) {
        match outcome {
            Outcome::Taken => self.taken += 1,
            Outcome::Compiled => self.compiled += 1,
            Outcome::Off => self.source_off += 1,
            Outcome::Miss(miss) => {
                if miss == Miss::ReadOnly { self.source_off += 1 } else { self.source_on_miss += 1 }
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

/// Count a load of PATH that took OUTCOME.
pub(crate) fn note(path: &Path, outcome: Outcome) {
    STATS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).note(path, outcome);
}

/// This process's counts so far.
pub(crate) fn snapshot() -> Stats {
    STATS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_only_miss_counts_as_off_and_is_kept() {
        let mut stats = Stats::new();
        stats.note(Path::new("/a.el"), Outcome::Taken);
        stats.note(Path::new("/b.el"), Outcome::Miss(Miss::ReadOnly));
        stats.note(Path::new("/c.el"), Outcome::Miss(Miss::Deferred));
        stats.note(Path::new("/d.el"), Outcome::Off);
        assert_eq!((stats.taken, stats.compiled, stats.source_on_miss, stats.source_off), (1, 0, 1, 2));
        assert_eq!(stats.misses, [(PathBuf::from("/b.el"), Miss::ReadOnly), (PathBuf::from("/c.el"), Miss::Deferred)]);
    }

    #[test]
    fn only_the_latest_misses_are_kept() {
        let mut stats = Stats::new();
        for n in 0..KEPT + 3 {
            stats.note(Path::new(&format!("/{n}.el")), Outcome::Miss(Miss::Failed));
        }
        assert_eq!(stats.misses.len(), KEPT);
        assert_eq!(stats.misses.front().map(|m| m.0.clone()), Some(PathBuf::from("/3.el")));
        assert_eq!(stats.source_on_miss, KEPT as u64 + 3);
    }
}
