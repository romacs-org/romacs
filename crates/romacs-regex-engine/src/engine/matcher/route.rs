//! Which engine runs a match: the budgeted backtracker by default, the
//! Pike VM after a catastrophic candidate, and the test seams that pin one.
//!
//! GNU has no counterpart: it has only the backtracker.

// Engine routing (production): the backtracker runs by DEFAULT — it is
// faster on the well-behaved patterns that dominate real workloads
// (font-lock, search).  For a `pike_eligible` pattern the backtracker runs
// under a LINEAR backtracking-step budget; if a single match blows past it
// (catastrophic backtracking, e.g. `a*a*b`), it sets `PIKE_FALLBACK` and the
// caller re-runs the match on the linear, byte-exact Pike VM.  This keeps
// the common-case speed while eliminating catastrophic blow-up.
//
// A test/fuzz-only typed override lets differential checks pin one engine and
// compare it with another (bypassing the budget heuristic). One enum makes
// contradictory "force both engines" states unrepresentable.
thread_local! {
    /// Set by the budgeted backtracker when it gives up so the caller
    /// re-runs the match on the Pike VM.
    static PIKE_FALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    pub static PIKE_FALLBACK_COUNT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
#[cfg(any(test, feature = "test-hooks"))]
pub fn pike_fallback_count() -> u64 {
    PIKE_FALLBACK_COUNT.with(|c| c.get())
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// Matcher entries made by `re_search`'s candidate scans, for tests that
    /// check how many candidates a scan leaves to the matcher.
    pub(crate) static MATCHER_ENTRY_COUNT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
#[cfg(any(test, feature = "test-hooks"))]
pub fn matcher_entry_count() -> u64 {
    MATCHER_ENTRY_COUNT.with(|c| c.get())
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// The deepest fail stack (frames plus undo entries) any backtracker run
    /// reached, and the furthest position at which it pushed, since the last
    /// [`take_fail_stack_probe`]: the measurements behind the bound in
    /// [`fail_stack_may_overflow`].
    static FAIL_STACK_PROBE: std::cell::Cell<FailStackProbe> =
        const { std::cell::Cell::new(FailStackProbe { max_depth: 0, max_push_pos: 0 }) };
}

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FailStackProbe {
    pub max_depth: usize,
    pub max_push_pos: usize,
}

#[cfg(any(test, feature = "test-hooks"))]
pub(crate) fn note_fail_stack_push(depth: usize, d: usize) {
    FAIL_STACK_PROBE.with(|probe| {
        let mut seen = probe.get();
        seen.max_depth = seen.max_depth.max(depth);
        seen.max_push_pos = seen.max_push_pos.max(d);
        probe.set(seen);
    });
}

#[cfg(any(test, feature = "test-hooks"))]
pub fn take_fail_stack_probe() -> FailStackProbe {
    FAIL_STACK_PROBE.with(|probe| probe.replace(FailStackProbe::default()))
}

pub(crate) fn set_pike_fallback() {
    PIKE_FALLBACK.with(|f| f.set(true));
    #[cfg(any(test, feature = "test-hooks"))]
    PIKE_FALLBACK_COUNT.with(|c| c.set(c.get() + 1));
}

/// Read-and-clear the Pike-fallback flag.
pub(crate) fn take_pike_fallback() -> bool {
    PIKE_FALLBACK.with(|f| f.replace(false))
}

#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RegexEngineOverride {
    #[default]
    Production,
    Backtracker,
    PikeVm,
    /// Every forward search of a linear-eligible pattern runs in the linear
    /// matcher from its first candidate (no work budget).
    LinearSearch,
}

#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
thread_local! {
    static REGEX_ENGINE_OVERRIDE: std::cell::Cell<RegexEngineOverride> =
        const { std::cell::Cell::new(RegexEngineOverride::Production) };
}

#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
#[inline]
pub(crate) fn force_backtrack() -> bool {
    REGEX_ENGINE_OVERRIDE.with(|slot| slot.get() == RegexEngineOverride::Backtracker)
}

#[cfg(not(any(test, feature = "test-hooks", feature = "fuzzing")))]
#[inline]
pub(crate) fn force_backtrack() -> bool {
    false
}

#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
#[inline]
pub(crate) fn force_pike() -> bool {
    REGEX_ENGINE_OVERRIDE.with(|slot| slot.get() == RegexEngineOverride::PikeVm)
}

#[cfg(not(any(test, feature = "test-hooks", feature = "fuzzing")))]
#[inline]
pub(crate) fn force_pike() -> bool {
    false
}

#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
#[inline]
pub(crate) fn force_linear_search() -> bool {
    REGEX_ENGINE_OVERRIDE.with(|slot| slot.get() == RegexEngineOverride::LinearSearch)
}

#[cfg(not(any(test, feature = "test-hooks", feature = "fuzzing")))]
#[inline]
pub(crate) fn force_linear_search() -> bool {
    false
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// Forward searches finished by the linear matcher (`linear_search_rest`).
    pub(crate) static LINEAR_SEARCH_COUNT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Forward searches the linear matcher has finished on this thread.
#[cfg(any(test, feature = "test-hooks"))]
pub fn linear_search_count() -> u64 {
    LINEAR_SEARCH_COUNT.with(|c| c.get())
}

/// Run `f` with one regex routing policy, restoring the previous policy even
/// when `f` unwinds.
#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
pub fn with_regex_engine_override<R>(engine: RegexEngineOverride, f: impl FnOnce() -> R) -> R {
    let _guard = crate::tls_scope::TlsScope::new(&REGEX_ENGINE_OVERRIDE, engine);
    f()
}

/// Run `f` with the pure backtracker forced (no budget/Pike). Test only.
#[cfg(any(test, feature = "test-hooks"))]
pub fn with_backtracker_forced<R>(f: impl FnOnce() -> R) -> R {
    with_regex_engine_override(RegexEngineOverride::Backtracker, f)
}

/// Run `f` with the Pike VM forced for eligible patterns. Test only.
#[cfg(any(test, feature = "test-hooks"))]
pub fn with_pike_forced<R>(f: impl FnOnce() -> R) -> R {
    with_regex_engine_override(RegexEngineOverride::PikeVm, f)
}

// SyntaxClass is defined in `syntax_class` (moved from romacs-core's syntax module).
