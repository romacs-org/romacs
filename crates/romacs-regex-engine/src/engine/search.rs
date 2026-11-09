//! The search loop: try each candidate start, or hand the rest of a
//! forward search to the linear matcher.
//!
//! GNU: `re_search_2` in regex-emacs.c.

use crate::engine::dfa;
use crate::engine::linear;
use crate::engine::matcher::fail::{
    MATCHER_OVERFLOWED, MatchScratch, clear_matcher_overflow, fail_stack_overflow_free_span,
    fail_stack_push_sites, matcher_interrupt, matcher_overflow_pending,
};
#[cfg(any(test, feature = "test-hooks"))]
use crate::engine::matcher::route::{LINEAR_SEARCH_COUNT, MATCHER_ENTRY_COUNT};
use crate::engine::matcher::route::{force_backtrack, force_linear_search};
use crate::engine::matcher::{MatchScratchLease, re_match_candidate_in};
use crate::engine::opcode::{RegexOp, extract_number};
use crate::engine::pattern::{CompiledPattern, MatchRegisters, PREFILTER_MIN_BUILD_SPAN};
use crate::engine::scan::{FoldedScan, SparseAsciiFastmap};
use crate::engine::syntax::SyntaxLookup;
use crate::engine::translate::{
    AsciiPreimage, ascii_translated_leading_code, multibyte_translated_leading_code,
};
use regex_automata::Span;

// Test/fuzz-only escape hatch: force `re_search` down the no-fastmap path so
// equivalence checks can prove search optimizations never change match results.
#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SearchOptimizationOverride {
    #[default]
    Production,
    Disabled,
}

#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
thread_local! {
    static SEARCH_OPTIMIZATION_OVERRIDE: std::cell::Cell<SearchOptimizationOverride> =
        const { std::cell::Cell::new(SearchOptimizationOverride::Production) };
}

#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
pub(crate) fn fastmap_force_disabled() -> bool {
    SEARCH_OPTIMIZATION_OVERRIDE.with(|slot| slot.get() == SearchOptimizationOverride::Disabled)
}

/// Run `f` with the fastmap AND prefilter search-skips disabled, so
/// `re_search` scans every position through the pure backtracker/Pike matcher.
/// This is the ORACLE for the prefilter differential fuzzer: comparing it
/// against the normal (prefilter-enabled) search proves the prefilter never
/// changes match results. The previous policy is restored even if `f` unwinds.
#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
pub fn with_fastmap_disabled<R>(f: impl FnOnce() -> R) -> R {
    let _guard = crate::tls_scope::TlsScope::new(
        &SEARCH_OPTIMIZATION_OVERRIDE,
        SearchOptimizationOverride::Disabled,
    );
    f()
}

#[cfg(not(any(test, feature = "test-hooks", feature = "fuzzing")))]
#[inline(always)]
pub(crate) fn fastmap_force_disabled() -> bool {
    false
}

/// Build every scanner a search derives lazily from `pattern`, as the first
/// search long enough to use them would.  The differential checker calls this
/// so a short generated text still exercises the optimized scans.
#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
pub fn build_search_optimizations(pattern: &CompiledPattern) {
    let _ = pattern.literal_prefilter();
    if let Some(table) = pattern.translate.as_ref() {
        let _ = pattern.folded_scan(table, true);
    }
}

// ---------------------------------------------------------------------------
// Search knobs (same-binary A/B)
// ---------------------------------------------------------------------------

/// Whether a regexp knob's value turns it on: `1`, `on`, `true` or `yes`.
/// An opt-out knob is off only when explicitly set to `0`/`off`/`false`/`no`.
pub(crate) fn regex_knob_off(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0" | "off" | "false" | "no")
    )
}

pub(crate) fn regex_knob_on(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "on" | "true" | "yes")
    )
}

#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
thread_local! {
    static ANCHOR_ALT_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Run `f` with `ROMACS_REGEX_ANCHOR_ALT` forced to `on` on this thread.  It is
/// read when a pattern compiles, so compile inside `f`.
#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
pub fn with_anchor_alt<R>(on: bool, f: impl FnOnce() -> R) -> R {
    let _guard = crate::tls_scope::TlsScope::new(&ANCHOR_ALT_OVERRIDE, Some(on));
    f()
}

/// `ROMACS_REGEX_ANCHOR_ALT` (default on; `off` disables): a pattern whose every
/// alternative begins with `^` (or `\``) searches line starts only, as a
/// pattern whose FIRST opcode is `^` already does (P3.3 Stage 0).  Read once
/// per process, when the first pattern compiles.  On by default since the
/// board A/B (org-editing -1.4% instructions, org-editing-heavy -0.45%, the
/// census pattern pat-026 1.81M -> 0.20M instructions per search).
pub fn anchor_alt_enabled() -> bool {
    #[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
    if let Some(on) = ANCHOR_ALT_OVERRIDE.with(|slot| slot.get()) {
        return on;
    }
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = !regex_knob_off(std::env::var("ROMACS_REGEX_ANCHOR_ALT").ok().as_deref());
        tracing::debug!(target: "romacs::regex", on, "ROMACS_REGEX_ANCHOR_ALT");
        on
    })
}

