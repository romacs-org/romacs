//! The candidate filter `re_search` consults: per pattern, whether a DFA is
//! built, warm or given up, and per candidate, whether any match can start
//! there.  Also the `ROMACS_REGEX_DFA*` knobs and the statistics.
//!
//! GNU has no counterpart.

use crate::engine::dfa::class::ClassContext;
use crate::engine::dfa::lazy::{DfaCounters, DfaGaveUp, ExistenceDfa, Exists, MEMORY_CAP};
use crate::engine::dfa::nfa::{DfaIneligible, Nfa};
use crate::engine::matcher::fail::{
    MatchScratch, fail_stack_overflow_free_span, matcher_overflow_pending,
};
use crate::engine::matcher::re_match_candidate_in;
use crate::engine::pattern::{CompiledPattern, MatchRegisters};
use crate::engine::syntax::SyntaxLookup;
use std::cell::{RefCell, RefMut};

/// `ROMACS_REGEX_DFA`: whether `re_search` filters candidates through the
/// existence DFA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DfaMode {
    /// No DFA: the search path is unchanged but for one branch per search
    /// and per candidate.
    Off,
    /// The default: a candidate the DFA rejects is skipped.
    On,
    /// Every candidate runs the matcher too, and a verdict the matcher
    /// contradicts is reported (`tracing::error!`, and the stats' mismatch
    /// counts).  For tests and soaks.
    Verify,
}

impl DfaMode {
    /// The mode a value of `ROMACS_REGEX_DFA` selects: `off`/`0`/`false`/`no`,
    /// `verify`, anything else (or unset) on.
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("verify") => Self::Verify,
            Some("0" | "off" | "false" | "no") => Self::Off,
            _ => Self::On,
        }
    }
}

#[cfg(test)]
thread_local! {
    static DFA_MODE_OVERRIDE: std::cell::Cell<Option<DfaMode>> =
        const { std::cell::Cell::new(None) };
}

/// Run `f` with `ROMACS_REGEX_DFA` forced to `mode` on this thread.
#[cfg(test)]
pub fn with_dfa_mode<R>(mode: DfaMode, f: impl FnOnce() -> R) -> R {
    struct Guard(Option<DfaMode>);
    impl Drop for Guard {
        fn drop(&mut self) {
            DFA_MODE_OVERRIDE.with(|slot| slot.set(self.0));
        }
    }
    let _guard = Guard(DFA_MODE_OVERRIDE.with(|slot| slot.replace(Some(mode))));
    f()
}

#[cfg(test)]
thread_local! {
    // A test-only knob override, not a cache of Lisp state.
    static DFA_COLD_OVERRIDE: std::cell::Cell<Option<bool>> =
        const { std::cell::Cell::new(None) };
}

/// Run `f` with the cold-path knob forced to `on` on this thread.
#[cfg(test)]
pub fn with_cold_path<R>(on: bool, f: impl FnOnce() -> R) -> R {
    struct Guard(Option<bool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            DFA_COLD_OVERRIDE.with(|slot| slot.set(self.0));
        }
    }
    let _guard = Guard(DFA_COLD_OVERRIDE.with(|slot| slot.replace(Some(on))));
    f()
}

/// `ROMACS_REGEX_DFA_COLD` (default on; `off` restores the eager lease): a
/// never-built slot pays no lease
/// or candidate wrapper until enough failed candidates justify a filter.
/// An isolated miss followed by a match does not heat the pattern. Two
/// nonempty failures in one scan are admitted together; a lone nonempty
/// failure is admitted when the whole search fails without overflow or quit. Dense
/// scans still build at the existing threshold, and repeated whole failures
/// build for later searches. Zero-span failures do not count. Read once per
/// process.
#[inline]
pub fn cold_path_enabled() -> bool {
    #[cfg(test)]
    if let Some(on) = DFA_COLD_OVERRIDE.with(|slot| slot.get()) {
        return on;
    }
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = !crate::engine::search::regex_knob_off(
            std::env::var("ROMACS_REGEX_DFA_COLD").ok().as_deref(),
        );
        tracing::debug!(target: "romacs::regex", on, "ROMACS_REGEX_DFA_COLD");
        on
    })
}

/// The `ROMACS_REGEX_DFA` mode, read once per process.
#[inline]
pub fn dfa_mode() -> DfaMode {
    #[cfg(test)]
    if let Some(mode) = DFA_MODE_OVERRIDE.with(|slot| slot.get()) {
        return mode;
    }
    static MODE: std::sync::OnceLock<DfaMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(read_dfa_mode)
}

