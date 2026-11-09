use crate::engine::opcode::{RegexOp, extract_number, opcode_len, store_number, store_number_u16};
use crate::engine::pattern::CompiledPattern;

// ---------------------------------------------------------------------------
// Peephole: fuse alternations of single positive syntax-class tests.
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// Test-only switch to compile WITHOUT the syntax-class fusion peephole,
    /// so an A/B benchmark can time the exact same pattern fused vs unfused
    /// interleaved in one process (immune to whole-machine timing noise).
    static SYNTAX_FUSION_DISABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` with the syntax-class fusion peephole disabled (test only).
#[cfg(any(test, feature = "test-hooks"))]
pub fn with_syntax_fusion_disabled<R>(f: impl FnOnce() -> R) -> R {
    SYNTAX_FUSION_DISABLED.with(|flag| flag.set(true));
    let r = f();
    SYNTAX_FUSION_DISABLED.with(|flag| flag.set(false));
    r
}

/// A fusible alternation branch of the exact 8-byte "triple" shape
/// `on_failure_jump(+5); syntaxspec(c); jump(->…->END)` that the alternation
/// compiler emits for a branch whose body is a single positive `\sC` / `\w`.
struct SyntaxTriple {
    /// Syntax class byte matched by the branch's `syntaxspec`.
    class: u8,
    /// Ultimate alternation END reached by following the branch's trailing
    /// `jump` chain (each non-final branch's `jump` targets the NEXT
    /// branch's `jump`, so the chain hops forward to the true END).
    end: usize,
}

/// If a maximal run of fusible positive-`syntaxspec` alternation branches
/// begins at `p`, describe it; otherwise `None`.
///
/// A run is a sequence of contiguous `on_failure_jump(+5); syntaxspec;
/// jump` triples (all `syntaxspec` — the positive form only — and all
/// resolving to the same alternation END), optionally extended by a
/// trailing *final* bare `syntaxspec` branch whose fall-through IS that END.
/// The trailing `jump` must point strictly FORWARD (past the triple): that
/// is what distinguishes an alternation branch from the visually identical
/// `on_failure_jump; body; jump(back)` shape of a resolved `*`/`+` loop.
struct FusibleRun {
    /// Union bitmask of all fused branches' syntax classes.
    mask: u16,
    /// One-past-the-end bytecode position of the whole rewritten region.
    region_end: usize,
    layout: FusionLayout,
}

enum FusionLayout {
    /// The run is followed by a further (non-fused) alternation branch at
    /// `p_next`; keep a single `on_failure_jump -> p_next` and a
    /// `jump -> end` around the fused op.
    Chain { p_next: usize, end: usize },
    /// The run reaches the alternation's final bare branch; the fused op
    /// simply falls through to `region_end` on success and fails to the
    /// enclosing failure point otherwise (no `on_failure_jump`/`jump`).
    FinalBare,
}

/// Resolve the absolute target of the offset-op at `pos` (`pc + 2 + offset`
/// with `pc` after the opcode byte).
fn jump_target(bytecode: &[u8], pos: usize) -> Option<usize> {
    if pos + 3 > bytecode.len() {
        return None;
    }
    let t = (pos as i64) + 3 + (extract_number(bytecode, pos + 1) as i64);
    if t < 0 || t as usize > bytecode.len() {
        return None;
    }
    Some(t as usize)
}

/// Follow a forward chain of `jump` ops starting at `pos`, returning the
/// first non-`jump` landing position.  `None` if the chain is malformed or
/// not strictly forward (which would risk a cycle).
fn follow_jump_chain(bytecode: &[u8], mut pos: usize) -> Option<usize> {
    // Each hop moves strictly forward, so the chain length is bounded by the
    // buffer size; no separate cycle guard is needed.
    while pos < bytecode.len() && bytecode[pos] == RegexOp::Jump as u8 {
        let next = jump_target(bytecode, pos)?;
        if next <= pos {
            return None; // must move strictly forward
        }
        pos = next;
    }
    Some(pos)
}

/// Recognize a single fusible triple starting at `q`.
fn parse_syntax_triple(bytecode: &[u8], q: usize) -> Option<SyntaxTriple> {
    if q + 8 > bytecode.len() {
        return None;
    }
    if bytecode[q] != RegexOp::OnFailureJump as u8 {
        return None;
    }
    // `on_failure_jump` must skip exactly the 2-byte syntaxspec + 3-byte
    // jump that follow it (target == q+8 = the next branch).
    if extract_number(bytecode, q + 1) != 5 {
        return None;
    }
    if bytecode[q + 3] != RegexOp::SyntaxSpec as u8 {
        return None;
    }
    let class = bytecode[q + 4];
    if bytecode[q + 5] != RegexOp::Jump as u8 {
        return None;
    }
    // The branch's `jump` target begins a (possibly one-hop) forward chain
    // of `jump`s that lands at the alternation END.  A strictly-forward
    // chain excludes the backward `jump` of a resolved `*`/`+` loop.
    let immediate = jump_target(bytecode, q + 5)?;
    if immediate <= q + 8 {
        return None;
    }
    let end = follow_jump_chain(bytecode, immediate)?;
    Some(SyntaxTriple { class, end })
}