/// The zero-width anchor that every match of a pattern begins with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StartAnchor {
    /// Some path from the start consumes a character (or succeeds) without
    /// passing `^` or `\``, or the pattern was not analyzed.
    #[default]
    None,
    /// Every path from the start meets `^` or `\`` before anything else, and
    /// one meets `^`: a match can start only at a line start.
    Line,
    /// Every path from the start meets `\`` first: only position 0.
    Buffer,
}

/// Which anchor every path from the start of `bytecode` meets before any
/// other opcode, following the opcodes that neither consume nor test
/// (`no_op`, group markers, jumps and both edges of every failure-point
/// opcode).  GNU's `re_search_2` looks only at the first opcode
/// (`anchored_start`); a candidate this analysis rules out is one where every
/// path dies at the anchor, having pushed a few failure points and consumed
/// nothing, so skipping it changes no result.
pub fn start_anchor(bytecode: &[u8]) -> StartAnchor {
    let mut visited = vec![false; bytecode.len() + 1];
    let mut stack = vec![0usize];
    let mut line = false;
    let mut buf = false;
    while let Some(pc) = stack.pop() {
        if pc >= bytecode.len() {
            // Fell off the end: an empty match with no anchor.
            return StartAnchor::None;
        }
        if std::mem::replace(&mut visited[pc], true) {
            continue;
        }
        let Some(op) = RegexOp::from_byte(bytecode[pc]) else {
            return StartAnchor::None;
        };
        match op {
            RegexOp::BegLine => line = true,
            RegexOp::BegBuf => buf = true,
            RegexOp::NoOp => stack.push(pc + 1),
            RegexOp::StartMemory | RegexOp::StopMemory => stack.push(pc + 2),
            RegexOp::Jump => {
                let target = pc as i64 + 3 + extract_number(bytecode, pc + 1) as i64;
                let Ok(target) = usize::try_from(target) else {
                    return StartAnchor::None;
                };
                stack.push(target);
            }
            RegexOp::OnFailureJump
            | RegexOp::OnFailureKeepStringJump
            | RegexOp::OnFailureJumpLoop
            | RegexOp::OnFailureJumpNastyloop
            | RegexOp::OnFailureJumpSmart => {
                let target = pc as i64 + 3 + extract_number(bytecode, pc + 1) as i64;
                let Ok(target) = usize::try_from(target) else {
                    return StartAnchor::None;
                };
                stack.push(pc + 3);
                stack.push(target);
            }
            _ => return StartAnchor::None,
        }
    }
    match (line, buf) {
        (true, _) => StartAnchor::Line,
        (false, true) => StartAnchor::Buffer,
        (false, false) => StartAnchor::None,
    }
}

/// Where a forward search's candidates end (and every candidate stops).
#[inline]
fn forward_end(start: usize, text_len: usize, range: isize) -> usize {
    start.saturating_add(range.max(0) as usize).min(text_len)
}

/// Work per byte of the searched span that a forward search's candidates
/// may do before the rest of the search moves to the linear matcher.
const SEARCH_WORK_PER_BYTE: u64 = 8;
/// Work any search may do regardless of its span.
const SEARCH_WORK_BASE: u64 = 8192;

/// The candidate work a forward search over `span` bytes may do before it
/// finishes in the linear matcher.  Each candidate's own cost is linear (a
/// DFA rejection reads at most the rest of the span; the budgeted
/// backtracker pops at most 8,000 failure points before the linear matcher
/// takes the candidate), so what makes a failing scan quadratic is many
/// candidates each reading far ahead -- `a*a*a*b` over a run of `a`s.  An
/// ordinary scan stays far below the budget: most candidates are rejected by
/// the fastmap, the prefilter or the DFA after a few bytes.
#[inline]
fn search_work_budget(span: usize) -> u64 {
    SEARCH_WORK_BASE.saturating_add(SEARCH_WORK_PER_BYTE.saturating_mul(span as u64))
}

