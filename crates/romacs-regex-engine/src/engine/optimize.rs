use crate::emacs_char;
use crate::engine::opcode::{RegexOp, extract_number, opcode_len, store_number};
use crate::engine::pattern::CompiledPattern;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Phase 4: Searcher (re_search_2)
//
// Translates GNU regex-emacs.c:3408-4070.
// Searches for a match in text, using fastmap for optimization.
// ---------------------------------------------------------------------------

/// Analyze compiled bytecode to populate `pattern.fastmap`.
///
/// For each byte value `c` that could possibly appear as the first byte of a
/// match, sets `pattern.fastmap[c] = true`.  The searcher (`re_search`) uses
/// this to skip positions that cannot start a match, giving a significant
/// speed-up for patterns that begin with a restricted set of characters.
///
/// Byte length of the opcode at `pc`, or `None` for a malformed buffer.
/// Conservative upper bound, in characters, on the buffer text a single
/// match attempt of this pattern can consume, or `None` when no finite
/// bound is known (repetition, backreferences, repeat counters,
/// range-table charsets, or a bound past the cap). Every jump in a
/// loop-free program points forward, so the program is a left-to-right
/// DAG and the sum of every consuming opcode bounds any path through it.
/// `exactn` counts bytes, which only overestimates its character count.
pub fn pattern_max_match_chars(pattern: &CompiledPattern) -> Option<usize> {
    const CAP: usize = 512;
    let bytecode = &pattern.buffer;
    let mut pc = 0usize;
    let mut total = 0usize;
    while pc < bytecode.len() {
        let op = RegexOp::from_byte(*bytecode.get(pc)?)?;
        match op {
            RegexOp::Exactn => total += *bytecode.get(pc + 1)? as usize,
            RegexOp::AnyChar
            | RegexOp::SyntaxSpec
            | RegexOp::NotSyntaxSpec
            | RegexOp::CategorySpec
            | RegexOp::NotCategorySpec
            | RegexOp::SyntaxSpecSet => total += 1,
            RegexOp::Charset | RegexOp::CharsetNot => {
                // The high bit marks a trailing range table that
                // `opcode_len` does not skip; walking past one would
                // misread its bytes as opcodes.
                if bytecode.get(pc + 1)? & 0x80 != 0 {
                    return None;
                }
                total += 1;
            }
            RegexOp::Duplicate | RegexOp::SucceedN | RegexOp::JumpN | RegexOp::SetNumberAt => {
                return None;
            }
            RegexOp::Jump
            | RegexOp::OnFailureJump
            | RegexOp::OnFailureKeepStringJump
            | RegexOp::OnFailureJumpLoop
            | RegexOp::OnFailureJumpNastyloop
            | RegexOp::OnFailureJumpSmart
                if extract_number(bytecode, pc + 1) < 0 =>
            {
                return None;
            }
            _ => {}
        }
        if total > CAP {
            return None;
        }
        match opcode_len(bytecode, pc) {
            Some(len) if len > 0 => pc += len,
            _ => return None,
        }
    }
    Some(total)
}

/// Is `[start, end)` exactly one character-matching opcode?  GNU
/// `skip_one_char (laststart) == b` (regex-emacs.c:1928): the compiler
/// splits trailing multi-char `exactn`s before postfix operators, so a
/// simple body is a single `anychar` / one-char `exactn` / `charset` /
/// `syntaxspec` / `categoryspec`.
pub(crate) fn simple_one_char_body(buf: &CompiledPattern, start: usize, end: usize) -> bool {
    let bytecode = &buf.buffer;
    let Some(op) = bytecode.get(start).copied().and_then(RegexOp::from_byte) else {
        return false;
    };
    if !matches!(
        op,
        RegexOp::AnyChar
            | RegexOp::Exactn
            | RegexOp::Charset
            | RegexOp::CharsetNot
            | RegexOp::SyntaxSpec
            | RegexOp::NotSyntaxSpec
            | RegexOp::SyntaxSpecSet
            | RegexOp::CategorySpec
            | RegexOp::NotCategorySpec
    ) {
        return false;
    }
    if op == RegexOp::Exactn {
        // One character only (multibyte chars span several bytes).
        let count = match bytecode.get(start + 1) {
            Some(&count) => count as usize,
            None => return false,
        };
        let char_len = if buf.multibyte && count > 0 && start + 2 + count <= bytecode.len() {
            emacs_char::string_char(&bytecode[start + 2..start + 2 + count]).1
        } else {
            1
        };
        if count != char_len {
            return false;
        }
    }
    opcode_len(bytecode, start) == Some(end - start)
}