fn detect_fusible_run(bytecode: &[u8], p: usize) -> Option<FusibleRun> {
    let first = parse_syntax_triple(bytecode, p)?;
    let end = first.end;
    let mut mask: u16 = 1u16 << first.class;
    let mut count = 1usize;
    let mut tail = p + 8; // start of the next branch after the run so far

    // Extend the run with further contiguous triples resolving to the same END.
    while let Some(t) = parse_syntax_triple(bytecode, tail) {
        if t.end != end {
            break;
        }
        mask |= 1u16 << t.class;
        count += 1;
        tail += 8;
    }

    // Optionally absorb the alternation's final bare `syntaxspec` branch:
    // it has no `on_failure_jump`/`jump`, and its fall-through position IS
    // the shared END.
    let final_bare = tail + 2 <= bytecode.len()
        && bytecode[tail] == RegexOp::SyntaxSpec as u8
        && tail + 2 == end;

    if final_bare {
        mask |= 1u16 << bytecode[tail + 1];
        // Need at least two fused branches for the rewrite to pay off.
        if count + 1 < 2 {
            return None;
        }
        Some(FusibleRun {
            mask,
            region_end: end, // == tail + 2
            layout: FusionLayout::FinalBare,
        })
    } else {
        // The run is followed by an unrelated branch at `tail`; keep the
        // alternation edge to it.  A single triple gains nothing here.
        if count < 2 {
            return None;
        }
        Some(FusibleRun {
            mask,
            region_end: tail,
            layout: FusionLayout::Chain { p_next: tail, end },
        })
    }
}

/// True if any control-transfer op ORIGINATING OUTSIDE `[p, region_end)`
/// targets a position in the OPEN interval `(p, region_end)`.  The fusion
/// overwrites the region's interior bytes, so a surviving *external*
/// reference into them would corrupt matching; if one exists we
/// conservatively skip the rewrite.  Ops inside the region are ignored:
/// they are the alternation's own branch jumps, which the fusion subsumes.
/// `p` itself stays a valid opcode boundary and is an allowed target.
fn any_target_inside(bytecode: &[u8], p: usize, region_end: usize) -> bool {
    let mut pc = 0usize;
    while pc < bytecode.len() {
        let Some(len) = opcode_len(bytecode, pc) else {
            // Malformed walk: be conservative and assume a hazard.
            return true;
        };
        if pc >= p && pc < region_end {
            // Interior op — overwritten by the fusion; its target is moot.
            pc += len;
            continue;
        }
        let target = match RegexOp::from_byte(bytecode[pc]) {
            Some(
                RegexOp::Jump
                | RegexOp::OnFailureJump
                | RegexOp::OnFailureKeepStringJump
                | RegexOp::OnFailureJumpLoop
                | RegexOp::OnFailureJumpNastyloop
                | RegexOp::OnFailureJumpSmart
                | RegexOp::SucceedN
                | RegexOp::JumpN
                | RegexOp::SetNumberAt,
            ) => {
                // All offset-bearing ops store their 2-byte offset right
                // after the opcode byte and resolve as `pc + 2 + offset`
                // with `pc` positioned after that opcode byte (== `pos+3`).
                Some(((pc + 1) as i64 + 2 + extract_number(bytecode, pc + 1) as i64) as usize)
            }
            _ => None,
        };
        if let Some(t) = target
            && t > p
            && t < region_end
        {
            return true;
        }
        pc += len;
    }
    false
}

/// Post-compile peephole that fuses alternations of single positive
/// syntax-class branches (`\w\|\s_`, `\sw\|\s_\|\s.`, …) into one
/// `SyntaxSpecSet` op.  Length-preserving: the fused op plus `NoOp` padding
/// exactly fills the original region, so every other jump offset,
/// HashMap-keyed position, and the fastmap walk stay valid without
/// relocation.  See `RegexOp::SyntaxSpecSet` for the correctness argument.
pub(crate) fn fuse_syntaxspec_alternations(buf: &mut CompiledPattern) {
    #[cfg(any(test, feature = "test-hooks"))]
    if SYNTAX_FUSION_DISABLED.with(std::cell::Cell::get) {
        return;
    }
    let mut p = 0usize;
    while p < buf.buffer.len() {
        if let Some(run) = detect_fusible_run(&buf.buffer, p)
            && !any_target_inside(&buf.buffer, p, run.region_end)
        {
            apply_fusion(&mut buf.buffer, p, &run);
            p = run.region_end;
            continue;
        }
        match opcode_len(&buf.buffer, p) {
            Some(len) if len > 0 => p += len,
            _ => return,
        }
    }
}

fn apply_fusion(bytecode: &mut [u8], p: usize, run: &FusibleRun) {
    match run.layout {
        FusionLayout::Chain { p_next, end } => {
            // on_failure_jump -> p_next  (fail: try the next branch)
            bytecode[p] = RegexOp::OnFailureJump as u8;
            store_number(bytecode, p + 1, (p_next as i64 - (p + 3) as i64) as i16);
            // syntaxspecset <mask>
            bytecode[p + 3] = RegexOp::SyntaxSpecSet as u8;
            store_number_u16(bytecode, p + 4, run.mask);
            // jump -> end  (success: leave the alternation)
            bytecode[p + 6] = RegexOp::Jump as u8;
            store_number(bytecode, p + 7, (end as i64 - (p + 9) as i64) as i16);
            // Pad the remainder of the original region.
            for b in bytecode.iter_mut().take(run.region_end).skip(p + 9) {
                *b = RegexOp::NoOp as u8;
            }
        }
        FusionLayout::FinalBare => {
            // syntaxspecset <mask>, then fall through (padding) to END.
            bytecode[p] = RegexOp::SyntaxSpecSet as u8;
            store_number_u16(bytecode, p + 1, run.mask);
            for b in bytecode.iter_mut().take(run.region_end).skip(p + 3) {
                *b = RegexOp::NoOp as u8;
            }
        }
    }
}