#[cold]
fn read_dfa_mode() -> DfaMode {
    let mode = DfaMode::parse(std::env::var("ROMACS_REGEX_DFA").ok().as_deref());
    tracing::debug!(target: "romacs::regex", ?mode, "ROMACS_REGEX_DFA");
    if mode != DfaMode::Off
        && crate::engine::search::regex_knob_on(
            std::env::var("ROMACS_REGEX_DFA_STATS").ok().as_deref(),
        )
    {
        STATS_ON.store(true, std::sync::atomic::Ordering::Relaxed);
        register_stats_report();
    }
    if mode == DfaMode::Verify {
        STATS_ON.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    mode
}

#[cfg(test)]
thread_local! {
    // A test-only knob override; the runtime has no additional TLS cache.
    static DFA_FIRST_STEP_OVERRIDE: std::cell::Cell<Option<bool>> =
        const { std::cell::Cell::new(None) };
}

/// Run `f` with the cached first-step knob forced to `on` on this thread.
#[cfg(test)]
pub fn with_first_step<R>(on: bool, f: impl FnOnce() -> R) -> R {
    struct Guard(Option<bool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            DFA_FIRST_STEP_OVERRIDE.with(|slot| slot.set(self.0));
        }
    }
    let _guard = Guard(DFA_FIRST_STEP_OVERRIDE.with(|slot| slot.replace(Some(on))));
    f()
}

/// `ROMACS_REGEX_DFA_FIRST_STEP` (default on; `off` disables): consult an existing dead
/// transition within an eight-byte cached prefix before the general loop.
#[inline]
pub fn first_step_enabled() -> bool {
    #[cfg(test)]
    if let Some(on) = DFA_FIRST_STEP_OVERRIDE.with(|slot| slot.get()) {
        return on;
    }
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = !crate::engine::search::regex_knob_off(
            std::env::var("ROMACS_REGEX_DFA_FIRST_STEP").ok().as_deref(),
        );
        tracing::debug!(target: "romacs::regex", on, "ROMACS_REGEX_DFA_FIRST_STEP");
        on
    })
}

/// Whether the filter keeps its counters: under `ROMACS_REGEX_DFA_STATS=1`,
/// in verify mode (its contradicted verdicts are counted), and in this
/// crate's tests.
/// Otherwise a search pays for none of them.
static STATS_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[inline]
fn stats_on() -> bool {
    cfg!(test) || STATS_ON.load(std::sync::atomic::Ordering::Relaxed)
}

/// Counters of the candidate filter on this thread (the regexp engine runs
/// on the Lisp thread), reported at exit under `ROMACS_REGEX_DFA_STATS=1`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DfaStats {
    /// Searches that used a lease.
    pub searches: u64,
    /// Searches of a pattern that reads syntax under a lookup where
    /// `syntax-table` properties may apply (the DFA splits its scan at
    /// property runs).
    pub positional: u64,
    /// Searches whose lazy `syntax-propertize` frontier lies within the
    /// positions they can read (candidates reaching it go to the matcher).
    pub frontier: u64,
    /// Searches in an adaptive-bypass holiday.
    pub holiday_off: u64,
    pub builds: u64,
    pub ineligible: u64,
    pub gave_up: u64,
    pub yes: u64,
    pub no: u64,
    pub unknown: u64,
    /// Rejections the fail-stack bound refused to act on.
    pub overflow_guarded: u64,
    /// Candidates skipped: matcher entries saved.
    pub skipped: u64,
    pub context_resets: u64,
    pub states: u64,
    pub clears: u64,
    pub slow_transitions: u64,
    pub bytes: u64,
    pub positional_chars: u64,
    pub plain_runs: u64,
    pub frontier_unknown: u64,
    /// Verify mode: a rejection the matcher contradicted (a missed match).
    pub verify_bad_no: u64,
    /// Verify mode: an acceptance the matcher contradicted.
    pub verify_bad_yes: u64,
}

thread_local! {
    static STATS: RefCell<DfaStats> = const {
        RefCell::new(DfaStats {
            searches: 0,
            positional: 0,
            frontier: 0,
            holiday_off: 0,
            builds: 0,
            ineligible: 0,
            gave_up: 0,
            yes: 0,
            no: 0,
            unknown: 0,
            overflow_guarded: 0,
            skipped: 0,
            context_resets: 0,
            states: 0,
            clears: 0,
            slow_transitions: 0,
            bytes: 0,
            positional_chars: 0,
            plain_runs: 0,
            frontier_unknown: 0,
            verify_bad_no: 0,
            verify_bad_yes: 0,
        })
    };
}