/// Conservative superset of the characters a single char-matching opcode
/// can accept: an ASCII bitmask plus a "may match something >= 0x80 /
/// multibyte" flag.  `None` = unanalyzable (syntax/category/class-bit
/// dependent — GNU resolves those against the live syntax table at its
/// runtime rewrite; our rewrite happens at compile time, so anything
/// table-dependent must stay unanalyzable to keep bytecode
/// table-independent).
#[derive(Clone, Copy, Default)]
struct CharSuperset {
    ascii: u128,
    high: bool,
}

impl CharSuperset {
    fn add_char(&mut self, ch: u32) {
        if ch < 0x80 {
            self.ascii |= 1u128 << ch;
        } else {
            self.high = true;
        }
    }

    fn union(&mut self, other: CharSuperset) {
        self.ascii |= other.ascii;
        self.high |= other.high;
    }

    fn disjoint(&self, other: &CharSuperset) -> bool {
        (self.ascii & other.ascii) == 0 && !(self.high && other.high)
    }
}

/// Match superset of the one char-matching opcode at `pos`.
fn one_char_match_superset(buf: &CompiledPattern, pos: usize) -> Option<CharSuperset> {
    let bytecode = &buf.buffer;
    let op = RegexOp::from_byte(*bytecode.get(pos)?)?;
    let mut set = CharSuperset::default();
    match op {
        RegexOp::Exactn => {
            let count = *bytecode.get(pos + 1)? as usize;
            if count == 0 || pos + 2 + count > bytecode.len() {
                return None;
            }
            // Pattern literals are stored translated (case-canonical);
            // the same holds for the sets compared against, so both
            // sides of the exclusivity test are in canonical space
            // (GNU compares RE_STRING_CHAR of the stored bytes too).
            if buf.multibyte {
                let (ch, _) = emacs_char::string_char(&bytecode[pos + 2..pos + 2 + count]);
                set.add_char(ch);
            } else {
                set.add_char(bytecode[pos + 2] as u32);
            }
        }
        RegexOp::AnyChar => {
            // Everything except newline (u128 bits 0..=127 = ASCII).
            set.ascii = !(1u128 << b'\n');
            set.high = true;
        }
        RegexOp::Charset => {
            let charset_pos = pos;
            let bitmap_len = (*bytecode.get(pos + 1)? & 0x7F) as usize;
            if pos + 2 + bitmap_len > bytecode.len() {
                return None;
            }
            // POSIX-class bits can match table-dependent sets; give up.
            if buf.charset_class_bits.contains_key(&charset_pos) {
                return None;
            }
            for c in 0..(bitmap_len * 8).min(256) {
                if (bytecode[pos + 2 + c / 8] >> (c % 8)) & 1 != 0 {
                    set.add_char(c as u32);
                }
            }
            if let Some(ranges) = buf.multibyte_charsets.get(&charset_pos) {
                for &(lo, hi) in ranges {
                    if (lo as u32) < 0x80 {
                        for c in (lo as u32)..=(hi as u32).min(0x7F) {
                            set.add_char(c);
                        }
                    }
                    if (hi as u32) >= 0x80 {
                        set.high = true;
                    }
                }
            }
        }
        RegexOp::CharsetNot => {
            let _charset_pos = pos;
            let bitmap_len = (*bytecode.get(pos + 1)? & 0x7F) as usize;
            if pos + 2 + bitmap_len > bytecode.len() {
                return None;
            }
            // Superset of "not in set": the bitmap complement plus all
            // multibyte characters (class bits and ranges only REMOVE
            // characters from a negated set, so ignoring them keeps
            // this a superset).
            for c in 0..128usize {
                let in_bitmap =
                    c / 8 < bitmap_len && (bytecode[pos + 2 + c / 8] >> (c % 8)) & 1 != 0;
                if !in_bitmap {
                    set.add_char(c as u32);
                }
            }
            set.high = true;
        }
        // Syntax and category tests depend on runtime tables.
        _ => return None,
    }
    Some(set)
}

