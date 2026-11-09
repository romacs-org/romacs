//! The matcher's entry points: one anchored match with the reusable
//! scratch, and the hand-off between the backtracker and the linear-time
//! matcher.
//!
//! GNU: `re_match_2` and the entry of `re_match_2_internal` in
//! regex-emacs.c.

pub(super) mod exec;
pub(super) mod fail;
pub(super) mod route;
pub(super) mod step;

use crate::engine::linear;
use crate::engine::matcher::exec::re_match_loop;
use crate::engine::matcher::fail::{
    MATCH_SCRATCH, MatchScratch, clear_matcher_overflow, fail_stack_may_overflow, note_search_work,
};
use crate::engine::matcher::route::{force_backtrack, force_pike, take_pike_fallback};
use crate::engine::pattern::{CompiledPattern, MatchRegisters};
use crate::engine::syntax::SyntaxLookup;

/// Match a compiled pattern against input text.
///
/// This is the core matching function, equivalent to GNU's `re_match_2_internal`.
///
/// # Arguments
/// * `pattern` - Compiled bytecode pattern
/// * `text` - Input text to match against
/// * `pos` - Starting position in text
/// * `stop` - End of matching region
/// * `syntax` - Syntax table for `\w`, `\b`, `\sC` etc.
/// * `point` - Buffer point position (for `\=` / AtDot)
///
/// # Returns
/// * `Some(end_pos)` if matched — end position of the match
/// * `None` if no match
pub fn re_match(
    pattern: &CompiledPattern,
    text: &[u8],
    pos: usize,
    stop: usize,
    syntax: &dyn SyntaxLookup,
    point: usize,
) -> Option<(usize, MatchRegisters)> {
    clear_matcher_overflow();
    re_match_candidate(pattern, text, pos, stop, syntax, point)
}

/// The per-thread `MatchScratch`, leased out of its cell for one search and
/// returned by `Drop` (so early returns and `?` hand it back too). A
/// re-entrant search finds the cell empty and works on a fresh one.
#[must_use = "the thread-local extent ends when this guard drops"]
pub(crate) struct MatchScratchLease {
    /// `Some` from `take` until `Drop` hands the box back.
    scratch: Option<Box<MatchScratch>>,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
static_assertions::assert_not_impl_any!(MatchScratchLease: Send, Sync);

impl std::fmt::Debug for MatchScratchLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MatchScratchLease")
            .field(
                "frame_capacity",
                &self.scratch.as_ref().map(|s| s.frames.capacity()),
            )
            .field(
                "undo_capacity",
                &self.scratch.as_ref().map(|s| s.undo.capacity()),
            )
            .finish_non_exhaustive()
    }
}

impl MatchScratchLease {
    #[inline]
    pub(crate) fn take() -> Self {
        Self {
            scratch: Some(MATCH_SCRATCH.with(|cell| match cell.try_borrow_mut() {
                Ok(mut cur) => cur.take().unwrap_or_default(),
                Err(_) => Box::default(),
            })),
            _thread: std::marker::PhantomData,
        }
    }
    #[inline]
    pub(crate) fn get(&mut self) -> &mut MatchScratch {
        self.scratch
            .as_deref_mut()
            .expect("a lease holds its scratch until it drops")
    }
}

impl Drop for MatchScratchLease {
    // Keep restoration at a call boundary: inlining the TLS cleanup into
    // `re_search` changes register allocation throughout its candidate loops.
    #[inline(never)]
    fn drop(&mut self) {
        crate::tls_scope::TlsScope::restore_now(&MATCH_SCRATCH, self.scratch.take());
    }
}