#[inline]
fn stat(update: impl FnOnce(&mut DfaStats)) {
    if !stats_on() {
        return;
    }
    STATS.with(|cell| {
        if let Ok(mut stats) = cell.try_borrow_mut() {
            update(&mut stats);
        }
    });
}

/// This thread's filter counters.
#[cfg(test)]
pub fn dfa_stats() -> DfaStats {
    STATS.with(|cell| *cell.borrow())
}

/// Reset this thread's filter counters (tests).
#[cfg(test)]
pub fn reset_dfa_stats() {
    STATS.with(|cell| *cell.borrow_mut() = DfaStats::default());
}

fn register_stats_report() {
    extern "C" fn report() {
        let stats = STATS
            .try_with(|cell| cell.try_borrow().map(|stats| *stats).unwrap_or_default())
            .unwrap_or_default();
        let line = format!("[romacs-regex-dfa] {stats:?}\n");
        let _ = std::io::Write::write_all(&mut std::io::stderr().lock(), line.as_bytes());
    }
    // SAFETY: `report` is an `extern "C" fn()` that only reads this
    // thread's counters and writes one line to stderr.
    unsafe {
        libc::atexit(report);
    }
}

/// Failed matcher entries a pattern's search sees before it builds a DFA.
pub const COLD_THRESHOLD: u32 = 16;

/// Whether a cold slot counts a failure under SYNTAX toward a build: not
/// when KEYLESS and SYNTAX, too, has no class identity to key one by.
#[inline]
fn cold_counts(keyless: bool, syntax: &dyn SyntaxLookup) -> bool {
    !keyless || syntax.class_cache_key().is_some()
}

/// Decisions per adaptive-bypass window.
const BYPASS_WINDOW: u32 = 64;
/// A window in which more than this share (percent) of the verdicts are
/// "yes" sends the pattern on holiday: the DFA saves nothing there.
const BYPASS_YES_PERCENT: u32 = 60;
/// Searches a holiday lasts before the DFA is probed again.
const BYPASS_HOLIDAY: u32 = 256;

/// A pattern's DFA, with its adaptive-bypass window.
pub struct LiveDfa {
    pub dfa: ExistenceDfa,
    /// A rejection that consumed fewer bytes than this cannot hide a
    /// fail-stack overflow ([`fail_stack_overflow_free_span`]).
    pub(crate) overflow_free_span: usize,
    pub(crate) decisions: u32,
    yes: u32,
    holiday: u32,
}

impl LiveDfa {
    fn new(dfa: ExistenceDfa) -> Self {
        Self {
            overflow_free_span: fail_stack_overflow_free_span(dfa.nfa().push_sites),
            dfa,
            decisions: 0,
            yes: 0,
            holiday: 0,
        }
    }
}

