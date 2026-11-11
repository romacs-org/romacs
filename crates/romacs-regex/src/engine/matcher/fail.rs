//! The failure stack: choice points, their undo log, GNU's overflow limit,
//! and the per-thread scratch a match reuses.
//!
//! GNU: `PUSH_FAILURE_POINT`, `POP_FAILURE_POINT`, `PUSH_FAILURE_REG`,
//! `PUSH_NUMBER`, `CHECK_INFINITE_LOOP` and `emacs_re_max_failures` in
//! regex-emacs.c.

use crate::engine::opcode::{RegexOp, extract_number_u16, opcode_len};
use smallvec::SmallVec;

/// Plain `Vec`: the register arrays live in the reusable `MatchScratch`, so
/// they never reallocate after warm-up, and a SmallVec's spilled-check on
/// every register access cost ~6% of the matcher loop on the org op.
type RegisterScratch = Vec<Option<usize>>;

/// A failure point (choice point) on the backtracking stack.
///
/// Mirrors GNU `PUSH_FAILURE_POINT` (regex-emacs.c:1080): a frame is just
/// the resume positions plus a mark into the shared undo log.  Register
/// and counter state is NOT snapshotted here — GNU's protocol delta-saves
/// a register when `start_memory` modifies it (`PUSH_FAILURE_REG`) and a
/// counter when it is written (`PUSH_NUMBER`); `POP_FAILURE_POINT`
/// replays the undo log down to the frame's mark.  The previous
/// implementation rebuilt a full register snapshot and cloned the counter
/// table on EVERY choice point, which was the dominant cost of
/// backtracking-heavy searches.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FailFrame {
    /// `undo.len()` at push time — POP replays undo entries above this.
    pub(crate) undo_mark: usize,

    /// Opcode that created this choice point.  GNU stores this as
    /// `FAILURE_PAT`; empty-loop detection compares this identity, not the
    /// address at which matching will resume.
    pub(crate) origin: FailureOrigin,

    /// Position in the bytecode to resume at when this choice is popped.
    pub(crate) resume: FailureResume,

    /// What popping this choice does to the input cursor.
    pub(crate) input: FailureInput,
}

/// Bytecode identity used only for GNU's empty-loop cycle detection.
///
/// This is deliberately a different type from [`FailureResume`]: two distinct
/// choice-point opcodes may jump to the same continuation, which must not make
/// them the same loop for `CHECK_INFINITE_LOOP`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FailureOrigin(pub(crate) usize);

/// Bytecode continuation restored by a backtrack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FailureResume(pub(crate) usize);

/// Input-cursor policy stored in a GNU regexp failure point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureInput {
    Restore(usize),
    KeepCurrent,
    /// The failure point of an `on_failure_jump_nastyloop` (GNU
    /// `PUSH_FAILURE_POINT (p - 3, d)`): popping it restores the position,
    /// pushes a [`FailureInput::NastyloopMarker`] and enters the loop body
    /// (regex-emacs.c:5249-5252).
    NastyloopRetry(usize),
    /// GNU's `no_op` cycle-detection frame, pushed when a nastyloop failure
    /// point is popped: it marks where one iteration of the loop began, and
    /// popping it just fails again (regex-emacs.c:5261-5263).
    NastyloopMarker(usize),
}

impl FailureInput {
    /// GNU `FAILURE_STR`: the string position recorded in the frame, or
    /// `None` for a keep-string frame (GNU pushes `NULL`).
    #[inline(always)]
    fn string_place(self) -> Option<usize> {
        match self {
            FailureInput::Restore(position)
            | FailureInput::NastyloopRetry(position)
            | FailureInput::NastyloopMarker(position) => Some(position),
            FailureInput::KeepCurrent => None,
        }
    }
}