/// `re_match` without the overflow-flag reset — `re_search` uses this per
/// fastmap candidate so an overflow set by one candidate survives until
/// the search loop checks it.
fn re_match_candidate(
    pattern: &CompiledPattern,
    text: &[u8],
    pos: usize,
    stop: usize,
    syntax: &dyn SyntaxLookup,
    point: usize,
) -> Option<(usize, MatchRegisters)> {
    let mut regs = MatchRegisters::default();
    let end = MATCH_SCRATCH.with(|cell| match cell.try_borrow_mut() {
        Ok(mut scratch) => re_match_candidate_in(
            {
                let scratch = scratch.get_or_insert_with(Box::default);
                scratch.parity_abandoned = false;
                scratch.work_budget = u64::MAX;
                scratch
            },
            pattern,
            text,
            pos,
            stop,
            syntax,
            point,
            &mut regs,
        ),
        Err(_) => re_match_candidate_in(
            &mut MatchScratch::default(),
            pattern,
            text,
            pos,
            stop,
            syntax,
            point,
            &mut regs,
        ),
    })?;
    Some((end, regs))
}

/// One match attempt on a caller-held scratch: the search loop borrows the
/// per-thread scratch ONCE and tries every candidate on it (17.6K attempts
/// per org font-lock op paid a thread-local RefCell borrow each).
/// A match fills REGS (GNU's caller-owned `struct re_registers`) and returns
/// its end; a failed attempt writes nothing. Returning the registers by value
/// instead -- a 176-byte `Option<(usize, MatchRegisters)>` -- cost a `memcpy`
/// out of the return slot at EVERY candidate position, matched or not:
/// ~22K per org-editing operation, ~30 Ir each.
#[allow(clippy::too_many_arguments)]
pub(crate) fn re_match_candidate_in(
    scratch: &mut MatchScratch,
    pattern: &CompiledPattern,
    text: &[u8],
    pos: usize,
    stop: usize,
    syntax: &dyn SyntaxLookup,
    point: usize,
    regs: &mut MatchRegisters,
) -> Option<usize> {
    if pattern.linear.eligible() && force_pike() {
        return pike_match_into(pattern, text, pos, stop, syntax, point, regs);
    }
    let budgeted = pattern.linear.eligible() && !force_backtrack();
    let result = re_match_internal(
        scratch, pattern, text, pos, stop, syntax, point, budgeted, regs,
    );
    // The budgeted backtracker gave up on a catastrophic match: recompute it
    // linearly (and byte-exactly) with the Pike VM.
    if budgeted && take_pike_fallback() {
        return pike_fallback_into(scratch, pattern, text, pos, stop, syntax, point, regs);
    }
    result
}

/// The budgeted backtracker's hand-off: the Pike VM decides the candidate
/// linearly and byte-exactly, but it cannot run out of fail stack, and GNU's
/// backtracker can ("Stack overflow in regexp matcher", which search.c turns
/// into an error).  The backtracker's stack is bounded by the characters its
/// longest path consumes ([`fail_stack_may_overflow`]), which the Pike run
/// measures; when that bound reaches GNU's limit, the unbudgeted backtracker
/// decides the candidate instead, overflow included, at GNU's own cost.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn pike_fallback_into(
    scratch: &mut MatchScratch,
    pattern: &CompiledPattern,
    text: &[u8],
    pos: usize,
    stop: usize,
    syntax: &dyn SyntaxLookup,
    point: usize,
    regs: &mut MatchRegisters,
) -> Option<usize> {
    let outcome = linear::run_anchored(pattern, text, pos, stop, syntax, point);
    scratch.work = scratch
        .work
        .saturating_add(outcome.reach.saturating_sub(pos) as u64);
    note_search_work(scratch);
    if !scratch.parity_abandoned
        && fail_stack_may_overflow(&pattern.buffer, outcome.reach.saturating_sub(pos))
    {
        tracing::debug!(
            target: "romacs::regex",
            pos,
            reach = outcome.reach,
            "Pike fallback defers to the backtracker: its fail stack may overflow"
        );
        scratch.parity_mode = true;
        let found = re_match_internal(
            scratch, pattern, text, pos, stop, syntax, point, false, regs,
        );
        scratch.parity_mode = false;
        if !scratch.parity_abandoned {
            return found;
        }
        // GNU would still be searching here: the linear answer stands.
        tracing::debug!(target: "romacs::regex", pos, "parity backtracker abandoned");
    }
    let (end, found) = outcome.found?;
    *regs = found;
    Some(end)
}