/// Superset of the first characters matchable by the continuation
/// starting at `from` — a worklist walk in the style of GNU
/// `forall_firstchar` as used by `mutually_exclusive_p`
/// (regex-emacs.c:4025-4035).  Returns `None` when any reachable path is
/// unanalyzable or may accept the empty string (reaching `succeed` /
/// end-of-pattern), in which case the loop must stay backtrack-safe.
fn continuation_first_superset(buf: &CompiledPattern, from: usize) -> Option<CharSuperset> {
    let bytecode = &buf.buffer;
    let mut set = CharSuperset::default();
    let mut worklist: Vec<usize> = vec![from];
    let mut visited: HashSet<usize> = HashSet::new();

    while let Some(start_pc) = worklist.pop() {
        let mut pc = start_pc;
        loop {
            if !visited.insert(pc) {
                break;
            }
            if pc >= bytecode.len() {
                // Pattern may end right after the loop: a shorter
                // iteration count could produce an overall match, so the
                // fast loop is not safe here.  (GNU's `unconstrained`
                // refinement is applied separately, and only to an
                // immediately terminal `Succeed`: see
                // `unconstrained_terminal_success`.)
                return None;
            }
            let op = RegexOp::from_byte(bytecode[pc])?;
            match op {
                RegexOp::Succeed | RegexOp::PosixEnd => return None,

                // Char-matching terminals: contribute their superset.
                RegexOp::Exactn | RegexOp::AnyChar | RegexOp::Charset | RegexOp::CharsetNot => {
                    set.union(one_char_match_superset(buf, pc)?);
                    break;
                }

                // Table-dependent matchers: unanalyzable.
                RegexOp::SyntaxSpec
                | RegexOp::NotSyntaxSpec
                | RegexOp::SyntaxSpecSet
                | RegexOp::CategorySpec
                | RegexOp::NotCategorySpec
                | RegexOp::Duplicate => return None,

                // endline is effectively `exactn \n` for this analysis
                // (GNU mutually_exclusive_one, `case endline`).
                RegexOp::EndLine => {
                    set.add_char(b'\n' as u32);
                    break;
                }

                // If the body matched a character we are not at
                // end-of-buffer, so `endbuf` fails: exclusive on this
                // path, nothing to add (GNU `case endbuf: return true`).
                RegexOp::EndBuf => break,

                // Other zero-width assertions cannot rescue a shorter
                // iteration count on their own (the character test
                // already fails there); walk through them.
                RegexOp::BegLine
                | RegexOp::BegBuf
                | RegexOp::AtDot
                | RegexOp::WordBound
                | RegexOp::NotWordBound
                | RegexOp::WordBeg
                | RegexOp::WordEnd
                | RegexOp::SymBeg
                | RegexOp::SymEnd
                | RegexOp::NoOp => {
                    pc += 1;
                }

                RegexOp::StartMemory | RegexOp::StopMemory => {
                    pc += 2;
                }

                RegexOp::Jump => {
                    let offset = extract_number(bytecode, pc + 1);
                    pc = ((pc as i64) + 3 + (offset as i64)) as usize;
                }

                RegexOp::OnFailureJump
                | RegexOp::OnFailureKeepStringJump
                | RegexOp::OnFailureJumpLoop
                | RegexOp::OnFailureJumpNastyloop
                | RegexOp::OnFailureJumpSmart => {
                    let offset = extract_number(bytecode, pc + 1);
                    worklist.push(((pc as i64) + 3 + (offset as i64)) as usize);
                    pc += 3;
                }

                RegexOp::SucceedN | RegexOp::JumpN => {
                    let offset = extract_number(bytecode, pc + 1);
                    worklist.push(((pc as i64) + 3 + (offset as i64)) as usize);
                    pc += 5;
                }

                RegexOp::SetNumberAt => {
                    pc += 5;
                }
            }
        }
    }
    Some(set)
}

