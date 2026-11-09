use crate::engine::opcode::{RegexOp, extract_number, opcode_len, store_number};
use crate::engine::pattern::{CompiledPattern, RewindView};
use smallvec::SmallVec;

/// Prove the compiled buffer safe for the backtracker's SEALED (unchecked)
/// dispatch: a linear decode must consume the buffer exactly, every opcode
/// byte must name a real [`RegexOp`], every operand span stays in-bounds
/// (via [`opcode_len`]), charsets carry no in-buffer range table (romacs
/// stores multibyte ranges in side maps; the matcher's `pc` advance would
/// walk into an in-buffer table), and every jump target and counter
/// position lands on an op boundary / in-bounds. Runtime `pc` values are
/// closed over {linear advances, validated jump targets, fail-frame
/// resumes derived from those}, so a sealed loop's fetches never leave the
/// buffer.
pub(crate) fn validate_sealed_buffer(pattern: &CompiledPattern) -> bool {
    let bytecode = &pattern.buffer;
    let len = bytecode.len();
    let mut is_boundary = vec![false; len + 1];
    is_boundary[len] = true;

    // Pass 1: linear decode — boundaries, opcode validity, operand spans.
    let mut pc = 0usize;
    while pc < len {
        is_boundary[pc] = true;
        let Some(op) = RegexOp::from_byte(bytecode[pc]) else {
            return false;
        };
        let Some(op_len) = opcode_len(bytecode, pc) else {
            return false;
        };
        if pc + op_len > len {
            return false;
        }
        if matches!(op, RegexOp::Charset | RegexOp::CharsetNot) && bytecode[pc + 1] & 0x80 != 0 {
            return false;
        }
        pc += op_len;
    }
    if pc != len {
        return false;
    }
    // The sealed loop replaces the per-op end-of-bytecode check with the
    // guarantee that execution only ends at a terminal op.
    if len == 0 {
        return false;
    }
    let mut last_op_start = 0usize;
    let mut walk = 0usize;
    while walk < len {
        last_op_start = walk;
        walk += opcode_len(bytecode, walk).expect("pass 1 validated operand spans");
    }
    if !matches!(
        RegexOp::from_byte(bytecode[last_op_start]),
        Some(RegexOp::Succeed | RegexOp::PosixEnd)
    ) {
        return false;
    }

    // Pass 2: every jump target on a boundary; counter positions in-bounds.
    let mut pc = 0usize;
    while pc < len {
        let op = RegexOp::from_byte(bytecode[pc]).expect("pass 1 validated opcodes");
        match op {
            RegexOp::Jump
            | RegexOp::OnFailureJump
            | RegexOp::OnFailureKeepStringJump
            | RegexOp::OnFailureJumpLoop
            | RegexOp::OnFailureJumpNastyloop
            | RegexOp::OnFailureJumpSmart
            | RegexOp::SucceedN
            | RegexOp::JumpN => {
                // Matcher convention: target = op_pc + 3 + offset.
                // Strictly inside the buffer: the sealed loop drops the
                // end-of-bytecode check, so no jump may land at len — the
                // compiler ends every pattern with Succeed/PosixEnd at the
                // old buffer end, which is where end-targeting jumps land.
                let offset = extract_number(bytecode, pc + 1);
                let target = pc as i64 + 3 + offset as i64;
                if target < 0 || target >= len as i64 || !is_boundary[target as usize] {
                    return false;
                }
            }
            RegexOp::SetNumberAt => {
                // Matcher convention: counter position = op_pc + 1 + offset,
                // read/written as a 2-byte number.
                let offset = extract_number(bytecode, pc + 1);
                let target = pc as i64 + 1 + offset as i64;
                if target < 0 || target + 2 > len as i64 {
                    return false;
                }
            }
            _ => {}
        }
        pc += opcode_len(bytecode, pc).expect("pass 1 validated operand spans");
    }
    true
}

/// Build the [`RewindView`]: de-optimize every keep-string smart loop
/// (`OnFailureKeepStringJump` whose jump-back targets the loop body) into the
/// equivalent rewind loop (`OnFailureJump` whose jump-back targets the split).
///
/// Every `OnFailureKeepStringJump` the compiler emits comes from
/// `resolve_smart_jumps`, so each must have the smart-loop shape; one that
/// does not makes the view [`RewindView::Unavailable`].
pub(crate) fn build_rewind_view(orig: &[u8]) -> RewindView {
    match rewrite_keep_string_loops(orig) {
        Some(None) => RewindView::Buffer,
        Some(Some(rewritten)) => RewindView::Rewritten(rewritten.into_boxed_slice()),
        None => RewindView::Unavailable,
    }
}