/// [`pike_match`] with [`re_match_candidate_in`]'s out-parameter protocol.
#[cold]
fn pike_match_into(
    pattern: &CompiledPattern,
    text: &[u8],
    pos: usize,
    stop: usize,
    syntax: &dyn SyntaxLookup,
    point: usize,
    regs: &mut MatchRegisters,
) -> Option<usize> {
    let (end, found) = pike_match(pattern, text, pos, stop, syntax, point)?;
    *regs = found;
    Some(end)
}

#[allow(clippy::too_many_arguments)]
pub fn re_match_internal(
    scratch: &mut MatchScratch,
    pattern: &CompiledPattern,
    text: &[u8],
    pos: usize,
    stop: usize,
    syntax: &dyn SyntaxLookup,
    point: usize,
    enable_pike_fallback: bool,
    regs: &mut MatchRegisters,
) -> Option<usize> {
    // SEALED selects unchecked bytecode fetches inside the loop, justified
    // by `validate_sealed_buffer`. Hand-assembled (unsealed) buffers take
    // the fully checked instantiation.
    if pattern.buffer_sealed {
        re_match_loop::<true>(
            scratch,
            pattern,
            text,
            pos,
            stop,
            syntax,
            point,
            enable_pike_fallback,
            regs,
        )
    } else {
        re_match_loop::<false>(
            scratch,
            pattern,
            text,
            pos,
            stop,
            syntax,
            point,
            enable_pike_fallback,
            regs,
        )
    }
}

// ---------------------------------------------------------------------------
// The linear-time matcher (`linear.rs`): the backtracker's fallback for one
// catastrophic candidate, and `re_search`'s engine for a whole forward search
// once the candidate loop has done more work than the text justifies.
// ---------------------------------------------------------------------------

/// Anchored leftmost-first match of a linear-eligible pattern starting
/// exactly at `pos`.  Byte-exact with `re_match_internal` for such patterns.
pub fn pike_match(
    pattern: &CompiledPattern,
    text: &[u8],
    pos: usize,
    stop: usize,
    syntax: &dyn SyntaxLookup,
    point: usize,
) -> Option<(usize, MatchRegisters)> {
    linear::run_anchored(pattern, text, pos, stop, syntax, point).found
}

#[cfg(test)]
mod tls_scratch_tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn tls_scope_match_scratch_nested_leases_return_outer_box() {
        let mut outer = MatchScratchLease::take();
        let outer_ptr = std::ptr::from_mut(outer.get());
        let mut inner = MatchScratchLease::take();
        assert_ne!(std::ptr::from_mut(inner.get()), outer_ptr);
        drop(inner);
        drop(outer);
        let mut reused = MatchScratchLease::take();
        assert_eq!(std::ptr::from_mut(reused.get()), outer_ptr);
    }

    #[test]
    fn tls_scope_match_scratch_returns_box_during_unwind() {
        let mut lease = MatchScratchLease::take();
        let scratch_ptr = std::ptr::from_mut(lease.get());
        assert!(
            catch_unwind(AssertUnwindSafe(move || {
                let _lease = lease;
                panic!("exercise scratch unwind restoration");
            }))
            .is_err()
        );
        let mut reused = MatchScratchLease::take();
        assert_eq!(std::ptr::from_mut(reused.get()), scratch_ptr);
    }

    #[test]
    fn tls_scope_match_scratch_borrowed_cache_drop_does_not_panic() {
        let lease = MatchScratchLease::take();
        MATCH_SCRATCH.with(|cache| {
            let borrowed = cache.borrow();
            assert!(catch_unwind(AssertUnwindSafe(|| drop(lease))).is_ok());
            assert!(borrowed.is_none());
        });
    }
}
