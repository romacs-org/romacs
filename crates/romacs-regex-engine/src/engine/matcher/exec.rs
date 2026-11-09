//! The backtracker's dispatch loop over the bytecode.
//!
//! GNU: the body of `re_match_2_internal` in regex-emacs.c.

use crate::engine::matcher::fail::{
    FAIL_STACK_ENTRY_LIMIT, FailFrame, FailUndo, FailureInput, FailureOrigin, FailureResume,
    MatchScratch, PARITY_POP_BUDGET, check_infinite_loop, get_counter, goto_fail, set_counter,
    set_matcher_overflow,
};
#[cfg(any(test, feature = "test-hooks"))]
use crate::engine::matcher::route::note_fail_stack_push;
use crate::engine::matcher::route::set_pike_fallback;
use crate::engine::matcher::step::{
    SyntaxAssertion, evaluate_syntax_assertion, match_anychar_at, match_categoryspec_at,
    match_charset_at, match_exactn_char_at, match_syntaxspec_at, match_syntaxspecset_at,
};
use crate::engine::opcode::{RegexOp, extract_number, extract_number_u16};
use crate::engine::pattern::{CompiledPattern, MatchRegisters};
use crate::engine::syntax::SyntaxLookup;

#[allow(clippy::too_many_arguments)]
pub(crate) fn re_match_loop<const SEALED: bool>(
    scratch: &mut MatchScratch,
    pattern: &CompiledPattern,
    text: &[u8],
    pos: usize,
    stop: usize,
    syntax: &dyn SyntaxLookup,
    point: usize,
    // When true, abort with `set_pike_fallback()` if backtracking exceeds a
    // linear budget — the caller then re-runs on the Pike VM.  Only set for
    // `pike_eligible` patterns (the fallback is byte-exact there).
    enable_pike_fallback: bool,
    regs: &mut MatchRegisters,
) -> Option<usize> {
    let bytecode: &[u8] = pattern.buffer.as_slice();
    let num_regs = pattern.re_nsub + 1;

    // Catastrophic-backtracking budget: a CONSTANT cap on the number of
    // backtracks (failure-stack pops) in a single anchored match.  A
    // well-behaved pattern backtracks a number of times bounded by its own
    // structure (alternation arms, quantifier boundaries) — independent of
    // input length — so it never approaches the cap.  A pathological pattern
    // (`a*a*b`, `.*x` with no `x`) backtracks a number of times that grows
    // with the input, blows past the cap, and is handed to the linear Pike
    // VM.  Counting only at backtracks (not every op) keeps the well-behaved
    // hot path free of per-op overhead.
    const BACKTRACK_BUDGET: u64 = 8_000;
    let backtrack_budget: u64 = if enable_pike_fallback {
        BACKTRACK_BUDGET
    } else if scratch.parity_mode {
        PARITY_POP_BUDGET
    } else {
        u64::MAX
    };
    let mut backtrack_count: u64 = 0;

    let MatchScratch {
        frames,
        undo,
        regstart,
        regend,
        best_regstart,
        best_regend,
        counters,
        work,
        parity_abandoned,
        ..
    } = scratch;

    // Failure (choice-point) stack — GNU's delta protocol: frames record
    // only resume positions; register/counter deltas accumulate in `undo`
    // and are replayed on pop (see `FailFrame`).
    frames.clear();
    undo.clear();

    // Mutable counter table for interval repetition (succeed_n / jump_n /
    // set_number_at).  GNU modifies bytecode in-place; we use a side
    // table keyed by bytecode position.
    counters.clear();

    // GNU regex-emacs.c:4188-4204 skips all internal register arrays
    // when the pattern has no subexpressions. Register 0 is handled
    // separately on success, so failed no-group candidate checks should
    // not allocate register scratch space.
    let has_subexpressions = pattern.re_nsub > 0;
    let scratch_regs = if has_subexpressions { num_regs } else { 0 };
    // The scratch arrays are reused across every match call; when the
    // length already fits, reset in place — four out-of-line
    // SmallVec::resize calls per match measured ~275 Ir of pure setup.
    if regstart.len() == scratch_regs {
        regstart.fill(None);
        regend.fill(None);
    } else {
        regstart.clear();
        regstart.resize(scratch_regs, None);
        regend.clear();
        regend.resize(scratch_regs, None);
    }

    // Best match tracking for POSIX longest-match (audit #2).
    //
    // Mirrors GNU regex-emacs.c:4143-4154 and the main loop handling
    // at lines 4268-4345. When the pattern reaches its end with the
    // matcher positioned before the end of the searchable region
    // (`d < stop`), we save the current register state as the "best
    // so far" and force a backtrack to explore alternative paths.
    // After all backtracks have been exhausted, the best saved match
    // is restored. See GNU regex-emacs.c:5278-5279 for the
    // equivalent "restore after total failure" path.
    let posix_longest = pattern.posix;
    let mut best_regs_set = false;
    let mut best_match_end: usize = pos;
    // The best-match registers are read only under POSIX longest-match
    // (`posix_longest` below); a non-POSIX attempt skips their reset.
    if posix_longest {
        if best_regstart.len() == scratch_regs {
            best_regstart.fill(None);
            best_regend.fill(None);
        } else {
            best_regstart.clear();
            best_regstart.resize(scratch_regs, None);
            best_regend.clear();
            best_regend.resize(scratch_regs, None);
        }
    }

    let mut pc = 0usize; // Bytecode program counter
    let mut d = pos; // Data position in text

    let translate = &pattern.translate;
    let pattern_multibyte = pattern.multibyte;
    let target_multibyte = pattern.target_multibyte;

    // Helper: translate a character for case-folding.  All other
    // per-character predicates (decode, charset, syntax/category tests,
    // word/symbol boundaries) live in the shared free functions
    // (`match_*_at`, `assert_*`) so the Pike fast path is byte-exact with
    // this backtracker; `tr` remains local because only the `Duplicate`
    // (backreference) arm — which the Pike VM never runs — still needs it.
    let tr = |c: u32| -> u32 {
        if let Some(table) = translate {
            table.translate(c)
        } else {
            c
        }
    };

    // `try_fail!()` is the in-function replacement for GNU's
    // `goto fail`: pop the failure stack and resume there. If the
    // stack is empty, every backtracking avenue has been exhausted.
    // GNU's re_match_2_internal checks `best_regs_set` at that point
    // (regex-emacs.c:5278) and restores the saved best match instead
    // of returning -1. We do the same by setting `total_failure` and
    // breaking to the outer finalization block, which consults
    // `best_regs_set` to decide between returning None and restoring
    // the best registers.
    let mut total_failure = false;
    // `macro_rules!` labels are hygienic, so the label has to be
    // passed in explicitly as a `lifetime` metavariable.
    macro_rules! try_fail {
        ($label:lifetime) => {
            // Catastrophic-backtracking guard: past the constant backtrack
            // budget, bail so the caller re-runs on the Pike VM.  Cheap —
            // evaluated only at backtracks, not per op.
            backtrack_count += 1;
            *work += 1;
            if backtrack_count > backtrack_budget {
                if enable_pike_fallback {
                    set_pike_fallback();
                } else {
                    *parity_abandoned = true;
                }
                return None;
            }
            if goto_fail(&mut pc, &mut d, frames, undo, regstart, regend, counters).is_none() {
                total_failure = true;
                break $label;
            }
        };
    }

    // GNU `PUSH_FAILURE_POINT` incl. `ENSURE_FAIL_STACK`: refuse to grow
    // the stack past the `emacs_re_max_failures` budget; GNU returns -2
    // and `search.c:matcher_overflow` signals "Stack overflow in regexp
    // matcher".  We flag the thread-local and return no-match; the
    // front-end promotes the flag to the same error.
    macro_rules! push_failure_point {
        ($origin:expr, $resume:expr, $input:expr) => {
            if frames.len() + undo.len() >= FAIL_STACK_ENTRY_LIMIT {
                set_matcher_overflow();
                return None;
            }
            frames.push(FailFrame {
                undo_mark: undo.len(),
                origin: FailureOrigin($origin),
                resume: FailureResume($resume),
                input: $input,
            });
            #[cfg(any(test, feature = "test-hooks"))]
            note_fail_stack_push(frames.len() + undo.len(), d);
        };
    }

    // GNU `PUSH_FAILURE_REG` / `PUSH_NUMBER`: delta-save one register or
    // counter onto the shared undo log (subject to the same stack budget).
    macro_rules! push_failure_undo {
        ($entry:expr) => {
            if frames.len() + undo.len() >= FAIL_STACK_ENTRY_LIMIT {
                set_matcher_overflow();
                return None;
            }
            undo.push($entry);
            #[cfg(any(test, feature = "test-hooks"))]
            note_fail_stack_push(frames.len() + undo.len(), d);
        };
    }

    // Sealed-gated bytecode fetches for the loop below. SAFETY of the
    // SEALED arms: `validate_sealed_buffer` proved every operand span
    // in-bounds at every op boundary, and runtime `pc` values are closed
    // over validated boundaries (linear advances, validated jump targets,
    // fail-frame resumes derived from those).
    macro_rules! bc_num {
        ($pos:expr) => {{
            let pos = $pos;
            if SEALED {
                debug_assert!(pos + 1 < bytecode.len());
                unsafe {
                    i16::from_le_bytes([
                        *bytecode.get_unchecked(pos),
                        *bytecode.get_unchecked(pos + 1),
                    ])
                }
            } else {
                extract_number(bytecode, pos)
            }
        }};
    }
    macro_rules! bc_byte {
        ($pos:expr) => {{
            let pos = $pos;
            if SEALED {
                debug_assert!(pos < bytecode.len());
                unsafe { *bytecode.get_unchecked(pos) }
            } else {
                bytecode[pos]
            }
        }};
    }

    'main_loop: loop {
        // End of pattern = potential match.
        //
        // GNU regex-emacs.c:4272-4345: if we haven't consumed the
        // full match region and POSIX longest-match is requested,
        // save the current registers as the best seen so far and
        // force another backtrack. When no more backtracks remain,
        // restore whichever saved best is better than the final
        // candidate (regex-emacs.c:4323-4344).
        // SEALED buffers cannot reach pc == len: validation proves the last
        // op is Succeed/PosixEnd and every jump target lands strictly
        // inside the buffer, so the sealed loop drops this per-op check
        // (the terminal ops' arms carry the end-of-pattern logic).
        if !SEALED && pc >= bytecode.len() {
            if d > stop {
                try_fail!('main_loop);
                continue 'main_loop;
            }
            if posix_longest && d < stop {
                let better_than_best = !best_regs_set || d > best_match_end;
                if !frames.is_empty() {
                    if better_than_best {
                        best_regs_set = true;
                        best_match_end = d;
                        best_regstart.clone_from_slice(regstart);
                        best_regend.clone_from_slice(regend);
                    }
                    // Force a backtrack to explore alternative paths.
                    // The stack is non-empty so goto_fail cannot fail.
                    try_fail!('main_loop);
                    continue 'main_loop;
                } else if best_regs_set && !better_than_best {
                    // No more backtracks; the previously saved best
                    // beats the current finishing position.  Restore
                    // it before finalizing.
                    d = best_match_end;
                    // A pattern without groups keeps zero-length register vectors
                    // (`num_regs` is 1): the old index loop never touched them.
                    if num_regs > 1 {
                        regstart[1..num_regs].copy_from_slice(&best_regstart[1..num_regs]);
                        regend[1..num_regs].copy_from_slice(&best_regend[1..num_regs]);
                    }
                }
            }
            break 'main_loop;
        }
        if SEALED {
            debug_assert!(pc < bytecode.len());
        }

        let op_pc = pc;
        // SEALED: `pc < len` was just checked, validation proved every
        // in-buffer op boundary carries a valid opcode byte, and runtime
        // `pc` values are closed over validated boundaries.
        let op = if SEALED {
            // SAFETY: sealing proves that pc is an in-bounds op boundary,
            // and the matcher advances only to validated op boundaries.
            let op_byte = unsafe { *bytecode.get_unchecked(pc) };
            RegexOp::from_repr(op_byte).unwrap_or_else(|| {
                debug_assert!(false, "sealed bytecode contains an invalid opcode");
                // SAFETY: validate_sealed_buffer checked every opcode with
                // the same derived decoder after all compiler rewrites;
                // the sealed buffer is immutable during matching, and pc
                // remains on those validated opcode boundaries. The private
                // compiler also emits NoOp before Nastyloop retry markers.
                unsafe { std::hint::unreachable_unchecked() }
            })
        } else {
            let op_byte = bytecode[pc];
            let Some(op) = RegexOp::from_byte(op_byte) else {
                // Invalid opcode — treat as match failure
                return None;
            };
            op
        };
        pc += 1;

        match op {
            RegexOp::NoOp => {
                // Skip
            }

            RegexOp::Succeed => {
                // GNU regex-emacs.c:4429-4431 jumps directly to
                // `succeed_label`, bypassing the POSIX longest-match
                // check. For non-POSIX patterns, romacs's compiler
                // emits a trailing `Succeed` so the matcher exits as
                // soon as the pattern completes (mirroring GNU's
                // `if (!posix_backtracking) BUF_PUSH(succeed)` at
                // regex-emacs.c:2685). In POSIX mode the trailing
                // `Succeed` is NOT emitted, so the matcher instead
                // falls through to the end-of-bytecode check above.
                if d > stop {
                    try_fail!('main_loop);
                    continue 'main_loop;
                }
                break 'main_loop;
            }

            RegexOp::PosixEnd => {
                // The reified "fell off the end" path (see the opcode doc):
                // identical to the !SEALED end-of-bytecode block above.
                if d > stop {
                    try_fail!('main_loop);
                    continue 'main_loop;
                }
                if posix_longest && d < stop {
                    let better_than_best = !best_regs_set || d > best_match_end;
                    if !frames.is_empty() {
                        if better_than_best {
                            best_regs_set = true;
                            best_match_end = d;
                            best_regstart.clone_from_slice(regstart);
                            best_regend.clone_from_slice(regend);
                        }
                        // Force a backtrack to explore alternative paths.
                        // The stack is non-empty so goto_fail cannot fail.
                        try_fail!('main_loop);
                        continue 'main_loop;
                    } else if best_regs_set && !better_than_best {
                        // No more backtracks; the previously saved best
                        // beats the current finishing position.  Restore
                        // it before finalizing.
                        d = best_match_end;
                        // A pattern without groups keeps zero-length register vectors
                        // (`num_regs` is 1): the old index loop never touched them.
                        if num_regs > 1 {
                            regstart[1..num_regs].copy_from_slice(&best_regstart[1..num_regs]);
                            regend[1..num_regs].copy_from_slice(&best_regend[1..num_regs]);
                        }
                    }
                }
                break 'main_loop;
            }

            RegexOp::Exactn => {
                let count = bc_byte!(pc) as usize;
                pc += 1;
                let literal_start = pc;
                let literal_end = literal_start + count;
                let lit = &bytecode[literal_start..literal_end];
                let mut matched = true;
                let mut lit_off = 0usize;
                // Run-level fast path: compare the literal run against the text
                // window in one pass (no per-char helper call, no pre-scans).
                // With no case translation and one representation on both
                // sides, byte equality is character equality, so a full match
                // is decided outright; a mismatch is decided only when the
                // matched prefix is pure ASCII (then `d` advances over it
                // exactly as the per-char loop would), otherwise the per-char
                // loop below re-runs the run from the start.
                if count > 0
                    && d + count <= stop
                    && let Some(win) = text.get(d..d + count)
                {
                    let decided = match translate {
                        None if pattern_multibyte == target_multibyte => {
                            match win.iter().zip(lit).position(|(&tb, &lb)| tb != lb) {
                                None => Some(count),
                                Some(at)
                                    if win[..at].iter().all(|&b| b < 0x80)
                                        && lit[..at].iter().all(|&b| b < 0x80)
                                        && lit[at] < 0x80
                                        && win[at] < 0x80 =>
                                {
                                    Some(at)
                                }
                                Some(_) => None,
                            }
                        }
                        // Mixed representations (the common case: an ASCII
                        // unibyte pattern against a multibyte buffer): one pass
                        // that compares and checks ASCII on both sides; a
                        // non-ASCII byte on either side hands the run to the
                        // per-char loop.
                        None => {
                            let mut at = count;
                            let mut undecided = false;
                            for (i, (&tb, &lb)) in win.iter().zip(lit).enumerate() {
                                if tb >= 0x80 || lb >= 0x80 {
                                    undecided = true;
                                    break;
                                }
                                if tb != lb {
                                    at = i;
                                    break;
                                }
                            }
                            (!undecided).then_some(at)
                        }
                        Some(table) => {
                            let mut at = count;
                            let mut undecided = false;
                            for (i, (&tb, &lb)) in win.iter().zip(lit).enumerate() {
                                if tb >= 0x80 || lb >= 0x80 {
                                    undecided = true;
                                    break;
                                }
                                let tr = table.translate(tb as u32);
                                if tr >= 0x80 {
                                    undecided = true;
                                    break;
                                }
                                if tr as u8 != lb {
                                    at = i;
                                    break;
                                }
                            }
                            (!undecided).then_some(at)
                        }
                    };
                    if let Some(at) = decided {
                        d += at;
                        lit_off = at;
                        matched = at == count;
                    }
                }
                while matched && lit_off < count {
                    match match_exactn_char_at(
                        lit,
                        lit_off,
                        pattern_multibyte,
                        target_multibyte,
                        translate,
                        text,
                        d,
                        stop,
                    ) {
                        Some((pat_advance, text_advance)) => {
                            lit_off += pat_advance;
                            d += text_advance;
                        }
                        None => {
                            matched = false;
                            break;
                        }
                    }
                }
                pc = literal_end;
                if !matched {
                    // NOTE: try_fail! FALLS THROUGH after a successful pop
                    // (it only breaks the loop on total failure), so the
                    // fusion below must stay inside this else-less arm's
                    // matched path — hence the explicit continue here.
                    try_fail!('main_loop);
                    continue 'main_loop;
                }
                // Greedy exactn-loop fusion: `a+` / `a*` compiles to
                // `Exactn; OnFailureJump exit; Jump back` — the same
                // triple as the charset loops, with the same protocol:
                // one failure frame per iteration (original count and
                // payload) and the back-edge quit poll preserved. The
                // literal is re-matched whole each iteration, exactly
                // as jumping back to the Exactn op would.
                if SEALED
                    && unsafe { *bytecode.get_unchecked(pc) } == RegexOp::OnFailureJump as u8
                    && unsafe { *bytecode.get_unchecked(pc + 3) } == RegexOp::Jump as u8
                    && ((pc as i64) + 6 + bc_num!(pc + 4) as i64) as usize == op_pc
                {
                    let ofj_pc = pc;
                    let exit = ((ofj_pc as i64) + 3 + bc_num!(ofj_pc + 1) as i64) as usize;
                    'fused_exactn: loop {
                        push_failure_point!(ofj_pc, exit, FailureInput::Restore(d));
                        if crate::host::quit_pending() {
                            return None;
                        }
                        let mut lit_off = 0usize;
                        let mut dd = d;
                        while lit_off < count {
                            match match_exactn_char_at(
                                lit,
                                lit_off,
                                pattern_multibyte,
                                target_multibyte,
                                translate,
                                text,
                                dd,
                                stop,
                            ) {
                                Some((pat_advance, text_advance)) => {
                                    lit_off += pat_advance;
                                    dd += text_advance;
                                }
                                None => break 'fused_exactn,
                            }
                        }
                        d = dd;
                    }
                    try_fail!('main_loop);
                    continue 'main_loop;
                }
                // Keep-string loop fusion: a simple `P+`/`P*` whose
                // continuation is mutually exclusive with the body resolves
                // (via the PP* rewrite + resolve_smart_jumps) to
                // `OFKSJ exit; P; Jump P` — ONE failure frame for the whole
                // loop, back edge jumping straight to the body.  Run it
                // inline: no frames, no dispatch, just the match and the
                // quit poll; on failure the loop-entry OFKSJ frame resumes
                // at the exit with the string position kept, via try_fail.
                if SEALED
                    && unsafe { *bytecode.get_unchecked(pc) } == RegexOp::Jump as u8
                    && ((pc as i64) + 3 + bc_num!(pc + 1) as i64) as usize == op_pc
                {
                    'ks_exactn: loop {
                        if crate::host::quit_pending() {
                            return None;
                        }
                        let mut lit_off = 0usize;
                        let mut dd = d;
                        while lit_off < count {
                            match match_exactn_char_at(
                                lit,
                                lit_off,
                                pattern_multibyte,
                                target_multibyte,
                                translate,
                                text,
                                dd,
                                stop,
                            ) {
                                Some((pat_advance, text_advance)) => {
                                    lit_off += pat_advance;
                                    dd += text_advance;
                                }
                                None => break 'ks_exactn,
                            }
                        }
                        d = dd;
                    }
                    try_fail!('main_loop);
                    continue 'main_loop;
                }
                // Backtracking star-loop fusion: a simple loop whose
                // continuation is NOT provably exclusive resolves to
                // `OFJ exit; P; Jump OFJ` — per-iteration frames with the
                // OFJ at the loop head.  Same frame protocol as the
                // body-first `+` fusion above, entered from the body arm:
                // the loop-head OFJ pushed the first frame before this
                // iteration, so push AFTER each success, before the next
                // attempt.  (Sealed pass 2 proves the Jump target is an
                // opcode boundary, so the op_pc-3 peek reads a real op.)
                if SEALED
                    && op_pc >= 3
                    && unsafe { *bytecode.get_unchecked(pc) } == RegexOp::Jump as u8
                    && ((pc as i64) + 3 + bc_num!(pc + 1) as i64) as usize == op_pc - 3
                    && unsafe { *bytecode.get_unchecked(op_pc - 3) } == RegexOp::OnFailureJump as u8
                {
                    let ofj_pc = op_pc - 3;
                    let exit = ((ofj_pc as i64) + 3 + bc_num!(ofj_pc + 1) as i64) as usize;
                    'star_exactn: loop {
                        push_failure_point!(ofj_pc, exit, FailureInput::Restore(d));
                        if crate::host::quit_pending() {
                            return None;
                        }
                        let mut lit_off = 0usize;
                        let mut dd = d;
                        while lit_off < count {
                            match match_exactn_char_at(
                                lit,
                                lit_off,
                                pattern_multibyte,
                                target_multibyte,
                                translate,
                                text,
                                dd,
                                stop,
                            ) {
                                Some((pat_advance, text_advance)) => {
                                    lit_off += pat_advance;
                                    dd += text_advance;
                                }
                                None => break 'star_exactn,
                            }
                        }
                        d = dd;
                    }
                    try_fail!('main_loop);
                    continue 'main_loop;
                }
            }

            RegexOp::AnyChar => {
                match match_anychar_at(text, d, stop, target_multibyte, translate) {
                    Some(len) => {
                        d += len;
                        // Greedy anychar-loop fusion: `.*` / `.+` compiles to
                        // the same `AnyChar; OnFailureJump exit; Jump back`
                        // triple as the charset loops — identical protocol
                        // (see the Charset arm): per-iteration frames with
                        // the original payload, quit poll on the back edge.
                        if SEALED
                            && unsafe { *bytecode.get_unchecked(pc) }
                                == RegexOp::OnFailureJump as u8
                            && unsafe { *bytecode.get_unchecked(pc + 3) } == RegexOp::Jump as u8
                            && ((pc as i64) + 6 + bc_num!(pc + 4) as i64) as usize == op_pc
                        {
                            let ofj_pc = pc;
                            let exit = ((ofj_pc as i64) + 3 + bc_num!(ofj_pc + 1) as i64) as usize;
                            loop {
                                push_failure_point!(ofj_pc, exit, FailureInput::Restore(d));
                                if crate::host::quit_pending() {
                                    return None;
                                }
                                match match_anychar_at(text, d, stop, target_multibyte, translate) {
                                    Some(len) => d += len,
                                    None => break,
                                }
                            }
                            try_fail!('main_loop);
                            continue 'main_loop;
                        }
                        // Keep-string loop fusion (see the Exactn arm): the
                        // resolved `OFKSJ exit; AnyChar; Jump AnyChar` loop
                        // runs inline with no frames and no dispatch.
                        if SEALED
                            && unsafe { *bytecode.get_unchecked(pc) } == RegexOp::Jump as u8
                            && ((pc as i64) + 3 + bc_num!(pc + 1) as i64) as usize == op_pc
                        {
                            loop {
                                if crate::host::quit_pending() {
                                    return None;
                                }
                                match match_anychar_at(text, d, stop, target_multibyte, translate) {
                                    Some(len) => d += len,
                                    None => break,
                                }
                            }
                            try_fail!('main_loop);
                            continue 'main_loop;
                        }
                        // Backtracking star-loop fusion (see the Exactn arm):
                        // `OFJ exit; AnyChar; Jump OFJ` with per-iteration
                        // frames, run inline.
                        if SEALED
                            && op_pc >= 3
                            && unsafe { *bytecode.get_unchecked(pc) } == RegexOp::Jump as u8
                            && ((pc as i64) + 3 + bc_num!(pc + 1) as i64) as usize == op_pc - 3
                            && unsafe { *bytecode.get_unchecked(op_pc - 3) }
                                == RegexOp::OnFailureJump as u8
                        {
                            let ofj_pc = op_pc - 3;
                            let exit = ((ofj_pc as i64) + 3 + bc_num!(ofj_pc + 1) as i64) as usize;
                            loop {
                                push_failure_point!(ofj_pc, exit, FailureInput::Restore(d));
                                if crate::host::quit_pending() {
                                    return None;
                                }
                                match match_anychar_at(text, d, stop, target_multibyte, translate) {
                                    Some(len) => d += len,
                                    None => break,
                                }
                            }
                            try_fail!('main_loop);
                            continue 'main_loop;
                        }
                    }
                    None => {
                        try_fail!('main_loop);
                    }
                }
            }

            RegexOp::Charset | RegexOp::CharsetNot => {
                let charset_op_pos = pc - 1; // bytecode position of the opcode
                let bitmap_len = (bc_byte!(pc) & 0x7F) as usize;
                pc += 1;
                // The shared helper (see `match_charset_at`) holds the full
                // GNU `execute_charset` logic — the mutually-exclusive
                // unibyte-bitmap vs multibyte-range/class-bit branches — so
                // the Pike fast path stays byte-exact with this arm.
                let result = match_charset_at(
                    pattern,
                    charset_op_pos,
                    text,
                    d,
                    stop,
                    target_multibyte,
                    translate,
                    syntax,
                );
                pc += bitmap_len;
                match result {
                    Some(ch_len) => {
                        d += ch_len;
                        // Greedy charset-loop fusion: `Charset;
                        // OnFailureJump exit; Jump back` is the compiled
                        // shape of `[...]+` / `[...]*`. Run the loop here,
                        // skipping two dispatches per matched character —
                        // with per-iteration failure frames (push AFTER
                        // each success, exactly the original count and
                        // payload) and the back-edge quit poll preserved.
                        // Sealed only: the peeks rely on validated bounds.
                        if SEALED
                            && unsafe { *bytecode.get_unchecked(pc) }
                                == RegexOp::OnFailureJump as u8
                            && unsafe { *bytecode.get_unchecked(pc + 3) } == RegexOp::Jump as u8
                            && ((pc as i64) + 6 + bc_num!(pc + 4) as i64) as usize == charset_op_pos
                        {
                            let ofj_pc = pc;
                            let exit = ((ofj_pc as i64) + 3 + bc_num!(ofj_pc + 1) as i64) as usize;
                            loop {
                                push_failure_point!(ofj_pc, exit, FailureInput::Restore(d));
                                if crate::host::quit_pending() {
                                    return None;
                                }
                                match match_charset_at(
                                    pattern,
                                    charset_op_pos,
                                    text,
                                    d,
                                    stop,
                                    target_multibyte,
                                    translate,
                                    syntax,
                                ) {
                                    Some(ch_len) => d += ch_len,
                                    None => break,
                                }
                            }
                            try_fail!('main_loop);
                            continue 'main_loop;
                        }
                        // Keep-string loop fusion (see the Exactn arm): the
                        // resolved `OFKSJ exit; Charset; Jump Charset` loop
                        // runs inline with no frames and no dispatch.
                        if SEALED
                            && unsafe { *bytecode.get_unchecked(pc) } == RegexOp::Jump as u8
                            && ((pc as i64) + 3 + bc_num!(pc + 1) as i64) as usize == charset_op_pos
                        {
                            loop {
                                if crate::host::quit_pending() {
                                    return None;
                                }
                                match match_charset_at(
                                    pattern,
                                    charset_op_pos,
                                    text,
                                    d,
                                    stop,
                                    target_multibyte,
                                    translate,
                                    syntax,
                                ) {
                                    Some(ch_len) => d += ch_len,
                                    None => break,
                                }
                            }
                            try_fail!('main_loop);
                            continue 'main_loop;
                        }
                        // Backtracking star-loop fusion (see the Exactn arm):
                        // `OFJ exit; Charset; Jump OFJ` with per-iteration
                        // frames, run inline.
                        if SEALED
                            && charset_op_pos >= 3
                            && unsafe { *bytecode.get_unchecked(pc) } == RegexOp::Jump as u8
                            && ((pc as i64) + 3 + bc_num!(pc + 1) as i64) as usize
                                == charset_op_pos - 3
                            && unsafe { *bytecode.get_unchecked(charset_op_pos - 3) }
                                == RegexOp::OnFailureJump as u8
                        {
                            let ofj_pc = charset_op_pos - 3;
                            let exit = ((ofj_pc as i64) + 3 + bc_num!(ofj_pc + 1) as i64) as usize;
                            loop {
                                push_failure_point!(ofj_pc, exit, FailureInput::Restore(d));
                                if crate::host::quit_pending() {
                                    return None;
                                }
                                match match_charset_at(
                                    pattern,
                                    charset_op_pos,
                                    text,
                                    d,
                                    stop,
                                    target_multibyte,
                                    translate,
                                    syntax,
                                ) {
                                    Some(ch_len) => d += ch_len,
                                    None => break,
                                }
                            }
                            try_fail!('main_loop);
                            continue 'main_loop;
                        }
                    }
                    None => {
                        try_fail!('main_loop);
                    }
                }
            }

            RegexOp::StartMemory => {
                let group = bc_byte!(pc) as usize;
                pc += 1;
                if group < num_regs && group < regstart.len() {
                    // GNU start_memory: "In case we need to undo this
                    // operation (via backtracking)" — PUSH_FAILURE_REG
                    // saves the old (start, end) pair on the fail stack.
                    push_failure_undo!(FailUndo::Reg {
                        idx: group,
                        start: regstart[group],
                        end: regend[group],
                    });
                    regstart[group] = Some(d);
                }
            }

            RegexOp::StopMemory => {
                let group = bc_byte!(pc) as usize;
                pc += 1;
                if group < num_regs
                    && let Some(end) = regend.get_mut(group)
                {
                    // GNU stop_memory pushes nothing: undoing this write
                    // is unnecessary because the value is only read again
                    // after the next start_memory (which delta-saves it)
                    // or at match end (only reachable if this
                    // stop_memory was not undone).  regex-emacs.c:4608.
                    *end = Some(d);
                }
            }

            RegexOp::Duplicate => {
                let group = bc_byte!(pc) as usize;
                pc += 1;

                let Some(start) = regstart.get(group).copied().flatten() else {
                    try_fail!('main_loop);
                    continue;
                };
                let Some(end) = regend.get(group).copied().flatten() else {
                    try_fail!('main_loop);
                    continue;
                };

                let ref_len = end - start;
                if d + ref_len > stop {
                    try_fail!('main_loop);
                    continue;
                }

                // Compare the backreference text
                let mut matched = true;
                for i in 0..ref_len {
                    if tr(text[d + i].into()) != tr(text[start + i].into()) {
                        matched = false;
                        break;
                    }
                }
                if !matched {
                    try_fail!('main_loop);
                    continue;
                }
                d += ref_len;
            }

            RegexOp::BegLine => {
                if d == 0 || (d > 0 && text[d - 1] == b'\n') {
                    // At beginning of line — succeed
                } else {
                    try_fail!('main_loop);
                }
            }

            RegexOp::EndLine => {
                if d >= text.len() || text[d] == b'\n' {
                    // At end of line — succeed
                } else {
                    try_fail!('main_loop);
                }
            }

            RegexOp::BegBuf => {
                if d != 0 {
                    try_fail!('main_loop);
                }
            }

            RegexOp::EndBuf => {
                if d != text.len() {
                    try_fail!('main_loop);
                }
            }

            RegexOp::AtDot => {
                if d != point {
                    try_fail!('main_loop);
                }
            }

            RegexOp::WordBound
            | RegexOp::NotWordBound
            | RegexOp::WordBeg
            | RegexOp::WordEnd
            | RegexOp::SymBeg
            | RegexOp::SymEnd => {
                if !evaluate_syntax_assertion(
                    SyntaxAssertion::from_regex_op(op),
                    text,
                    d,
                    stop,
                    target_multibyte,
                    syntax,
                ) {
                    try_fail!('main_loop);
                }
            }

            RegexOp::SyntaxSpec => {
                let class_byte = bytecode[pc];
                pc += 1;
                match match_syntaxspec_at(
                    class_byte,
                    false,
                    text,
                    d,
                    stop,
                    target_multibyte,
                    syntax,
                ) {
                    Some(len) => d += len,
                    None => {
                        try_fail!('main_loop);
                    }
                }
            }

            RegexOp::NotSyntaxSpec => {
                let class_byte = bytecode[pc];
                pc += 1;
                match match_syntaxspec_at(class_byte, true, text, d, stop, target_multibyte, syntax)
                {
                    Some(len) => d += len,
                    None => {
                        try_fail!('main_loop);
                    }
                }
            }

            RegexOp::SyntaxSpecSet => {
                // Fused positive syntax-class set (see `RegexOp::SyntaxSpecSet`
                // and `fuse_syntaxspec_alternations`).  ONE `char_syntax`
                // lookup + a mask test replaces the per-branch
                // `on_failure_jump` chain and repeated lookups of the original
                // `\sA\|\sB\|...` alternation.
                let mask = extract_number_u16(bytecode, pc);
                pc += 2;
                match match_syntaxspecset_at(mask, text, d, stop, target_multibyte, syntax) {
                    Some(len) => d += len,
                    None => {
                        try_fail!('main_loop);
                    }
                }
            }

            RegexOp::CategorySpec => {
                let cat = bytecode[pc];
                pc += 1;
                match match_categoryspec_at(cat, false, text, d, stop, target_multibyte, syntax) {
                    Some(len) => d += len,
                    None => {
                        try_fail!('main_loop);
                    }
                }
            }

            RegexOp::NotCategorySpec => {
                let cat = bytecode[pc];
                pc += 1;
                match match_categoryspec_at(cat, true, text, d, stop, target_multibyte, syntax) {
                    Some(len) => d += len,
                    None => {
                        try_fail!('main_loop);
                    }
                }
            }

            RegexOp::Jump => {
                // Mirrors GNU `regex-emacs.c:4901`: poll quit at the
                // unconditional-jump site inside the matcher bytecode
                // dispatch loop. Gives interactive `C-g` a chance to
                // abort a pathological regex that would otherwise run
                // for many seconds on a large input.
                if crate::host::quit_pending() {
                    return None;
                }
                let offset = bc_num!(pc);
                pc = ((pc as i64) + 2 + (offset as i64)) as usize;
            }

            RegexOp::OnFailureJump => {
                let offset = bc_num!(pc);
                pc += 2;
                let fail_pc = ((pc as i64) + (offset as i64)) as usize;
                push_failure_point!(op_pc, fail_pc, FailureInput::Restore(d));
            }

            RegexOp::OnFailureKeepStringJump => {
                let offset = bc_num!(pc);
                pc += 2;
                let fail_pc = ((pc as i64) + (offset as i64)) as usize;
                push_failure_point!(op_pc, fail_pc, FailureInput::KeepCurrent);
            }

            RegexOp::OnFailureJumpLoop => {
                let offset = bc_num!(pc);
                pc += 2;
                let fail_pc = ((pc as i64) + (offset as i64)) as usize;
                // Check for infinite loop (empty match detection).
                if check_infinite_loop(frames, FailureOrigin(op_pc), d) {
                    // Would loop forever on empty match — skip the loop
                    pc = fail_pc;
                } else {
                    push_failure_point!(op_pc, fail_pc, FailureInput::Restore(d));
                }
            }

            RegexOp::OnFailureJumpNastyloop => {
                // GNU regex-emacs.c:4793-4806.  A non-greedy loop over a
                // nullable body keeps one failure point at a time, so the
                // cycle check looks for the marker frame that popping this
                // op's failure point pushed at the loop's `no_op` (the byte
                // before this op): finding it at the same position means the
                // last iteration matched nothing, and the "try again" option
                // is not pushed.
                let offset = bc_num!(pc);
                pc += 2;
                let fail_pc = ((pc as i64) + (offset as i64)) as usize;
                debug_assert_eq!(bytecode[op_pc - 1], RegexOp::NoOp as u8);
                if !check_infinite_loop(frames, FailureOrigin(op_pc - 1), d) {
                    push_failure_point!(op_pc, fail_pc, FailureInput::NastyloopRetry(d));
                }
            }

            RegexOp::OnFailureJumpSmart => {
                // Unreachable in practice: `resolve_smart_jumps` rewrites
                // every on_failure_jump_smart at compile time (GNU does
                // the same rewrite lazily at first execution,
                // regex-emacs.c:4864-4906).  Fall back to the safe plain
                // on_failure_jump semantics if one survives.
                let offset = bc_num!(pc);
                pc += 2;
                let fail_pc = ((pc as i64) + (offset as i64)) as usize;
                push_failure_point!(op_pc, fail_pc, FailureInput::Restore(d));
            }

            RegexOp::SucceedN => {
                // GNU: succeed_n  <jump-offset:2> <counter:2>
                // "Have to succeed matching what follows at least n times."
                // Counter lives at pc+2 (2 bytes).  When counter > 0 we
                // decrement and continue (must still succeed more times).
                // When counter == 0 we fall through to on_failure_jump_loop
                // semantics using the jump offset.
                let counter_pos = pc + 2; // bytecode position of the counter
                let count = get_counter(counters, bytecode, counter_pos);
                if count != 0 {
                    // Still must succeed more times — decrement & continue.
                    // GNU PUSH_NUMBER: the old value is delta-saved so
                    // backtracking restores it.
                    push_failure_undo!(FailUndo::Counter {
                        pos: counter_pos,
                        val: count,
                    });
                    set_counter(counters, counter_pos, count.saturating_sub(1));
                    pc += 4;
                } else {
                    // Counter exhausted — behave like on_failure_jump_loop.
                    // Read the jump offset and push a failure point.
                    let offset = bc_num!(pc);
                    pc += 2; // skip the offset field
                    let fail_pc = ((pc as i64) + (offset as i64)) as usize;
                    pc += 2; // skip the counter field
                    // Infinite-loop detection (same as OnFailureJumpLoop)
                    if check_infinite_loop(frames, FailureOrigin(op_pc), d) {
                        pc = fail_pc;
                    } else {
                        push_failure_point!(op_pc, fail_pc, FailureInput::Restore(d));
                    }
                }
            }

            RegexOp::JumpN => {
                // GNU: jump_n  <jump-offset:2> <counter:2>
                // "Originally, this is how many times we CAN jump."
                // If counter > 0, decrement and jump.
                // If counter == 0, skip past (don't jump).
                let counter_pos = pc + 2;
                let count = get_counter(counters, bytecode, counter_pos);
                if count != 0 {
                    // Decrement counter (delta-saved, GNU PUSH_NUMBER)
                    // and perform the unconditional jump.
                    push_failure_undo!(FailUndo::Counter {
                        pos: counter_pos,
                        val: count,
                    });
                    set_counter(counters, counter_pos, count.saturating_sub(1));
                    let offset = bc_num!(pc);
                    pc = ((pc as i64) + 2 + (offset as i64)) as usize;
                } else {
                    pc += 4; // Skip past offset + counter fields
                }
            }

            RegexOp::SetNumberAt => {
                // GNU: set_number_at  <offset-to-counter:2> <value:2>
                // Sets the counter at the given offset to the given value.
                // Used to reset interval counters at the start of a loop.
                let rel_offset = bc_num!(pc);
                pc += 2; // advance past the offset field
                let value = extract_number_u16(bytecode, pc);
                pc += 2; // advance past the value field
                // Target counter position: relative to position after
                // the offset field (same convention as GNU).
                let target_pos = ((pc as i64) - 2 + (rel_offset as i64)) as usize;
                // GNU set_number_at writes through PUSH_NUMBER too.
                push_failure_undo!(FailUndo::Counter {
                    pos: target_pos,
                    val: get_counter(counters, bytecode, target_pos),
                });
                set_counter(counters, target_pos, value);
            }
        }
    }

    // GNU regex-emacs.c:5278-5279: when the matcher breaks out of
    // the main loop due to total backtracking exhaustion, if a best
    // match was previously saved for POSIX longest-match, restore it
    // and fall through to the success path; otherwise there is no
    // match at all.
    if total_failure {
        if best_regs_set {
            d = best_match_end;
            // A pattern without groups keeps zero-length register vectors
            // (`num_regs` is 1): the old index loop never touched them.
            if num_regs > 1 {
                regstart[1..num_regs].copy_from_slice(&best_regstart[1..num_regs]);
                regend[1..num_regs].copy_from_slice(&best_regend[1..num_regs]);
            }
        } else {
            return None;
        }
    }

    // If we got here, we matched!
    // Fill in registers
    regs.start.clear();
    regs.end.clear();
    regs.start.push(pos as i64);
    regs.end.push(d as i64);
    for i in 1..num_regs {
        regs.start.push(
            regstart
                .get(i)
                .copied()
                .flatten()
                .map(|v| v as i64)
                .unwrap_or(-1),
        );
        regs.end.push(
            regend
                .get(i)
                .copied()
                .flatten()
                .map(|v| v as i64)
                .unwrap_or(-1),
        );
    }

    Some(d)
}