/// One delta-undo entry (GNU `PUSH_FAILURE_REG` / `PUSH_NUMBER`).
#[derive(Clone, Copy, Debug)]
pub(crate) enum FailUndo {
    /// Register `idx`'s (start, end) before a `start_memory` overwrote it.
    Reg {
        idx: usize,
        start: Option<usize>,
        end: Option<usize>,
    },
    /// Counter at bytecode position `pos` before it was rewritten.
    Counter { pos: usize, val: u16 },
}

/// GNU `emacs_re_max_failures` (regex-emacs.c:893).
const EMACS_RE_MAX_FAILURES: usize = 40_000;
/// GNU `TYPICAL_FAILURE_SIZE` (regex-emacs.c:1110): estimated stack slots
/// per failure point; the fail stack refuses to grow beyond
/// `emacs_re_max_failures * TYPICAL_FAILURE_SIZE` slots.
const TYPICAL_FAILURE_SIZE: usize = 20;
/// Our frames and undo entries each stand for one 3-slot GNU stack item
/// (frame = [prev_frame, str, pat]; reg = [start, end, num]; counter =
/// [val, ptr, -1]), so the GNU slot budget divides by 3.
pub(crate) const FAIL_STACK_ENTRY_LIMIT: usize = EMACS_RE_MAX_FAILURES * TYPICAL_FAILURE_SIZE / 3;

/// Reusable per-thread matcher state.  `re_search` runs `re_match` once
/// per fastmap candidate; allocating the fail stack, register vectors and
/// counter table afresh for every candidate was measured as a major share
/// of search time.  Cleared (not deallocated) on each entry.
#[derive(Default)]
pub struct MatchScratch {
    pub(crate) frames: Vec<FailFrame>,
    pub(crate) undo: Vec<FailUndo>,
    pub(crate) regstart: RegisterScratch,
    pub(crate) regend: RegisterScratch,
    pub(crate) best_regstart: RegisterScratch,
    pub(crate) best_regend: RegisterScratch,
    /// Live interval-counter overrides, keyed by bytecode position of the
    /// 2-byte counter field.  An association list: patterns rarely have
    /// more than a couple of `\{n,m\}` counters.
    pub(crate) counters: SmallVec<[(usize, u16); 4]>,
    /// Work done by the candidates of the current search, in bytes stepped
    /// or failure points popped: `re_search` resets it and hands the rest of
    /// a forward search to the linear matcher once it exceeds what the text
    /// justifies (`search::search_work_budget`).
    pub(crate) work: u64,
    /// The work at which the current search switches ([`note_search_work`]);
    /// `u64::MAX` outside a forward search that may switch.
    pub(crate) work_budget: u64,
    /// The unbudgeted backtracker runs (for GNU's fail-stack overflow) under
    /// [`PARITY_POP_BUDGET`] instead of unbounded.
    pub(crate) parity_mode: bool,
    /// A parity run exhausted that budget in this search: the candidate is a
    /// freeze in GNU, not an overflow, so the linear answers stand from here
    /// on without the fail-stack model.
    pub(crate) parity_abandoned: bool,
}

/// Failure points a parity run of the backtracker may pop before it is
/// abandoned.  GNU overflows on the first deep descent of its depth-first
/// search -- a path that pushes past the limit does so before many pops --
/// so a run that has popped four stacks' worth without overflowing is the
/// exponential search Monnier describes as "Emacs froze", and its answer
/// (absent an overflow) is the linear matcher's.  A GNU run that would
/// overflow only after more pops than this is a recorded divergence.
pub(crate) const PARITY_POP_BUDGET: u64 = 4 * FAIL_STACK_ENTRY_LIMIT as u64;