/// GNU `mutually_exclusive_p` (regex-emacs.c:4025): true if "the loop
/// body at `body_start` matches something" implies "the continuation at
/// `cont` fails".  Conservative: only provable byte-set disjointness
/// counts, so a `false` merely keeps the safe backtracking loop.
fn mutually_exclusive_p(buf: &CompiledPattern, body_start: usize, cont: usize) -> bool {
    let Some(body) = one_char_match_superset(buf, body_start) else {
        return false;
    };
    let Some(cont_set) = continuation_first_superset(buf, cont) else {
        return false;
    };
    body.disjoint(&cont_set)
}

/// GNU `mutually_exclusive_one`'s `case succeed` (regex-emacs.c:3938-3951),
/// in its narrowest form: the loop exit at `cont` reaches the pattern's
/// final non-POSIX `Succeed` with nothing but `NoOp`s on the way.
///
/// GNU's argument: when N and N+1 iterations of the one-char body both
/// match, the `succeed` after N+1 always wins (the greedy path reaches it
/// first and `succeed` ends the whole match), so the `succeed` after N
/// may be treated as a failure and the loop needs no per-iteration
/// failure point.  It holds only while the path from the loop exit to
/// `succeed` is *unconstrained* — no operator that could reject the
/// longer run.  GNU tracks that with `data->unconstrained`; here only the
/// trivially unconstrained path qualifies.  Assertions, captures, jumps,
/// alternation, `PosixEnd` and every other continuation keep the
/// existing analysis (and, failing it, the backtracking loop).
///
/// The body must be the verified one-character shape
/// ([`simple_one_char_body`]) and analyzable by
/// [`one_char_match_superset`]: a single opcode that either matches one
/// character or fails without consuming, so the keep-string exit resumes
/// at the same string position the per-iteration frame would have
/// restored.  Matching predicates, case translation, ranges, stops,
/// registers and quit polling are untouched — only the frame protocol of
/// this one loop changes, exactly as in GNU's rewrite.
fn unconstrained_terminal_success(
    buf: &CompiledPattern,
    body_start: usize,
    body_end: usize,
    cont: usize,
) -> bool {
    if buf.posix
        || !simple_one_char_body(buf, body_start, body_end)
        || one_char_match_superset(buf, body_start).is_none()
    {
        return false;
    }
    let bytecode = &buf.buffer;
    let mut pc = cont;
    while bytecode.get(pc) == Some(&(RegexOp::NoOp as u8)) {
        pc += 1;
    }
    // The compiler emits exactly one `Succeed`, as the last byte of a
    // non-POSIX pattern (see `regex_compile_lisp_with_translation`).
    pc + 1 == bytecode.len() && bytecode[pc] == RegexOp::Succeed as u8
}