/// `Some(None)` when nothing needs rewriting, `Some(Some(copy))` with the
/// rewritten copy, `None` for an unexpected keep-string shape.
#[allow(clippy::option_option)]
fn rewrite_keep_string_loops(orig: &[u8]) -> Option<Option<Vec<u8>>> {
    let mut rewritten: Option<Vec<u8>> = None;
    let mut pc = 0usize;
    while pc < orig.len() {
        let op = RegexOp::from_byte(orig[pc])?;
        if op == RegexOp::OnFailureKeepStringJump {
            // Expected smart keep-string loop shape (after `resolve_smart_jumps`):
            //   pc:      OnFailureKeepStringJump  → p2 (LEXIT)
            //   pc+3:    <one-char body>
            //   p2 - 3:  Jump                     → pc + 3 (body start)
            //   p2:      <continuation>
            let offset = extract_number(orig, pc + 1);
            let p2 = (pc as i64 + 3 + offset as i64) as usize;
            if p2 < pc + 6 || p2 > orig.len() {
                return None;
            }
            let jump_at = p2 - 3;
            if orig.get(jump_at).copied() != Some(RegexOp::Jump as u8) {
                return None;
            }
            let jtarget = (jump_at as i64 + 3 + extract_number(orig, jump_at + 1) as i64) as usize;
            if jtarget != pc + 3 {
                return None;
            }
            // Rewrite: OFKSJ → OnFailureJump (same offset to LEXIT), and
            // retarget the jump-back from the body (pc+3) to the split (pc).
            let out = rewritten.get_or_insert_with(|| orig.to_vec());
            out[pc] = RegexOp::OnFailureJump as u8;
            let new_off = pc as i64 - (jump_at as i64 + 3);
            store_number(out, jump_at + 1, new_off as i16);
        }
        pc += opcode_len(orig, pc)?;
    }
    Some(rewritten)
}

/// Walk the compiled bytecode and decide whether the Pike VM (see
/// [`pike_match`]) can simulate it byte-exactly.
///
/// The Pike VM reproduces Emacs's leftmost-greedy NFA semantics by
/// exploring the SAME bytecode the backtracker does, in the SAME priority
/// order, but linearly.  These force a fall back to the backtracker:
///   * `Duplicate` — a backreference is not a regular-language construct.
///   * `SucceedN` / `JumpN` / `SetNumberAt` — `\{n,m\}` interval counters
///     carry mutable per-position state that would break the Pike VM's
///     `(pc)`-keyed thread dedup; conservatively excluded in Stage 1.
///   * `OnFailureJumpNastyloop` — a non-greedy quantifier over a NULLABLE
///     body (`\(?:a\|\)*?`).  The backtracker's cycle check for it walks
///     GNU's per-iteration marker frames, whose zero-progress ordering the
///     Pike VM's `seen` dedup does not model, so it is excluded.
///     (Non-greedy quantifiers over NON-nullable bodies use
///     `OnFailureJump` and stay eligible.)
///   * A non-greedy `??` (`has_nongreedy_optional`) — it compiles to GNU's
///     skip-first `on_failure_jump` + `jump` shape (an earlier emit used a
///     keep-string jump); the Pike VM's priority order has not been
///     validated on it, so it stays excluded.
///   * POSIX-longest mode (`buf.posix`) — the Pike VM produces
///     leftmost-greedy captures, not POSIX-longest, so it must not run
///     for a POSIX pattern.
///   * A **capture group inside a zero-consume (epsilon) cycle**, e.g.
///     `\(a*\)*` or `\(.??\B\)+?`.  When a `*` / `+` body can iterate
///     without consuming a character, GNU's `check_infinite_loop` takes
///     exactly ONE zero-progress iteration and KEEPS that iteration's
///     captures before forcing the loop exit; the Pike VM's `seen`-set
///     prunes the zero-progress path entirely, losing those captures.
///     Reproducing GNU's empty-loop capture semantics byte-exactly is out
///     of scope for Stage 1.  Crucially this is checked PRECISELY (an
///     actual epsilon cycle through a `StartMemory`/`StopMemory`), NOT by
///     excluding `OnFailureJumpLoop` wholesale — that op is emitted
///     conservatively for many NON-nullable bodies (e.g. the fontlock
///     `\(?:\w\|\s_\|\\.\)+`), which the Pike VM handles correctly as a
///     plain split.
///
/// Every other op the compiler can emit (literals, `.`, charsets,
/// `* + ?` and their non-greedy forms, alternation, groups, anchors,
/// boundaries, `\w \W \sC \SC` and the fused `SyntaxSpecSet`, categories,
/// `\=`) is handled by the Pike VM.  Unknown/malformed bytes make the
/// pattern ineligible (fail closed).
pub(crate) fn compute_pike_eligible(buf: &CompiledPattern) -> bool {
    if buf.posix {
        return false;
    }
    // Non-greedy `??`: see the doc above.
    if buf.has_nongreedy_optional {
        return false;
    }
    let bytecode = &buf.buffer;
    let mut pc = 0usize;
    while pc < bytecode.len() {
        let Some(op) = RegexOp::from_byte(bytecode[pc]) else {
            return false;
        };
        match op {
            RegexOp::Duplicate
            | RegexOp::SucceedN
            | RegexOp::JumpN
            | RegexOp::SetNumberAt
            // A non-greedy quantifier whose body can match empty (`a*?`/`+?`
            // with a nullable body) compiles to `OnFailureJumpNastyloop`,
            // whose cycle check reads GNU's per-iteration marker frames
            // (`goto_fail_nastyloop`); the Pike VM does not model that
            // zero-progress ordering, so fall back conservatively.
            | RegexOp::OnFailureJumpNastyloop => return false,
            _ => {}
        }
        let Some(len) = opcode_len(bytecode, pc) else {
            return false;
        };
        pc += len;
    }
    // Precise empty-loop-with-capture check (see doc above).
    !has_capturing_epsilon_cycle(bytecode)
}