thread_local! {
    /// Boxed so a search leases it by moving one pointer: the scratch's
    /// inline register arrays made each lease, and each return, a memcpy.
    pub(crate) static MATCH_SCRATCH: std::cell::RefCell<Option<Box<MatchScratch>>> =
        std::cell::RefCell::new(Some(Box::default()));

    /// Set when a match aborts on the GNU fail-stack limit.  `re_search`
    /// bails out of its candidate loop when it sees the flag; the
    /// front-end (`regex/mod.rs`) promotes a `None`-with-flag result into the
    /// GNU error `"Stack overflow in regexp matcher"` (search.c:78
    /// `matcher_overflow`, reached via `re_match_2_internal` returning
    /// -2).  Same TLS-flag idiom as the quit poll.
    ///
    /// The same cell carries [`MATCHER_SWITCH`]: the candidates of the current
    /// forward search have done more work than its budget, and the rest of the
    /// search should run in the linear matcher.  Sharing the cell keeps the
    /// per-candidate check `re_search` already made to one read.
    static MATCHER_OVERFLOW: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// [`MATCHER_OVERFLOW`] bit: GNU's fail-stack limit was hit.
pub(crate) const MATCHER_OVERFLOWED: u8 = 1;
/// [`MATCHER_OVERFLOW`] bit: the search's candidate work passed its budget.
const MATCHER_SWITCH: u8 = 2;

/// GNU `matcher_overflow` message (search.c:80).
pub const MATCHER_OVERFLOW_MESSAGE: &str = "Stack overflow in regexp matcher";

pub(crate) fn set_matcher_overflow() {
    MATCHER_OVERFLOW.with(|flag| flag.set(flag.get() | MATCHER_OVERFLOWED));
}

pub(crate) fn matcher_overflow_pending() -> bool {
    MATCHER_OVERFLOW.with(|flag| flag.get() & MATCHER_OVERFLOWED != 0)
}

/// Both bits of [`MATCHER_OVERFLOW`]: one read for `re_search`'s failed
/// candidates.
#[inline]
pub(crate) fn matcher_interrupt() -> u8 {
    MATCHER_OVERFLOW.with(std::cell::Cell::get)
}

pub(crate) fn clear_matcher_overflow() {
    MATCHER_OVERFLOW.with(|flag| flag.set(0));
}

/// Raise [`MATCHER_SWITCH`] once the search's candidate work passes its
/// budget.  Called where work is counted, off the per-candidate fast path.
#[inline]
pub(crate) fn note_search_work(scratch: &MatchScratch) {
    if scratch.work > scratch.work_budget {
        MATCHER_OVERFLOW.with(|flag| flag.set(flag.get() | MATCHER_SWITCH));
    }
}

/// Read-and-clear the overflow flag.  The front-end calls this after a
/// failed search to distinguish "no match" from "aborted on the
/// fail-stack limit" (which GNU signals as an error).
pub fn take_matcher_overflow() -> bool {
    MATCHER_OVERFLOW.with(|flag| flag.replace(0) & MATCHER_OVERFLOWED != 0)
}

/// Live counter overrides, keyed by bytecode position of the 2-byte
/// counter field.  GNU mutates the bytecode in place (`STORE_NUMBER`);
/// romacs keeps compiled patterns immutable and shared, so overrides
/// live in this small association list instead.
type CounterTable = SmallVec<[(usize, u16); 4]>;

/// Read a counter value, falling back to the bytecode if no override has
/// been stored yet.  Used by `succeed_n`, `jump_n`, and `set_number_at`.
pub(crate) fn get_counter(counters: &CounterTable, bytecode: &[u8], pos: usize) -> u16 {
    counters
        .iter()
        .find(|(counter_pos, _)| *counter_pos == pos)
        .map(|(_, val)| *val)
        .unwrap_or_else(|| extract_number_u16(bytecode, pos))
}

/// Store a counter value WITHOUT recording an undo entry.  Only the
/// backtracking pop path uses this directly; everything else goes through
/// the PUSH_NUMBER-style delta save in the matcher.
pub(crate) fn set_counter(counters: &mut CounterTable, pos: usize, val: u16) {
    if let Some(entry) = counters
        .iter_mut()
        .find(|(counter_pos, _)| *counter_pos == pos)
    {
        entry.1 = val;
    } else {
        counters.push((pos, val));
    }
}

/// Opcodes that push onto the backtracker's fail stack when they run: every
/// failure-point opcode (a fused literal or charset loop pushes the frames of
/// its own `on_failure_jump`) and `start_memory`'s register save.  Interval
/// counters push too; patterns with them never reach the callers.
pub(crate) fn fail_stack_push_sites(bytecode: &[u8]) -> usize {
    let mut sites = 0usize;
    let mut pc = 0usize;
    while pc < bytecode.len() {
        let Some(op) = RegexOp::from_byte(bytecode[pc]) else {
            return usize::MAX;
        };
        if matches!(
            op,
            RegexOp::OnFailureJump
                | RegexOp::OnFailureKeepStringJump
                | RegexOp::OnFailureJumpLoop
                | RegexOp::OnFailureJumpNastyloop
                | RegexOp::OnFailureJumpSmart
                | RegexOp::StartMemory
                | RegexOp::SucceedN
                | RegexOp::JumpN
                | RegexOp::SetNumberAt
        ) {
            sites += 1;
        }
        let Some(len) = opcode_len(bytecode, pc) else {
            return usize::MAX;
        };
        pc += len;
    }
    sites
}

/// Whether a backtracker path that consumes at most `consumed` characters
/// might fill GNU's fail stack.  Between two consumed characters a path runs
/// each push site at most twice: re-entering one without progress needs an
/// empty loop iteration, which `CHECK_INFINITE_LOOP` cuts on its second
/// visit.  A path's stack holds only its own live pushes, so it stays below
/// `(consumed + 1) * 2 * sites` entries.
pub fn fail_stack_may_overflow(bytecode: &[u8], consumed: usize) -> bool {
    fail_stack_may_overflow_with(fail_stack_push_sites(bytecode), consumed)
}

/// [`fail_stack_may_overflow`] with the push sites already counted.
#[inline]
pub fn fail_stack_may_overflow_with(push_sites: usize, consumed: usize) -> bool {
    consumed
        .saturating_add(1)
        .saturating_mul(2)
        .saturating_mul(push_sites)
        >= FAIL_STACK_ENTRY_LIMIT
}

/// The consumed spans that cannot overflow, as one bound: for every
/// `consumed`, `consumed < fail_stack_overflow_free_span(push_sites)` exactly
/// when `!fail_stack_may_overflow_with(push_sites, consumed)`.
pub fn fail_stack_overflow_free_span(push_sites: usize) -> usize {
    match push_sites.checked_mul(2) {
        Some(0) => usize::MAX,
        // `(consumed + 1) * per < LIMIT` iff `consumed + 1 <= (LIMIT - 1) / per`.
        Some(per) => (FAIL_STACK_ENTRY_LIMIT - 1) / per,
        None => 0,
    }
}

/// GNU `CHECK_INFINITE_LOOP` (regex-emacs.c:1049-1069): walk down the
/// failure stack while the recorded string position equals the current
/// one (or has keep-current input policy); a frame created by the same
/// failure opcode means the loop has made no progress since it was
/// last here — an empty-match cycle.
pub(crate) fn check_infinite_loop(frames: &[FailFrame], origin: FailureOrigin, d: usize) -> bool {
    for frame in frames.iter().rev() {
        match frame.input.string_place() {
            Some(sp) if sp != d => return false,
            _ => {
                if frame.origin == origin {
                    return true;
                }
            }
        }
    }
    false
}

/// Handle match failure — pop the failure stack and backtrack.
/// Returns None if the failure stack is empty (complete failure).
///
/// Mirrors GNU `POP_FAILURE_POINT` (regex-emacs.c:1128): replay the undo
/// log down to the popped frame's mark (restoring registers and counters
/// delta-saved since the push), then resume at the frame's positions.
#[inline(always)] // GNU's POP_FAILURE_POINT is a macro at the fail site; 70K failures per org op paid a 7-arg call each
pub(crate) fn goto_fail(
    pc: &mut usize,
    d: &mut usize,
    frames: &mut Vec<FailFrame>,
    undo: &mut Vec<FailUndo>,
    regstart: &mut RegisterScratch,
    regend: &mut RegisterScratch,
    counters: &mut CounterTable,
) -> Option<()> {
    // Mirrors GNU `regex-emacs.c:5236`: poll quit at the failure /
    // backtrack site. Backtracking loops are the worst offenders for
    // pathological-regex responsiveness; a `C-g` arriving mid-backtrack
    // aborts the entire search here so the evaluator can surface the
    // quit signal on its next `maybe_quit` poll.
    if crate::host::quit_pending() {
        return None;
    }
    let frame = pop_failure_point(frames, undo, regstart, regend, counters)?;
    *pc = frame.resume.0;
    match frame.input {
        FailureInput::Restore(position) => *d = position,
        FailureInput::KeepCurrent => {}
        FailureInput::NastyloopRetry(_) | FailureInput::NastyloopMarker(_) => {
            return goto_fail_nastyloop(frame, pc, d, frames, undo, regstart, regend, counters);
        }
    }
    Some(())
}

/// GNU `POP_FAILURE_POINT` proper: pop the top frame and replay the undo log
/// down to its mark.
#[inline(always)]
fn pop_failure_point(
    frames: &mut Vec<FailFrame>,
    undo: &mut Vec<FailUndo>,
    regstart: &mut RegisterScratch,
    regend: &mut RegisterScratch,
    counters: &mut CounterTable,
) -> Option<FailFrame> {
    let frame = frames.pop()?;
    while undo.len() > frame.undo_mark {
        match undo.pop().expect("undo log at least undo_mark deep") {
            FailUndo::Reg { idx, start, end } => {
                if idx < regstart.len() {
                    regstart[idx] = start;
                }
                if idx < regend.len() {
                    regend[idx] = end;
                }
            }
            FailUndo::Counter { pos, val } => set_counter(counters, pos, val),
        }
    }
    Some(frame)
}

/// The fail path for the two frames a non-greedy loop over a nullable body
/// pushes (GNU regex-emacs.c:5249-5263).  Only patterns containing
/// `on_failure_jump_nastyloop` ever get here.
///
/// * [`FailureInput::NastyloopRetry`]: push the iteration marker (at the
///   loop's `no_op`, which sits right before the nastyloop opcode), restore the
///   position and enter the loop body.  The marker replaces the popped frame,
///   so the stack cannot outgrow its limit here.
/// * [`FailureInput::NastyloopMarker`]: GNU's `case no_op: goto fail`: poll
///   quit and pop the next frame.
#[cold]
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn goto_fail_nastyloop(
    mut frame: FailFrame,
    pc: &mut usize,
    d: &mut usize,
    frames: &mut Vec<FailFrame>,
    undo: &mut Vec<FailUndo>,
    regstart: &mut RegisterScratch,
    regend: &mut RegisterScratch,
    counters: &mut CounterTable,
) -> Option<()> {
    loop {
        match frame.input {
            FailureInput::NastyloopRetry(position) => {
                let marker = FailureOrigin(frame.origin.0 - 1);
                frames.push(FailFrame {
                    undo_mark: undo.len(),
                    origin: marker,
                    resume: FailureResume(marker.0),
                    input: FailureInput::NastyloopMarker(position),
                });
                *d = position;
                *pc = frame.resume.0;
                return Some(());
            }
            FailureInput::NastyloopMarker(_) => {
                if crate::host::quit_pending() {
                    return None;
                }
                frame = pop_failure_point(frames, undo, regstart, regend, counters)?;
                *pc = frame.resume.0;
                match frame.input {
                    FailureInput::Restore(position) => {
                        *d = position;
                        return Some(());
                    }
                    FailureInput::KeepCurrent => return Some(()),
                    FailureInput::NastyloopRetry(_) | FailureInput::NastyloopMarker(_) => {}
                }
            }
            FailureInput::Restore(_) | FailureInput::KeepCurrent => {
                unreachable!("only nastyloop frames take the nastyloop fail path")
            }
        }
    }
}