/// The state of a pattern's existence DFA.
pub enum DfaSlot {
    /// Not built yet: admitted failed matcher entries seen so far.
    /// KEYLESS: a build found that the pattern reads the lookup and the
    /// search's lookup had no class identity ([`ClassContext::of_search`]).
    /// Failures under such a lookup do not count toward another build,
    /// which would only find the same; a lookup with one counts again.
    Cold {
        failed: u32,
        keyless: bool,
    },
    Live(Box<LiveDfa>),
    /// The reasons are kept for tests and debugging.
    Ineligible(#[allow(dead_code, reason = "read by tests and Debug")] DfaIneligible),
    Disabled(#[allow(dead_code, reason = "read by tests and Debug")] DfaGaveUp),
}

/// `CompiledPattern`'s DFA slot. A clone starts cold: the DFA's caches
/// belong to one pattern object.
///
/// Threading: compiled patterns and their `RefCell` slots are mutator-owned,
/// not `Sync`; another mutator cannot borrow or mutate this slot concurrently.
/// The atomic is an advisory publication hint, not permission to share the slot:
/// Release publishes a fully initialized non-cold slot, and Acquire reads it
/// before trying the existing checked borrow. A false hint runs the matcher.
/// Re-entrant searches still fall back when the slot is borrowed.
pub struct DfaCell {
    slot: RefCell<DfaSlot>,
    initialized: std::sync::atomic::AtomicBool,
}

impl Default for DfaCell {
    fn default() -> Self {
        Self {
            slot: RefCell::new(DfaSlot::Cold {
                failed: 0,
                keyless: false,
            }),
            initialized: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl Clone for DfaCell {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl DfaCell {
    /// The slot, for tests.
    #[cfg(test)]
    pub fn slot(&self) -> std::cell::Ref<'_, DfaSlot> {
        self.slot.borrow()
    }

    /// Whether initialization has published a non-cold slot.
    #[inline]
    pub fn initialized(&self) -> bool {
        self.initialized.load(std::sync::atomic::Ordering::Acquire)
    }

    fn publish_initialized(&self) {
        self.initialized
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

/// Cold admission for one search. Its first nonempty failure stays local:
/// a second failure credits both during the scan; otherwise an isolated
/// failure is credited only when the whole search fails without overflow or quit.
///
/// Threading: this transient state belongs to one search's stack, and is
/// independent of another mutator or a re-entrant search of the same pattern.
#[derive(Default)]
pub enum ColdSearchHeat {
    #[default]
    First,
    Second,
    Repeated,
}

impl ColdSearchHeat {
    #[inline]
    pub fn is_isolated(&self) -> bool {
        matches!(self, Self::Second)
    }

    #[inline]
    pub fn failure_credit(&mut self) -> u32 {
        match self {
            Self::First => {
                *self = Self::Second;
                0
            }
            Self::Second => {
                *self = Self::Repeated;
                2
            }
            Self::Repeated => 1,
        }
    }
}

/// One search's hold on its pattern's DFA slot.
pub struct DfaLease<'p> {
    pub(crate) slot: RefMut<'p, DfaSlot>,
    mode: DfaMode,
    /// The live DFA's counters when this search began using it; the delta is
    /// added to [`DfaStats`] when the lease is dropped.
    counters_before: Option<DfaCounters>,
    pub(crate) skipped: u64,
    overflow_guarded: u64,
}

impl Drop for DfaLease<'_> {
    fn drop(&mut self) {
        if !stats_on() {
            return;
        }
        let delta = match (&*self.slot, self.counters_before) {
            (DfaSlot::Live(live), Some(before)) => {
                let now = live.dfa.counters;
                DfaCounters {
                    yes: now.yes - before.yes,
                    no: now.no - before.no,
                    unknown: now.unknown - before.unknown,
                    states: now.states - before.states,
                    clears: now.clears - before.clears,
                    slow_transitions: now.slow_transitions - before.slow_transitions,
                    bytes: now.bytes - before.bytes,
                    positional_chars: now.positional_chars - before.positional_chars,
                    plain_runs: now.plain_runs - before.plain_runs,
                    frontier_unknown: now.frontier_unknown - before.frontier_unknown,
                }
            }
            _ => DfaCounters::default(),
        };
        let (skipped, overflow_guarded) = (self.skipped, self.overflow_guarded);
        stat(|s| {
            s.yes += delta.yes;
            s.no += delta.no;
            s.unknown += delta.unknown;
            s.states += delta.states;
            s.clears += delta.clears;
            s.slow_transitions += delta.slow_transitions;
            s.bytes += delta.bytes;
            s.positional_chars += delta.positional_chars;
            s.plain_runs += delta.plain_runs;
            s.frontier_unknown += delta.frontier_unknown;
            s.skipped += skipped;
            s.overflow_guarded += overflow_guarded;
        });
    }
}

/// The matcher alone at one candidate (`re_search`'s own call).
#[inline]
#[allow(clippy::too_many_arguments)]
fn classic_candidate(
    scratch: &mut MatchScratch,
    pattern: &CompiledPattern,
    text: &[u8],
    pos: usize,
    stop: usize,
    syntax: &dyn SyntaxLookup,
    point: usize,
    regs: &mut MatchRegisters,
) -> Option<usize> {
    #[cfg(test)]
    crate::engine::matcher::route::MATCHER_ENTRY_COUNT.with(|c| c.set(c.get() + 1));
    re_match_candidate_in(scratch, pattern, text, pos, stop, syntax, point, regs)
}

impl<'p> DfaLease<'p> {
    /// The lease for a search of `pattern` whose candidates all stop by
    /// `max_stop`, or `None` when the DFA cannot serve it:
    ///
    /// * the lookup has no table identity to key the classes by;
    /// * the pattern is ineligible, gave up, or is on holiday;
    /// * a re-entrant search holds the slot.
    ///
    /// `syntax-table` properties and the lazy `syntax-propertize` frontier
    /// are the DFA's own business ([`ExistenceDfa::begin_search`]): it reads
    /// syntax where the matcher does, and leaves a candidate that reaches the
    /// frontier to the matcher, which records the read.
    #[inline(never)]
    pub fn acquire(
        pattern: &'p CompiledPattern,
        syntax: &dyn SyntaxLookup,
        max_stop: usize,
    ) -> Option<Self> {
        let mode = dfa_mode();
        if stats_on() && pattern.uses_syntax {
            if syntax.position_dependent() {
                stat(|s| s.positional += 1);
            }
            if syntax.syntax_read_limit() <= max_stop {
                stat(|s| s.frontier += 1);
            }
        }
        let mut slot = pattern.dfa.slot.try_borrow_mut().ok()?;
        let mut counters_before = None;
        match &mut *slot {
            DfaSlot::Cold { .. } => {}
            DfaSlot::Live(live) => {
                if live.holiday > 0 {
                    live.holiday -= 1;
                    stat(|s| s.holiday_off += 1);
                    return None;
                }
                let context = ClassContext::of_search(pattern, live.dfa.nfa(), syntax)?;
                let resets = live.dfa.classes.resets;
                live.dfa.classes.sync(context);
                if live.dfa.classes.resets != resets {
                    stat(|s| s.context_resets += 1);
                }
                live.dfa.begin_search(pattern, syntax);
                counters_before = Some(live.dfa.counters);
            }
            DfaSlot::Ineligible(_) | DfaSlot::Disabled(_) => return None,
        }
        stat(|s| s.searches += 1);
        Some(Self {
            slot,
            mode,
            counters_before,
            skipped: 0,
            overflow_guarded: 0,
        })
    }

    /// A search admitted one or two classic failures without overflow. Record
    /// them without holding the slot across successful candidates; at the
    /// threshold, build and lease the remaining candidates of this search.
    /// Admission at whole-search failure drops the new lease immediately;
    /// its published slot serves later searches.
    #[cold]
    #[inline(never)]
    pub fn after_cold_failure(
        pattern: &'p CompiledPattern,
        syntax: &dyn SyntaxLookup,
        max_stop: usize,
        failed_candidates: u32,
    ) -> Option<Self> {
        debug_assert!((1..=2).contains(&failed_candidates));
        {
            let mut slot = pattern.dfa.slot.try_borrow_mut().ok()?;
            let DfaSlot::Cold { failed, keyless } = &mut *slot else {
                return None;
            };
            if !cold_counts(*keyless, syntax) {
                return None;
            }
            *failed += failed_candidates;
            if *failed < COLD_THRESHOLD {
                return None;
            }
        }
        let mut lease = Self::acquire(pattern, syntax, max_stop)?;
        lease.build(pattern, syntax);
        if matches!(*lease.slot, DfaSlot::Live(_)) {
            Some(lease)
        } else {
            None
        }
    }

    /// Select the inline path once for a search that actually has a lease.
    /// Verify keeps its anchored probe and always runs the matcher too.
    #[inline]
    pub fn inline_first_step_enabled(&self) -> bool {
        self.mode == DfaMode::On && first_step_enabled()
    }

    /// Skip one bounded cached rejection without entering `candidate`.
    /// The caller selected this path with `inline_first_step_enabled`.
    ///
    /// Threading: this search holds its mutator-owned slot's checked borrow.
    /// Context maps have already been synchronized by `acquire` or `build`;
    /// the cache lookup reads no Lisp state and calls no syntax lookup.
    #[inline]
    pub fn try_inline_first_step_skip(
        &mut self,
        pattern: &CompiledPattern,
        text: &[u8],
        pos: usize,
        stop: usize,
        point: usize,
    ) -> bool {
        debug_assert_eq!(self.mode, DfaMode::On);
        let DfaSlot::Live(live) = &mut *self.slot else {
            return false;
        };
        // Leave state clearing, retirement, and a possible overflow to the
        // outlined path. Its original ordering and accounting stay intact.
        if live.dfa.gave_up.is_some()
            || live.dfa.memory > MEMORY_CAP
            || live.overflow_free_span == 0
        {
            return false;
        }
        let Some(consumed) = live.dfa.cached_prefix_rejection(
            pattern,
            text,
            pos,
            stop,
            point,
            live.overflow_free_span,
        ) else {
            return false;
        };
        live.dfa.counters.no += 1;
        live.dfa.counters.bytes += consumed as u64;
        live.note(Exists::No { consumed });
        self.skipped += 1;
        true
    }

    /// Decide one candidate: the DFA's verdict, then the matcher unless the
    /// verdict is a rejection the fail-stack bound allows acting on.
    /// Returns what `re_match_candidate_in` would, with its side effects.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    pub fn candidate<const FIRST_STEP: bool>(
        &mut self,
        scratch: &mut MatchScratch,
        pattern: &CompiledPattern,
        text: &[u8],
        pos: usize,
        stop: usize,
        syntax: &dyn SyntaxLookup,
        point: usize,
        regs: &mut MatchRegisters,
    ) -> Option<usize> {
        if self.mode == DfaMode::On
            && let DfaSlot::Live(live) = &mut *self.slot
        {
            let verdict = live
                .dfa
                .anchored_exists_inner::<FIRST_STEP>(pattern, text, pos, stop, point, syntax);
            live.note(verdict);
            // A rejection within the fail-stack bound: the matcher would
            // fail here without a side effect.  (A DFA that gave up answers
            // `Unknown`, so a rejection never comes from one.)
            if let Exists::No { consumed } = verdict
                && consumed < live.overflow_free_span
            {
                // The search's work: what makes a long failing scan
                // quadratic is this, summed over its candidates.
                scratch.work = scratch.work.saturating_add(consumed as u64);
                crate::engine::matcher::fail::note_search_work(scratch);
                self.skipped += 1;
                return None;
            }
            return self.settle(
                verdict, scratch, pattern, text, pos, stop, syntax, point, regs,
            );
        }
        // A pattern whose candidates keep matching never leaves the cold
        // state: it costs one branch more than the matcher alone.
        if let DfaSlot::Cold { failed, keyless } = &mut *self.slot {
            let found = classic_candidate(scratch, pattern, text, pos, stop, syntax, point, regs);
            if found.is_none() && !matcher_overflow_pending() && cold_counts(*keyless, syntax) {
                *failed += 1;
                if *failed >= COLD_THRESHOLD {
                    self.build(pattern, syntax);
                }
            }
            return found;
        }
        self.candidate_slow(scratch, pattern, text, pos, stop, syntax, point, regs)
    }

    /// A live DFA's verdict that does not skip the candidate: the matcher
    /// runs (after retiring a DFA that gave up).
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn settle(
        &mut self,
        verdict: Exists,
        scratch: &mut MatchScratch,
        pattern: &CompiledPattern,
        text: &[u8],
        pos: usize,
        stop: usize,
        syntax: &dyn SyntaxLookup,
        point: usize,
        regs: &mut MatchRegisters,
    ) -> Option<usize> {
        if let DfaSlot::Live(live) = &*self.slot
            && let Some(why) = live.dfa.gave_up()
        {
            self.retire(why);
        } else if matches!(verdict, Exists::No { .. }) {
            self.overflow_guarded += 1;
        }
        classic_candidate(scratch, pattern, text, pos, stop, syntax, point, regs)
    }

    #[cold]
    fn retire(&mut self, why: DfaGaveUp) {
        stat(|s| s.gave_up += 1);
        self.counters_before = None;
        *self.slot = DfaSlot::Disabled(why);
        tracing::debug!(target: "romacs::regex", ?why, "existence DFA gave up");
    }

    /// Every candidate of a slot that is not live, and of verify mode.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn candidate_slow(
        &mut self,
        scratch: &mut MatchScratch,
        pattern: &CompiledPattern,
        text: &[u8],
        pos: usize,
        stop: usize,
        syntax: &dyn SyntaxLookup,
        point: usize,
        regs: &mut MatchRegisters,
    ) -> Option<usize> {
        let classic = |scratch: &mut MatchScratch, regs: &mut MatchRegisters| {
            classic_candidate(scratch, pattern, text, pos, stop, syntax, point, regs)
        };
        let live = match &mut *self.slot {
            DfaSlot::Live(live) => live,
            DfaSlot::Cold { failed, keyless } => {
                let found = classic(scratch, regs);
                if found.is_none() && !matcher_overflow_pending() && cold_counts(*keyless, syntax) {
                    *failed += 1;
                    if *failed >= COLD_THRESHOLD {
                        self.build(pattern, syntax);
                    }
                }
                return found;
            }
            DfaSlot::Ineligible(_) | DfaSlot::Disabled(_) => return classic(scratch, regs),
        };
        let verdict = live
            .dfa
            .anchored_exists(pattern, text, pos, stop, point, syntax);
        live.note(verdict);
        if let Some(why) = live.dfa.gave_up() {
            self.retire(why);
            return classic(scratch, regs);
        }
        let overflow_free_span = live.overflow_free_span;
        match (self.mode, verdict) {
            (DfaMode::Verify, verdict) => {
                let found = classic(scratch, regs);
                let overflow = matcher_overflow_pending();
                match verdict {
                    Exists::No { .. } if found.is_some() => {
                        stat(|s| s.verify_bad_no += 1);
                        tracing::error!(
                            target: "romacs::regex",
                            pos,
                            stop,
                            "existence DFA rejected a candidate the matcher matched"
                        );
                        #[cfg(test)]
                        panic!("ROMACS_REGEX_DFA=verify: a rejected candidate matched at {pos}");
                    }
                    Exists::Yes if found.is_none() && !overflow => {
                        stat(|s| s.verify_bad_yes += 1);
                        tracing::error!(
                            target: "romacs::regex",
                            pos,
                            stop,
                            "existence DFA accepted a candidate the matcher failed"
                        );
                        #[cfg(test)]
                        panic!("ROMACS_REGEX_DFA=verify: an accepted candidate failed at {pos}");
                    }
                    _ => {}
                }
                found
            }
            (_, Exists::No { consumed }) => {
                if consumed < overflow_free_span {
                    self.skipped += 1;
                    None
                } else {
                    self.overflow_guarded += 1;
                    classic(scratch, regs)
                }
            }
            _ => classic(scratch, regs),
        }
    }

    #[cold]
    #[inline(never)]
    fn build(&mut self, pattern: &CompiledPattern, syntax: &dyn SyntaxLookup) {
        match Nfa::build(pattern) {
            Ok(nfa) => {
                let mut dfa = ExistenceDfa::new(nfa);
                // A lookup with no table identity cannot key the classes: stay
                // cold, and count failures toward another try from zero only
                // under a lookup that has one.
                let Some(context) = ClassContext::of_search(pattern, dfa.nfa(), syntax) else {
                    *self.slot = DfaSlot::Cold {
                        failed: 0,
                        keyless: true,
                    };
                    return;
                };
                dfa.classes.sync(context);
                dfa.begin_search(pattern, syntax);
                stat(|s| s.builds += 1);
                self.counters_before = Some(DfaCounters::default());
                *self.slot = DfaSlot::Live(Box::new(LiveDfa::new(dfa)));
            }
            Err(why) => {
                stat(|s| s.ineligible += 1);
                tracing::debug!(target: "romacs::regex", ?why, "no existence DFA");
                *self.slot = DfaSlot::Ineligible(why);
            }
        }
        // Publish only after the new state and its complete DFA are written.
        // A lookup without a class identity leaves the slot cold for a retry.
        pattern.dfa.publish_initialized();
    }
}

/// Build `pattern`'s DFA now, skipping the cold phase, so a short
/// differential case exercises the filter.
#[cfg(test)]
pub fn prime(pattern: &CompiledPattern, syntax: &dyn SyntaxLookup) -> Result<(), DfaIneligible> {
    let nfa = Nfa::build(pattern)?;
    let mut dfa = ExistenceDfa::new(nfa);
    if let Some(context) = ClassContext::of_search(pattern, dfa.nfa(), syntax) {
        dfa.classes.sync(context);
    }
    *pattern.dfa.slot.borrow_mut() = DfaSlot::Live(Box::new(LiveDfa::new(dfa)));
    pattern.dfa.publish_initialized();
    Ok(())
}

impl LiveDfa {
    /// Count a verdict toward the adaptive bypass: a window of mostly "yes"
    /// sends the pattern on holiday.
    #[inline]
    fn note(&mut self, verdict: Exists) {
        self.decisions += 1;
        if verdict == Exists::Yes {
            self.yes += 1;
        }
        if self.decisions >= BYPASS_WINDOW {
            if self.yes * 100 > BYPASS_YES_PERCENT * self.decisions {
                self.holiday = BYPASS_HOLIDAY;
            }
            self.decisions = 0;
            self.yes = 0;
        }
    }
}