/// The non-consuming (epsilon) successors of the op at `pc`.  Consuming ops
/// and `Succeed`/end-of-program are epsilon dead-ends (they break any
/// zero-consume cycle).  Used only by the eligibility analysis.
fn epsilon_successors(bytecode: &[u8], pc: usize) -> SmallVec<[usize; 2]> {
    let mut out: SmallVec<[usize; 2]> = SmallVec::new();
    let Some(op) = RegexOp::from_byte(bytecode[pc]) else {
        return out;
    };
    match op {
        RegexOp::NoOp
        | RegexOp::BegLine
        | RegexOp::EndLine
        | RegexOp::BegBuf
        | RegexOp::EndBuf
        | RegexOp::AtDot
        | RegexOp::WordBound
        | RegexOp::NotWordBound
        | RegexOp::WordBeg
        | RegexOp::WordEnd
        | RegexOp::SymBeg
        | RegexOp::SymEnd => out.push(pc + 1),
        RegexOp::StartMemory | RegexOp::StopMemory => out.push(pc + 2),
        RegexOp::Jump => {
            let offset = extract_number(bytecode, pc + 1);
            out.push((pc as i64 + 3 + offset as i64) as usize);
        }
        RegexOp::OnFailureJump
        | RegexOp::OnFailureKeepStringJump
        | RegexOp::OnFailureJumpLoop
        | RegexOp::OnFailureJumpNastyloop
        | RegexOp::OnFailureJumpSmart => {
            let offset = extract_number(bytecode, pc + 1);
            out.push(pc + 3);
            out.push((pc as i64 + 3 + offset as i64) as usize);
        }
        // Consuming ops, Succeed, and the ineligible ops break the cycle.
        _ => {}
    }
    out
}

/// True if some `StartMemory` / `StopMemory` op lies on a zero-consume
/// (epsilon-only) cycle — the precise condition under which GNU's
/// empty-loop capture semantics diverge from the Pike VM.
fn has_capturing_epsilon_cycle(bytecode: &[u8]) -> bool {
    let mut pc = 0usize;
    while pc < bytecode.len() {
        let Some(op) = RegexOp::from_byte(bytecode[pc]) else {
            return false;
        };
        if matches!(op, RegexOp::StartMemory | RegexOp::StopMemory)
            && epsilon_node_on_cycle(bytecode, pc)
        {
            return true;
        }
        let Some(len) = opcode_len(bytecode, pc) else {
            return false;
        };
        pc += len;
    }
    false
}

/// DFS over epsilon edges: can `start` reach itself without consuming?
fn epsilon_node_on_cycle(bytecode: &[u8], start: usize) -> bool {
    let mut stack: Vec<usize> = epsilon_successors(bytecode, start).to_vec();
    let mut visited = vec![false; bytecode.len() + 1];
    while let Some(n) = stack.pop() {
        if n == start {
            return true;
        }
        if n >= bytecode.len() || visited[n] {
            continue;
        }
        visited[n] = true;
        stack.extend(epsilon_successors(bytecode, n));
    }
    false
}
