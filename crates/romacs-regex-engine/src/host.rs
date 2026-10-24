//! Per-thread host hooks the matcher polls.
//!
//! GNU's matcher polls `maybe_quit` at its backtrack sites (regex-emacs.c,
//! `re_match_2_internal`'s `fail:` label) so `C-g` aborts a pathological
//! match.  The quit flag belongs to the host's evaluator, not to any text the
//! engine reads, so it is a hook rather than a trait method: each thread that
//! runs host code installs its probe once (romacs-core does so whenever it
//! activates an evaluator's thread locals), and a thread with no probe never
//! quits.
//!
//! The probe is a plain `fn` reading the host's own thread-local flag; it must
//! not run Lisp or touch any heap, since it is called mid-match.

use std::cell::Cell;

thread_local! {
    static QUIT_PROBE: Cell<Option<fn() -> bool>> = const { Cell::new(None) };
}

/// Install `probe` as this thread's quit poll, returning the one it replaces.
pub fn set_quit_probe(probe: Option<fn() -> bool>) -> Option<fn() -> bool> {
    QUIT_PROBE.with(|slot| slot.replace(probe))
}

/// Whether the host has a quit pending on this thread.  `false` when no probe
/// is installed.
#[inline]
pub fn quit_pending() -> bool {
    QUIT_PROBE.with(Cell::get).is_some_and(|probe| probe())
}