/// Resolve every `on_failure_jump_smart` in freshly-compiled bytecode.
///
/// GNU performs this rewrite lazily on first execution
/// (regex-emacs.c:4864-4906, discarding `const` and making `re_match`
/// non-reentrant); romacs shares compiled patterns via a cache, so the
/// rewrite happens once here, immediately after compilation — same
/// resulting bytecode, no runtime mutation:
///
/// - exclusive loop (`mutually_exclusive_p`, or GNU's `unconstrained`
///   terminal-`succeed` case, `unconstrained_terminal_success`): the opcode
///   becomes
///   `on_failure_keep_string_jump` and the trailing jump is retargeted
///   from the opcode to the loop body (GNU `STORE_NUMBER (p2 - 2,
///   mcnt + 3)`), so ONE failure point is pushed for the whole loop and
///   iterations do not save/restore the string position;
/// - otherwise it becomes a plain `on_failure_jump`.
pub(crate) fn resolve_smart_jumps(buf: &mut CompiledPattern) {
    let mut pc = 0;
    let mut prev_start: Option<usize> = None;
    while pc < buf.buffer.len() {
        let Some(len) = opcode_len(&buf.buffer, pc) else {
            return;
        };
        if buf.buffer[pc] == RegexOp::OnFailureJumpSmart as u8 {
            resolve_one_smart_jump(buf, pc, prev_start);
        }
        prev_start = Some(pc);
        pc += len;
    }
}

fn resolve_one_smart_jump(buf: &mut CompiledPattern, p3: usize, prev_start: Option<usize>) {
    // Expected shape (emitted by `compile_repetition`):
    //   p3: on_failure_jump_smart <offset -> p2>
    //   p3+3: <one-char body>
    //   p2-3: jump <offset -> p3>
    //   p2: continuation
    // Default to the safe plain loop; upgrade if the shape checks out
    // and the body excludes the continuation.
    buf.buffer[p3] = RegexOp::OnFailureJump as u8;
    if p3 + 3 > buf.buffer.len() {
        return;
    }
    let offset = extract_number(&buf.buffer, p3 + 1);
    let p2 = ((p3 + 3) as i64 + offset as i64) as usize;
    if p2 < p3 + 6 || p2 > buf.buffer.len() {
        return;
    }
    let jump_at = p2 - 3;
    if buf.buffer[jump_at] != RegexOp::Jump as u8 {
        return;
    }
    let back = extract_number(&buf.buffer, jump_at + 1);
    if ((jump_at + 3) as i64 + back as i64) as usize != p3 {
        return;
    }
    if mutually_exclusive_p(buf, p3 + 3, p2)
        || unconstrained_terminal_success(buf, p3 + 3, jump_at, p2)
    {
        buf.buffer[p3] = RegexOp::OnFailureKeepStringJump as u8;
        store_number(&mut buf.buffer, jump_at + 1, back + 3);
        return;
    }
    // Non-exclusive loop: keep-string is off the table, so a PP*-duplicated
    // body only costs an extra dispatch and loop-head execution per loop
    // entry.  When the op right before the loop is a verified identical copy
    // of the body (`P; OFJ exit; P'; Jump OFJ` — PP* by construction, or a
    // literal `PP*` which is semantically the same `P+`), demote it in place
    // to the body-first plus shape the matcher fuses at per-iteration-frame
    // cost with no loop-entry overhead:
    //   P; OFJ exit; Jump P; <dead NoOps>
    // The dead NoOps keep the buffer length and every other offset stable;
    // the OFJ keeps its exit target.  `prev_start` comes from the resolver's
    // linear walk, so `P` is a real opcode boundary.
    let body_start = p3 + 3;
    let plen = jump_at - body_start;
    let Some(first_p) = prev_start else {
        return;
    };
    if first_p + plen != p3
        || buf.buffer[first_p..p3] != buf.buffer[body_start..jump_at]
        || buf.multibyte_charsets.get(&first_p) != buf.multibyte_charsets.get(&body_start)
        || buf.charset_class_bits.get(&first_p) != buf.charset_class_bits.get(&body_start)
    {
        return;
    }
    buf.buffer[body_start] = RegexOp::Jump as u8;
    let back_to_first = first_p as i64 - (body_start as i64 + 3);
    store_number(&mut buf.buffer, body_start + 1, back_to_first as i16);
    for byte in &mut buf.buffer[body_start + 3..p2] {
        *byte = RegexOp::NoOp as u8;
    }
    // The demoted copy's charset side entries are dead with it.
    buf.multibyte_charsets.remove(&body_start);
    buf.charset_class_bits.remove(&body_start);
}