/// Finish a forward search in the linear matcher: the candidates
/// `from..=end`, each stopping by `end`, decided in one pass.
///
/// A candidate the linear run hands back ([`linear::SearchOutcome::Defer`]:
/// its paths ran far enough that GNU's fail stack might overflow) is decided
/// by the per-candidate path, overflow included, and the search resumes after
/// it.  Every other candidate is one whose backtracker would stay inside the
/// fail-stack bound, where the linear matcher's answer is the backtracker's.
#[cold]
#[inline(never)]
fn linear_search_rest(
    scratch: &mut MatchScratch,
    pattern: &CompiledPattern,
    text: &[u8],
    mut from: usize,
    end: usize,
    syntax: &dyn SyntaxLookup,
    point: usize,
) -> Option<(usize, MatchRegisters)> {
    #[cfg(any(test, feature = "test-hooks"))]
    LINEAR_SEARCH_COUNT.with(|c| c.set(c.get() + 1));
    scratch.work_budget = u64::MAX;
    let free_span = fail_stack_overflow_free_span(fail_stack_push_sites(&pattern.buffer));
    // The fastmap may skip a position only where GNU's own scan would: an
    // accurate map, a pattern that cannot match empty, no translation.
    let skip = (pattern.fastmap_accurate && !pattern.can_be_null && pattern.translate.is_none())
        .then_some(&pattern.fastmap);
    loop {
        if from > end {
            return None;
        }
        // Once a parity run was abandoned the fail-stack model is off.
        let span = if scratch.parity_abandoned {
            usize::MAX
        } else {
            free_span
        };
        match linear::run_search(pattern, text, from, end, end, syntax, point, skip, span) {
            linear::SearchOutcome::Found(start, regs) => return Some((start, regs)),
            linear::SearchOutcome::NotFound | linear::SearchOutcome::Quit => return None,
            linear::SearchOutcome::Defer(candidate) => {
                let mut regs = MatchRegisters::default();
                if re_match_candidate_in(
                    scratch, pattern, text, candidate, end, syntax, point, &mut regs,
                )
                .is_some()
                {
                    return Some((candidate, regs));
                }
                if matcher_overflow_pending() {
                    return None;
                }
                from = candidate + 1;
            }
        }
    }
}

/// Search for a match of the compiled pattern in text.
///
/// Equivalent to GNU's `re_search_2()` operating on a single
/// contiguous string. GNU also exposes the two-string variant
/// `re_match_2(pattern, string1, size1, string2, size2, ...)` which
/// walks the buffer text across the gap boundary
/// (`BEG..GPT` and `GPT..ZV`) without copying — for a 100MB buffer
/// that saves a 100MB allocation per search. Audit finding #17 in
/// `drafts/regex-search-audit.md` flags this as missing in romacs.
///
/// We currently allocate the full buffer text via
/// `Buffer::buffer_substring_range(Buffer::accessible_emacs_byte_range())` at
/// the call site in `regex/mod.rs::re_search_forward_with_posix` and friends, which is
/// correctness-equivalent to GNU's `re_match_2_internal` running
/// over a single string but is O(buffer-size) per search instead of
/// O(match-length). Porting the gap-aware path is a separate
/// optimization (audit Phase D Task 4.1, ~1 day).
///
/// # Arguments
/// * `pattern` - Compiled pattern
/// * `text` - Input text
/// * `start` - Starting search position
/// * `range` - How far to search (positive = forward, negative = backward)
/// * `syntax` - Syntax table lookup
/// * `point` - Buffer point (for `\=`)
///
/// # Returns
/// * `Some((match_start, registers))` if found
/// * `None` if no match
// `try_candidate!`'s cold-path arm reassigns `dfa_first_step` and
// `dfa_cold_pending` for the NEXT candidate; the `\``-anchored arm expands it
// once and leaves, so rustc sees those stores as dead there.  The same
// warning fired while this lived in romacs-core (which builds with warnings).
#[allow(unused_assignments)]
pub fn re_search(
    pattern: &CompiledPattern,
    text: &[u8],
    start: usize,
    range: isize,
    syntax: &dyn SyntaxLookup,
    point: usize,
) -> Option<(usize, MatchRegisters)> {
    let text_len = text.len();
    // GNU regex-emacs.c:3483: `if (fastmap && startpos < total_size &&
    // !bufp->can_be_null)` — the fastmap skip is gated ONLY on having an
    // accurate map and the pattern not matching the null string.  Syntax-
    // dependent constructs do not disable it: leading `\w`/`\sC` set
    // `can_be_null` via `analyze_first`, zero-width `\b \< \_>` etc.
    // contribute the following atom's characters, and `[[:word:]]` /
    // `[[:space:]]` bake the active table into the fastmap with the
    // compile cache keyed by that table (`used_syntax`).
    let use_fastmap = pattern.fastmap_accurate && !pattern.can_be_null && !fastmap_force_disabled();
    let translate = pattern.translate.as_ref();

    // `^`-anchored pattern: a forward candidate can only match at position 0
    // or right after a newline, so reject others with one byte compare
    // instead of a full matcher entry (scratch borrow, register reset,
    // dispatch) per fastmap hit. GNU enters the matcher and fails at
    // `begline`; the candidate set is identical either way.
    let first_op = {
        let mut pc = 0;
        while pc < pattern.buffer.len() && pattern.buffer[pc] == RegexOp::NoOp as u8 {
            pc += 1;
        }
        pattern.buffer.get(pc).copied()
    };
    // `ROMACS_REGEX_ANCHOR_ALT`: the whole-pattern analysis, when compiled in
    // (off under the search-optimization oracle).
    let start_anchor = if fastmap_force_disabled() {
        StartAnchor::None
    } else {
        pattern.start_anchor
    };
    let bol_anchored =
        first_op == Some(RegexOp::BegLine as u8) || start_anchor == StartAnchor::Line;
    // `\``-anchored pattern (GNU `begbuf`, which succeeds only where
    // `AT_STRINGS_BEG`, `d == 0` here): position 0 is the one candidate.
    // Every other entry would pay the matcher's setup only to fail on the
    // first opcode -- 61 such entries for each 62-byte file name that
    // `find-file-name-handler` tests against TRAMP's archive handler
    // (`\`\(.+\.\(?:7z\|...`), which made a match 1.5 times GNU's.
    let buf_anchored =
        first_op == Some(RegexOp::BegBuf as u8) || start_anchor == StartAnchor::Buffer;

    // A fresh search starts with a clean overflow flag; a candidate match
    // that hits the fail-stack limit sets it, aborting the whole scan
    // (GNU re_search_2 propagates re_match_2_internal's -2 immediately).
    clear_matcher_overflow();
    // One scratch for the whole search (see re_match_candidate_in): lease
    // the per-thread one out of its cell for the duration (its Vec capacity
    // moves with it, nothing allocates) and hand it back on every exit; a
    // re-entrant search finds an empty cell and works on a fresh one.
    let mut lease = MatchScratchLease::take();
    let scratch: &mut MatchScratch = lease.get();
    // The quadratic-scan guard: a forward search of a linear-eligible pattern
    // whose candidates do more than `search_work_budget` work finishes in the
    // linear matcher (`u64::MAX`: never).
    scratch.work = 0;
    scratch.parity_abandoned = false;
    let linear_ok = range >= 0
        && pattern.linear.eligible()
        && !force_backtrack()
        && !fastmap_force_disabled()
        && !buf_anchored;
    scratch.work_budget = if linear_ok {
        search_work_budget(forward_end(start, text_len, range) - start.min(text_len))
    } else {
        u64::MAX
    };
    if linear_ok && force_linear_search() {
        let end = forward_end(start, text_len, range);
        return linear_search_rest(scratch, pattern, text, start, end, syntax, point);
    }
    // The registers of the one match this search returns: every candidate
    // fills them only on success, so a failed candidate moves nothing.
    let mut regs = MatchRegisters::default();
    // `ROMACS_REGEX_DFA`: the existence DFA filters the candidates, when on
    // and usable for this search (see `dfa::DfaLease::acquire`).  Every
    // candidate of a search stops at most at `max_stop`.
    let dfa_enabled = dfa::dfa_mode() != dfa::DfaMode::Off && !fastmap_force_disabled();
    let mut dfa_cold_pending =
        dfa_enabled && dfa::cold_path_enabled() && !pattern.dfa.initialized();
    let dfa_max_stop = if range >= 0 {
        start.saturating_add(range as usize).min(text_len)
    } else {
        start
    };
    let mut dfa_lease = if !dfa_enabled || dfa_cold_pending {
        None
    } else {
        dfa::DfaLease::acquire(pattern, syntax, dfa_max_stop)
    };
    // Never-built successful searches have no lease under the cold path,
    // so they never read the first-step knob. A newly built lease renews
    // this flag below; there is no per-candidate knob lookup in inline mode.
    let mut dfa_first_step = dfa_lease
        .as_ref()
        .is_some_and(|lease| lease.inline_first_step_enabled());
    let mut dfa_cold_heat = dfa::ColdSearchHeat::default();
    macro_rules! try_candidate {
        ($pos:expr, $stop:expr) => {{
            let found = match dfa_lease.as_mut() {
                None => {
                    #[cfg(any(test, feature = "test-hooks"))]
                    MATCHER_ENTRY_COUNT.with(|c| c.set(c.get() + 1));
                    re_match_candidate_in(
                        scratch, pattern, text, $pos, $stop, syntax, point, &mut regs,
                    )
                }
                Some(lease) => {
                    if dfa_first_step {
                        if lease.try_inline_first_step_skip(pattern, text, $pos, $stop, point) {
                            None
                        } else {
                            // The inline check already tried the cached
                            // rejection: do not repeat it inside candidate.
                            lease.candidate::<false>(
                                scratch, pattern, text, $pos, $stop, syntax, point, &mut regs,
                            )
                        }
                    } else {
                        lease.candidate::<true>(
                            scratch, pattern, text, $pos, $stop, syntax, point, &mut regs,
                        )
                    }
                }
            };
            match found {
                Some(end) => Some((end, std::mem::take(&mut regs))),
                None => {
                    let interrupt = matcher_interrupt();
                    if interrupt != 0 {
                        if interrupt & MATCHER_OVERFLOWED != 0 {
                            return None;
                        }
                        // The candidates have cost more than the text
                        // justifies: the rest of the forward search runs in
                        // one linear pass.
                        clear_matcher_overflow();
                        return linear_search_rest(
                            scratch,
                            pattern,
                            text,
                            $pos + 1,
                            $stop,
                            syntax,
                            point,
                        );
                    }
                    // A classic zero-span attempt still decides assertions
                    // and overflow, but cannot seed useful DFA scan work.
                    // In particular, the sparse scan's EOF failure should
                    // not heat a pattern whose real candidates all match.
                    if dfa_cold_pending && $pos < $stop && $pos < text_len {
                        // One isolated miss cannot justify filtering all the
                        // true candidates of a mostly successful pattern.
                        // Keep it local until a second failure in this scan;
                        // successful candidates add no slot bookkeeping.
                        let credit = dfa_cold_heat.failure_credit();
                        if credit != 0 {
                            dfa_lease = dfa::DfaLease::after_cold_failure(
                                pattern,
                                syntax,
                                dfa_max_stop,
                                credit,
                            );
                            dfa_first_step = dfa_lease
                                .as_ref()
                                .is_some_and(|lease| lease.inline_first_step_enabled());
                            // Ineligible/disabled slots stop counting too. A
                            // classless lookup stays cold and retries later.
                            dfa_cold_pending = !pattern.dfa.initialized();
                        }
                    }
                    None
                }
            }
        }};
    }

    let result = 'scan: {
        if range >= 0 {
            // Forward search
            let end = (start + range as usize).min(text_len);
            let mut pos = start;
            if buf_anchored {
                if start != 0 {
                    break 'scan None;
                }
                break 'scan try_candidate!(0, end).map(|result| (0, result.1));
            }
            if bol_anchored {
                // `^`-anchored: candidates are position 0 and each byte after a
                // newline — drive the scan with memchr('\n') (SIMD) instead of
                // entering the matcher at every fastmap hit only to fail at
                // `begline`. The fastmap still pre-rejects a line whose first
                // byte can't start a match; BOL positions are always char
                // boundaries, so no continuation-byte check is needed.
                loop {
                    if pos > end {
                        break 'scan None;
                    }
                    if pos == 0 || text[pos - 1] == b'\n' {
                        let first_ok = !use_fastmap
                            || pos >= text_len
                            || match translate {
                                Some(table) => {
                                    pattern.fastmap[table.translate_byte(text[pos]) as usize]
                                }
                                None => pattern.fastmap[text[pos] as usize],
                            };
                        if first_ok && let Some(result) = try_candidate!(pos, end) {
                            break 'scan Some((pos, result.1));
                        }
                    }
                    let search_from = pos.min(text_len);
                    // Only newlines before `end` can yield a candidate at or
                    // before the bound. Include `end - 1`: a zero-width match
                    // may start at `end`. A short bounded failure must not scan
                    // the remaining buffer when it contains no nearby newline.
                    match memchr::memchr(b'\n', &text[search_from..end]) {
                        Some(idx) => pos = search_from + idx + 1,
                        None => break 'scan None,
                    }
                }
            }
            if use_fastmap {
                // Whether this search is long enough to build a lazily derived
                // scanner it finds missing (it always uses one already built).
                let long_span = text_len.saturating_sub(start) >= PREFILTER_MIN_BUILD_SPAN;
                // The folded scans and the folded prefilter are exact only while
                // no non-ASCII character translates into ASCII; a case-canon
                // char-table can stop meeting that after they were built.
                let folds_exactly = match translate {
                    None => true,
                    Some(table) => {
                        table.ascii_preimage_for_span(end.saturating_sub(start))
                            == AsciiPreimage::AsciiOnly
                    }
                };
                let prefilter = if !folds_exactly {
                    None
                } else if long_span {
                    pattern.literal_prefilter()
                } else {
                    pattern.prefilter.get().and_then(Option::as_ref)
                };
                if let Some(pref) = prefilter {
                    // SIMD multi-literal skip: jump straight to the next position
                    // whose text provably contains a required match literal, far
                    // more aggressively than the single-byte fastmap.  The
                    // backtracker still verifies every candidate, so correctness
                    // rests only on the literal set being SOUND — every match
                    // contains one of the needles at `off` bytes from its start
                    // (see `build_literal_prefilter`).  A case-folded pattern's
                    // needles are every ASCII spelling of its literals
                    // (`fold_prefix_literals`), so the scan itself needs no
                    // translation.
                    let off = pref.offset;
                    // The literal for a match starting at `m` sits at
                    // `text[m + off ..]`; the earliest candidate is `start`, whose
                    // literal begins at `start + off`.
                    let mut next_lit = start.saturating_add(off).min(text_len);
                    // GNU's fastmap skip loop is bounded by the remaining RANGE
                    // (`while (range > lim && !fastmap[*d])`, regex-emacs.c), so a
                    // bounded search never looks past its bound. Scanning to
                    // `text_len` instead made every bounded FAILING search cost the
                    // whole buffer -- 34.6ms on 800KB against GNU's flat 0.7ms --
                    // which is the shape font-lock runs constantly.
                    //
                    // `end` is the match STOP handed to `re_match_candidate_in`,
                    // so an acceptable match lies entirely within `[cand, end]`.
                    // The literal is a required prefix of that match, so it lies
                    // there too, and a scan span reaching one byte past `end`
                    // cannot hide one. Nothing about the needle lengths enters:
                    // it is the MATCH that is bounded, not the literal.
                    let scan_end = end.saturating_add(1).min(text_len);
                    while next_lit <= scan_end {
                        let Some(span) = pref.pf.find(
                            text,
                            Span {
                                start: next_lit,
                                end: scan_end,
                            },
                        ) else {
                            break;
                        };
                        let lit_at = span.start;
                        let cand = lit_at.saturating_sub(off);
                        if cand > end {
                            break;
                        }
                        // Advance past this literal occurrence for the next probe,
                        // regardless of whether this candidate matches — keeps the
                        // scan strictly monotonic (no infinite loop).
                        next_lit = lit_at + 1;
                        if cand < start {
                            // Literal too early to back up to an in-range start.
                            continue;
                        }
                        // A match cannot start inside a multibyte character; the
                        // needle bytes are exact, so a continuation-byte candidate
                        // is never a real match — skip it.
                        if pattern.target_multibyte
                            && cand < text_len
                            && (text[cand] & 0xC0) == 0x80
                        {
                            continue;
                        }
                        if let Some(result) = try_candidate!(cand, end) {
                            break 'scan Some((cand, result.1));
                        }
                    }
                } else {
                    // A case-folded search first folds `TRANSLATE` into the
                    // fastmap (`build_folded_scan`), which leaves one table load
                    // per byte, or memchr when 1-3 ASCII bytes can start a
                    // match.  No per-byte `CaseTranslation::translate` remains.
                    let folded = match translate {
                        Some(table) if folds_exactly => pattern.folded_scan(table, long_span),
                        _ => None,
                    };
                    let sparse = match (translate, folded) {
                        (None, _) => pattern.sparse_ascii_fastmap(),
                        (Some(_), Some(FoldedScan::Sparse(bytes))) => Some(*bytes),
                        (Some(_), _) => None,
                    };
                    if let Some(bytes) = sparse {
                        // The candidate first-byte set is tiny and pure ASCII
                        // (e.g. `{'('}` for the font-lock defun matchers, or
                        // `{'D', 'd'}` for a case-folded `defun`):
                        // let memchr's SIMD scan find candidates instead of
                        // testing the fastmap byte by byte.  ASCII hits are
                        // never UTF-8 continuation bytes, so the char-boundary
                        // skip is vacuous here.  GNU uses the plain
                        // `while (range > lim && !fastmap[*d]) d++` loop; the
                        // candidate set and attempt positions are identical.
                        let hi = if end < text_len { end + 1 } else { text_len };
                        while pos <= end {
                            if pos < text_len {
                                let found = match bytes {
                                    SparseAsciiFastmap::One(b0) => {
                                        memchr::memchr(b0, &text[pos..hi])
                                    }
                                    SparseAsciiFastmap::Two(b0, b1) => {
                                        memchr::memchr2(b0, b1, &text[pos..hi])
                                    }
                                    SparseAsciiFastmap::Three(b0, b1, b2) => {
                                        memchr::memchr3(b0, b1, b2, &text[pos..hi])
                                    }
                                };
                                match found {
                                    Some(idx) => pos += idx,
                                    None => {
                                        // No candidate byte before `hi`; the only
                                        // remaining attempt position is text_len
                                        // itself (when the range allows it).
                                        pos = text_len;
                                        if pos > end {
                                            break;
                                        }
                                    }
                                }
                            }
                            if let Some(result) = try_candidate!(pos, end) {
                                break 'scan Some((pos, result.1));
                            }
                            pos += 1;
                        }
                    } else if let Some(FoldedScan::Table(accept)) = folded {
                        let accept: &[bool; 256] = accept;
                        // GNU `while (range > lim && !fastmap[*d]) d++`, with
                        // TRANSLATE and the fastmap folded into one table.  The
                        // table admits no byte >= 0x80 of multibyte text, so no
                        // continuation byte is ever a candidate.
                        let hi = if end < text_len { end + 1 } else { text_len };
                        while pos <= end {
                            if pos < hi {
                                match text[pos..hi].iter().position(|&b| accept[b as usize]) {
                                    Some(idx) => pos += idx,
                                    // Only the end of the text remains, when in range.
                                    None => pos = hi,
                                }
                            }
                            if pos > end {
                                break;
                            }
                            if let Some(result) = try_candidate!(pos, end) {
                                break 'scan Some((pos, result.1));
                            }
                            pos += 1;
                        }
                    } else if let Some(table) = translate
                        && pattern.target_multibyte
                    {
                        // GNU `re_search_2` (regex-emacs.c) on multibyte text:
                        // translate the WHOLE character and test the leading code of
                        // the result, stepping a character at a time.
                        while pos <= end {
                            if pos > text_len {
                                break;
                            }
                            if pos < text_len {
                                let byte = text[pos];
                                if byte < 0x80 {
                                    if !pattern.fastmap_translated
                                        [ascii_translated_leading_code(table, byte)]
                                    {
                                        pos += 1;
                                        continue;
                                    }
                                } else if (byte & 0xC0) == 0x80 {
                                    pos += 1;
                                    continue;
                                } else {
                                    let (lead, len) =
                                        multibyte_translated_leading_code(table, text, pos);
                                    if !pattern.fastmap_translated[lead] {
                                        pos += len;
                                        continue;
                                    }
                                }
                            }
                            if let Some(result) = try_candidate!(pos, end) {
                                break 'scan Some((pos, result.1));
                            }
                            pos += 1;
                        }
                    } else if let Some(table) = translate {
                        while pos <= end {
                            if pos > text_len {
                                break;
                            }
                            // GNU disables fastmap skipping for nullable patterns so zero-width
                            // matches like `\\(?:...\\)\\=` are still considered at every point.
                            //
                            // GNU regex-emacs.c:3568 applies TRANSLATE to the input
                            // byte before indexing the fastmap. Under case-fold that
                            // is what lets a fastmap built for a bitmap of lowercase
                            // characters still catch uppercase input (audit #9).
                            if pos < text_len {
                                let idx = table.translate_byte(text[pos]) as usize;
                                if !pattern.fastmap[idx] {
                                    pos += 1;
                                    continue;
                                }
                            }
                            if let Some(result) = try_candidate!(pos, end) {
                                break 'scan Some((pos, result.1));
                            }
                            pos += 1;
                        }
                    } else {
                        while pos <= end {
                            if pos > text_len {
                                break;
                            }
                            if pattern.target_multibyte
                                && pos < text_len
                                && (text[pos] & 0xC0) == 0x80
                            {
                                pos += 1;
                                continue;
                            }
                            if pos < text_len && !pattern.fastmap[text[pos] as usize] {
                                pos += 1;
                                continue;
                            }
                            if let Some(result) = try_candidate!(pos, end) {
                                break 'scan Some((pos, result.1));
                            }
                            pos += 1;
                        }
                    }
                }
            } else {
                while pos <= end {
                    if pos > text_len {
                        break;
                    }
                    if pattern.target_multibyte && pos < text_len && (text[pos] & 0xC0) == 0x80 {
                        pos += 1;
                        continue;
                    }
                    if let Some(result) = try_candidate!(pos, end) {
                        break 'scan Some((pos, result.1));
                    }
                    pos += 1;
                }
            }
        } else {
            // Backward search
            let end = start.saturating_sub((-range) as usize);
            if use_fastmap {
                // A case-folded search scans with its folded table, as forward,
                // built by the first search long enough to repay it.
                let folded = match translate {
                    Some(table)
                        if start <= text_len
                            && table.ascii_preimage_for_span(start - end)
                                == AsciiPreimage::AsciiOnly =>
                    {
                        pattern.folded_scan(table, start - end >= PREFILTER_MIN_BUILD_SPAN)
                    }
                    _ => None,
                };
                if let Some(scan @ (FoldedScan::Sparse(_) | FoldedScan::Table(_))) = folded {
                    // Candidates descend from `start` to `end`: GNU's backward
                    // `re_search_2` tests one position per step.  The end of the
                    // text (no byte to test) is tried unconditionally, as the
                    // per-character loops do.
                    let mut upper = start;
                    if upper == text_len {
                        if let Some(result) = try_candidate!(text_len, start) {
                            break 'scan Some((text_len, result.1));
                        }
                        if end == text_len {
                            break 'scan None;
                        }
                        upper = text_len - 1;
                    }
                    // Invariant: `end <= upper < text_len`.
                    loop {
                        let window = &text[end..=upper];
                        let found = match scan {
                            FoldedScan::Sparse(SparseAsciiFastmap::One(b0)) => {
                                memchr::memrchr(*b0, window)
                            }
                            FoldedScan::Sparse(SparseAsciiFastmap::Two(b0, b1)) => {
                                memchr::memrchr2(*b0, *b1, window)
                            }
                            FoldedScan::Sparse(SparseAsciiFastmap::Three(b0, b1, b2)) => {
                                memchr::memrchr3(*b0, *b1, *b2, window)
                            }
                            FoldedScan::Table(accept) => {
                                window.iter().rposition(|&b| accept[b as usize])
                            }
                            FoldedScan::PerChar => unreachable!("excluded by the pattern above"),
                        };
                        let Some(idx) = found else {
                            break 'scan None;
                        };
                        let cand = end + idx;
                        // Backward candidates end at `start` (see below).
                        if let Some(result) = try_candidate!(cand, start) {
                            break 'scan Some((cand, result.1));
                        }
                        if cand == end {
                            break 'scan None;
                        }
                        upper = cand - 1;
                    }
                } else if let Some(table) = translate
                    && pattern.target_multibyte
                {
                    for pos in (end..=start).rev() {
                        if pos < text_len {
                            // As forward: the leading code of the translated
                            // character (GNU `re_search_2`).
                            let byte = text[pos];
                            let lead = if byte < 0x80 {
                                ascii_translated_leading_code(table, byte)
                            } else if (byte & 0xC0) == 0x80 {
                                continue;
                            } else {
                                multibyte_translated_leading_code(table, text, pos).0
                            };
                            if !pattern.fastmap_translated[lead] {
                                continue;
                            }
                        }
                        // Backward candidates end at `start` (see below).
                        if let Some(result) = try_candidate!(pos, start) {
                            break 'scan Some((pos, result.1));
                        }
                    }
                } else if let Some(table) = translate {
                    for pos in (end..=start).rev() {
                        // Skip UTF-8 continuation bytes — only try at character boundaries.
                        if pattern.target_multibyte && pos < text_len && (text[pos] & 0xC0) == 0x80
                        {
                            continue;
                        }
                        // GNU disables fastmap skipping for nullable patterns so zero-width
                        // matches like `\\(?:...\\)\\=` are still considered at every point.
                        if pos < text_len {
                            let idx = table.translate_byte(text[pos]) as usize;
                            if !pattern.fastmap[idx] {
                                continue;
                            }
                        }
                        // GNU `search.c:1195-1201` calls `re_search_2` for backward
                        // searches with STOP set to the point where the search began.
                        // That means a candidate may start before `start`, but it may
                        // not extend past it.  This prevents a repeated backward search
                        // from re-matching the same non-empty match that begins at
                        // point but ends after it.
                        if let Some(result) = try_candidate!(pos, start) {
                            break 'scan Some((pos, result.1));
                        }
                    }
                } else {
                    for pos in (end..=start).rev() {
                        if pattern.target_multibyte && pos < text_len && (text[pos] & 0xC0) == 0x80
                        {
                            continue;
                        }
                        if pos < text_len && !pattern.fastmap[text[pos] as usize] {
                            continue;
                        }
                        if let Some(result) = try_candidate!(pos, start) {
                            break 'scan Some((pos, result.1));
                        }
                    }
                }
            } else {
                for pos in (end..=start).rev() {
                    // Skip UTF-8 continuation bytes — only try at character boundaries.
                    if pattern.target_multibyte && pos < text_len && (text[pos] & 0xC0) == 0x80 {
                        continue;
                    }
                    // GNU `search.c:1195-1201` calls `re_search_2` for backward
                    // searches with STOP set to the point where the search began.
                    // That means a candidate may start before `start`, but it may
                    // not extend past it.  This prevents a repeated backward search
                    // from re-matching the same non-empty match that begins at
                    // point but ends after it.
                    if let Some(result) = try_candidate!(pos, start) {
                        break 'scan Some((pos, result.1));
                    }
                }
            }
        }

        None
    };
    // A successful scan keeps its first miss local. Whole failed searches
    // admit that miss for future searches; overflow returned above before
    // reaching this epilogue, quit aborts are excluded, and zero-span
    // attempts never marked it.
    if dfa_cold_heat.is_isolated() && result.is_none() && !crate::host::quit_pending() {
        drop(dfa::DfaLease::after_cold_failure(
            pattern,
            syntax,
            dfa_max_stop,
            1,
        ));
    }
    result
}
