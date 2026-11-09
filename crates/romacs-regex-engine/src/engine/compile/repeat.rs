use crate::emacs_char;
use crate::engine::compile::RegexCompileError;
use crate::engine::compile::emit::{
    insert_jump, insert_jump2, insert_set_number_at, store_jump_at, store_jump2_at,
};
use crate::engine::opcode::{RegexOp, store_number};
use crate::engine::optimize::simple_one_char_body;
use crate::engine::pattern::CompiledPattern;

/// Split the final character out of a multi-character `exactn` atom.
///
/// GNU `regex-emacs.c` avoids building one `exactn` across a character
/// followed by a postfix or interval operator (see the `normal_char`
/// check for `*`, `+`, `?`, and `\{`). Since this Rust compiler may
/// already have coalesced adjacent literal characters, split lazily
/// before compiling the repeat so `ab\{0,1\}` repeats only `b`, not
/// the whole `ab`.
pub(crate) fn split_trailing_exactn_atom_if_needed(
    buf: &mut CompiledPattern,
    laststart: usize,
) -> usize {
    if buf.buffer.get(laststart).copied() != Some(RegexOp::Exactn as u8) {
        return laststart;
    }

    let count_pos = laststart + 1;
    let Some(&count_byte) = buf.buffer.get(count_pos) else {
        return laststart;
    };
    let count = count_byte as usize;
    let exact_start = count_pos + 1;
    let exact_end = exact_start + count;
    if exact_end > buf.buffer.len() {
        return laststart;
    }
    let exact_bytes = &buf.buffer[exact_start..exact_end];
    let last_char_start = if buf.multibyte {
        let mut rel = 0;
        let mut previous = 0;
        let mut chars = 0;
        while rel < exact_bytes.len() {
            previous = rel;
            let (_, len) = emacs_char::string_char(&exact_bytes[rel..]);
            rel += len;
            chars += 1;
        }
        if chars > 1 { Some(previous) } else { None }
    } else if count > 1 {
        Some(count - 1)
    } else {
        None
    };

    let Some(split_start) = last_char_start else {
        return laststart;
    };

    let split_bytes = exact_bytes[split_start..].to_vec();
    buf.buffer.truncate(exact_start + split_start);
    buf.buffer[count_pos] = split_start as u8;
    let split_atom = buf.buffer.len();
    buf.buffer.push(RegexOp::Exactn as u8);
    buf.buffer.push(split_bytes.len() as u8);
    buf.buffer.extend_from_slice(&split_bytes);
    split_atom
}

/// Compile a repetition operator (*, +, ?).
///
/// Inserts jump opcodes around the preceding expression to implement
/// the repetition. Mirrors GNU's handling in regex_compile cases '*', '+', '?'.
pub(crate) fn compile_repetition(
    op: u8,
    greedy: bool,
    _posix: bool,
    laststart: usize,
    buf: &mut CompiledPattern,
) -> Result<(), RegexCompileError> {
    // All offsets are relative to the position right after the 2-byte offset
    // field.  This matches GNU's convention: after EXTRACT_NUMBER_AND_INCR,
    // `p` points past the offset, and the target is `p + mcnt`.

    let after_last = buf.buffer.len();

    match op {
        b'*' => {
            // * = zero or more
            if greedy {
                // Layout:
                //   [laststart] OFJ*  offset(2)  <expr>  Jump  offset(2)
                //   OFJ* fail target → past the Jump instruction
                //   Jump target → back to the OFJ* opcode
                //
                // Opcode choice mirrors GNU regex-emacs.c:1926-1971:
                // a "simple" one-character body gets on_failure_jump_smart
                // (resolved after compilation to a keep-string fast loop
                // or a plain on_failure_jump by `resolve_smart_jumps`);
                // otherwise on_failure_jump when the body cannot match
                // the empty string, else on_failure_jump_loop (which
                // pays the cycle check).
                let loop_op = if simple_one_char_body(buf, laststart, after_last) {
                    RegexOp::OnFailureJumpSmart
                } else if repeated_body_may_match_empty(&buf.buffer[laststart..after_last]) {
                    RegexOp::OnFailureJumpLoop
                } else {
                    RegexOp::OnFailureJump
                };
                buf.splice_bytecode(laststart, &[loop_op as u8, 0, 0]);
                // After splice, expr occupies [laststart+3 .. laststart+3+expr_len)
                let expr_len = after_last - laststart; // original expr length

                // Add Jump back to the OFJL
                buf.buffer.push(RegexOp::Jump as u8);
                let jpos = buf.buffer.len();
                buf.buffer.push(0);
                buf.buffer.push(0);

                // OFJL fail target: from (laststart+3) → past Jump = (jpos+2)
                // offset = (jpos+2) - (laststart+3) = expr_len + 3
                let ofjl_offset = (expr_len + 3) as i16;
                store_number(&mut buf.buffer, laststart + 1, ofjl_offset);

                // Jump target: from (jpos+2) → OFJL opcode at laststart
                // offset = laststart - (jpos + 2)
                let jump_offset = laststart as i16 - (jpos as i16 + 2);
                store_number(&mut buf.buffer, jpos, jump_offset);
            } else {
                // GNU `regex-emacs.c` compiles non-greedy `*?` as:
                //
                //   jump cond
                // loop:
                //   <expr>
                //   [no-op when expr may match empty]
                // cond:
                //   on_failure_jump[_nastyloop] loop
                //
                // This tries zero iterations first and only falls back
                // into the loop body when a later piece fails.
                let expr_bytes = buf.buffer[laststart..after_last].to_vec();
                let body_may_be_empty = repeated_body_may_match_empty(&expr_bytes);

                buf.buffer.truncate(laststart);

                buf.buffer.push(RegexOp::Jump as u8);
                let jump_pos = buf.buffer.len();
                buf.buffer.push(0);
                buf.buffer.push(0);

                let expr_start = buf.buffer.len();
                buf.buffer.extend_from_slice(&expr_bytes);
                // The body bytes moved from [laststart..after_last) to start at
                // expr_start; re-key any charset side tables to follow them.
                buf.relocate_charset_keys(laststart, after_last, expr_start);
                if body_may_be_empty {
                    buf.buffer.push(RegexOp::NoOp as u8);
                }

                let cond_pos = buf.buffer.len();
                buf.buffer.push(if body_may_be_empty {
                    RegexOp::OnFailureJumpNastyloop as u8
                } else {
                    RegexOp::OnFailureJump as u8
                });
                let cond_arg_pos = buf.buffer.len();
                buf.buffer.push(0);
                buf.buffer.push(0);

                // Initial jump skips directly to the conditional branch.
                let jump_offset = cond_pos as i16 - (jump_pos as i16 + 2);
                store_number(&mut buf.buffer, jump_pos, jump_offset);

                // The conditional branch backtracks into the loop body.
                let cond_offset = expr_start as i16 - (cond_arg_pos as i16 + 2);
                store_number(&mut buf.buffer, cond_arg_pos, cond_offset);
            }
        }
        b'+' => {
            // + = one or more
            // Layout: <expr(already emitted)>  OFJL/OFJ  offset(2)  Jump  offset(2)
            if greedy {
                if simple_one_char_body(buf, laststart, after_last) {
                    // GNU regex-emacs.c:1937-1957: "we turn P+ into PP* if P
                    // is simple", i.e. duplicate the one-char body and compile
                    // the copy as a `*` loop whose head is a smart jump —
                    // `P; OFJS exit; P'; Jump OFJS; exit:` — so
                    // `resolve_smart_jumps` can upgrade the loop to a single
                    // keep-string failure frame for the WHOLE loop (when the
                    // body and continuation are mutually exclusive) instead of
                    // one frame per iteration.  A simple body consumes at
                    // least one char, so the smart opcode is unconditional.
                    let plen = after_last - laststart;
                    buf.buffer.extend_from_within(laststart..after_last);
                    buf.clone_charset_keys(laststart, after_last, after_last);
                    let copy_start = after_last;
                    buf.splice_bytecode(copy_start, &[RegexOp::OnFailureJumpSmart as u8, 0, 0]);
                    buf.buffer.push(RegexOp::Jump as u8);
                    let jpos = buf.buffer.len();
                    buf.buffer.push(0);
                    buf.buffer.push(0);
                    // Smart-jump fail target: from (copy_start+3) → past the
                    // Jump = plen + 3 (same arithmetic as the greedy `*`).
                    store_number(&mut buf.buffer, copy_start + 1, (plen + 3) as i16);
                    // Jump target: back to the smart-jump opcode.
                    let jump_offset = copy_start as i16 - (jpos as i16 + 2);
                    store_number(&mut buf.buffer, jpos, jump_offset);
                    return Ok(());
                }
                // GNU uses on_failure_jump for a body that cannot match
                // empty and on_failure_jump_loop otherwise (the `ofj`
                // choice at regex-emacs.c:1929-1934).  Non-simple bodies
                // skip the PP* rewrite, exactly like GNU: the plain
                // per-iteration on_failure_jump is GNU's own fallback shape.
                let loop_op = if repeated_body_may_match_empty(&buf.buffer[laststart..after_last]) {
                    RegexOp::OnFailureJumpLoop
                } else {
                    RegexOp::OnFailureJump
                };
                // Loop-op fail target → past the Jump instruction (continue)
                buf.buffer.push(loop_op as u8);
                let ofjl_pos = buf.buffer.len();
                buf.buffer.push(0);
                buf.buffer.push(0);

                buf.buffer.push(RegexOp::Jump as u8);
                let jpos = buf.buffer.len();
                buf.buffer.push(0);
                buf.buffer.push(0);

                // OFJL fail: from (ofjl_pos+2) → (jpos+2)
                store_number(&mut buf.buffer, ofjl_pos, (jpos + 2 - ofjl_pos - 2) as i16);

                // Jump target: from (jpos+2) → laststart (start of expr)
                let jump_offset = laststart as i16 - (jpos as i16 + 2);
                store_number(&mut buf.buffer, jpos, jump_offset);
            } else {
                // GNU `regex-emacs.c:1987-1999`: non-greedy `+?`
                // matches one body copy, then uses the same
                // "repeat until fail" conditional jump as `*?`.
                let expr_bytes = buf.buffer[laststart..after_last].to_vec();
                let body_may_be_empty = repeated_body_may_match_empty(&expr_bytes);

                buf.buffer.truncate(laststart);
                let expr_start = buf.buffer.len();
                buf.buffer.extend_from_slice(&expr_bytes);
                // The body moved from [laststart..after_last) to expr_start
                // (here unchanged, but re-key defensively to track the move).
                buf.relocate_charset_keys(laststart, after_last, expr_start);
                if body_may_be_empty {
                    buf.buffer.push(RegexOp::NoOp as u8);
                }

                buf.buffer.push(if body_may_be_empty {
                    RegexOp::OnFailureJumpNastyloop as u8
                } else {
                    RegexOp::OnFailureJump as u8
                });
                let cond_arg_pos = buf.buffer.len();
                buf.buffer.push(0);
                buf.buffer.push(0);

                let cond_offset = expr_start as i16 - (cond_arg_pos as i16 + 2);
                store_number(&mut buf.buffer, cond_arg_pos, cond_offset);
            }
        }
        b'?' => {
            // ? = zero or one
            if greedy {
                // Layout: [laststart] OFJ  offset(2)  <expr>
                // OFJ fail target → past expr
                buf.splice_bytecode(laststart, &[RegexOp::OnFailureJump as u8, 0, 0]);
                let expr_len = after_last - laststart;
                // From (laststart+3) → (laststart+3+expr_len), offset = expr_len
                store_number(&mut buf.buffer, laststart + 1, expr_len as i16);
            } else {
                // Non-greedy `??` (GNU `regex-emacs.c:2009-2015`):
                //
                //   L:    on_failure_jump -> expr_start   ; push body as fallback
                //   L+3:  jump            -> END          ; ...but SKIP it first
                //   L+6:  <expr>
                //   END:
                //
                // The primary path skips the body (zero match = the non-greedy
                // preference); the body is only tried when a later piece fails
                // and matching backtracks to the pushed failure point.  The
                // previous single `OnFailureKeepStringJump` fell through into
                // the body first, i.e. it behaved *greedily* (`a??` matched one
                // char instead of zero).
                //
                // Keep `has_nongreedy_optional` set so the pattern stays on the
                // backtracking matcher (the Pike VM does not model this split).
                buf.has_nongreedy_optional = true;
                let expr_bytes = buf.buffer[laststart..after_last].to_vec();

                buf.buffer.truncate(laststart);

                buf.buffer.push(RegexOp::OnFailureJump as u8);
                let ofj_arg = buf.buffer.len();
                buf.buffer.push(0);
                buf.buffer.push(0);

                buf.buffer.push(RegexOp::Jump as u8);
                let jump_arg = buf.buffer.len();
                buf.buffer.push(0);
                buf.buffer.push(0);

                let expr_start = buf.buffer.len();
                buf.buffer.extend_from_slice(&expr_bytes);
                buf.relocate_charset_keys(laststart, after_last, expr_start);
                let end = buf.buffer.len();

                // on_failure_jump target → body start (the fallback path)
                let ofj_offset = expr_start as i16 - (ofj_arg as i16 + 2);
                store_number(&mut buf.buffer, ofj_arg, ofj_offset);

                // jump target → past the body (the preferred skip path)
                let jump_offset = end as i16 - (jump_arg as i16 + 2);
                store_number(&mut buf.buffer, jump_arg, jump_offset);
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

/// GNU's non-greedy `*?` / `+?` loops use the "repeat until fail"
/// layout from `src/regex-emacs.c:1980-2006`, not the eager
/// `on_failure_keep_string_jump` prefix form used for some greedy
/// optimizations. We only need to know whether the repeated body
/// could match the empty string in order to pick between
/// `on_failure_jump` and `on_failure_jump_nastyloop`.
///
/// A full `analyze_first` port would be ideal, but a conservative
/// first-opcode check is sufficient here: return `false` only for
/// obviously consuming atoms. Everything else falls back to the
/// nastyloop opcode, which is slower but semantics-safe.
fn repeated_body_may_match_empty(body: &[u8]) -> bool {
    let Some(&op) = body.first() else {
        return true;
    };

    !matches!(
        RegexOp::from_byte(op),
        Some(
            RegexOp::Exactn
                | RegexOp::AnyChar
                | RegexOp::Charset
                | RegexOp::CharsetNot
                | RegexOp::SyntaxSpec
                | RegexOp::NotSyntaxSpec
                | RegexOp::SyntaxSpecSet
                | RegexOp::CategorySpec
                | RegexOp::NotCategorySpec
        )
    )
}

/// Parse an interval \{n,m\} from the pattern.
/// Returns (min, max) where max=None means unbounded.
pub(crate) fn parse_interval(
    pattern: &[u8],
    p: &mut usize,
) -> Result<(usize, Option<usize>), RegexCompileError> {
    let plen = pattern.len();

    // Parse min
    let mut min = 0usize;
    while *p < plen && pattern[*p].is_ascii_digit() {
        min = min * 10 + (pattern[*p] - b'0') as usize;
        *p += 1;
    }

    let max = if *p < plen && pattern[*p] == b',' {
        *p += 1; // skip comma
        if *p < plen && pattern[*p] == b'\\' && *p + 1 < plen && pattern[*p + 1] == b'}' {
            // \{n,\} — unbounded
            None
        } else {
            let mut m = 0usize;
            while *p < plen && pattern[*p].is_ascii_digit() {
                m = m * 10 + (pattern[*p] - b'0') as usize;
                *p += 1;
            }
            Some(m)
        }
    } else {
        Some(min) // \{n\} — exact count
    };

    // GNU's GET_UNSIGNED_NUMBER (regex-emacs.c) rejects a repeat count that
    // exceeds RE_DUP_MAX (0xffff) with `(invalid-regexp "Invalid content of
    // \\{\\}")` while reading the number -- not the generic "too big".
    const RE_DUP_MAX: usize = 0xffff;
    if min > RE_DUP_MAX || max.is_some_and(|m| m > RE_DUP_MAX) {
        return Err(RegexCompileError {
            message: "Invalid content of \\{\\}".to_string(),
        });
    }

    // GNU regex-emacs.c:2390 rejects a descending interval where a finite
    // upper bound is smaller than the lower bound (e.g. `a\{2,1\}`), signaling
    // `(invalid-regexp "Invalid content of \\{\\}")`.  An unbounded `\{n,\}`
    // (max == None) is always valid.
    if let Some(m) = max
        && m < min
    {
        return Err(RegexCompileError {
            message: "Invalid content of \\{\\}".to_string(),
        });
    }

    // Expect \}
    if *p + 1 < plen && pattern[*p] == b'\\' && pattern[*p + 1] == b'}' {
        *p += 2;
    } else {
        return Err(RegexCompileError {
            message: "Unmatched \\{".to_string(),
        });
    }

    Ok((min, max))
}

/// Compile an interval \{n,m\} into GNU's counted interval bytecode.
///
/// This mirrors `src/regex-emacs.c`'s interval layout:
///
/// ```text
/// set_number_at <jump_n count> <upper>
/// set_number_at <succeed_n count> <lower>
/// succeed_n     <after jump_n>   <lower>
/// <body>
/// jump_n        <succeed_n>      <upper - 1>
/// ```
///
/// GNU uses `on_failure_jump_loop` instead of `succeed_n` for a zero
/// lower bound and omits the upper-bound `jump_n` when no finite upper
/// bound exists.  Keeping this counted shape matters for large intervals
/// such as CC Mode's `[[:alnum:]]\\{,1000\\}`: expanding the body into
/// hundreds of optional copies creates backtracking behavior GNU avoids.
pub(crate) fn compile_interval(
    min: usize,
    max: Option<usize>,
    laststart: usize,
    buf: &mut CompiledPattern,
) -> Result<(), RegexCompileError> {
    if let Some(max_val) = max {
        if max_val == 0 {
            buf.buffer.truncate(laststart);
            buf.truncate_charset_keys(laststart);
            return Ok(());
        }
        if min == 1 && max_val == 1 {
            return Ok(());
        }
    }

    let old_end = buf.buffer.len();
    let upper_extra_bytes = match max {
        None => 3,
        Some(max_val) if max_val > 1 => 5,
        Some(_) => 0,
    };
    let mut emitted_end = old_end;
    let mut startoffset = 0usize;

    if min == 0 {
        insert_jump(
            buf,
            laststart,
            RegexOp::OnFailureJumpLoop,
            old_end + 3 + upper_extra_bytes,
        )?;
        emitted_end += 3;
    } else {
        insert_jump2(
            buf,
            laststart,
            RegexOp::SucceedN,
            old_end + 5 + upper_extra_bytes,
            min,
        )?;
        emitted_end += 5;
        insert_set_number_at(buf, laststart, 5, min)?;
        emitted_end += 5;
        startoffset += 5;
    }

    match max {
        None => {
            let op_pos = emitted_end;
            buf.buffer.extend_from_slice(&[RegexOp::Jump as u8, 0, 0]);
            store_jump_at(
                &mut buf.buffer,
                op_pos,
                RegexOp::Jump,
                laststart + startoffset,
            )?;
        }
        Some(max_val) if max_val > 1 => {
            let op_pos = emitted_end;
            buf.buffer
                .extend_from_slice(&[RegexOp::JumpN as u8, 0, 0, 0, 0]);
            store_jump2_at(
                &mut buf.buffer,
                op_pos,
                RegexOp::JumpN,
                laststart + startoffset,
                max_val - 1,
            )?;
            emitted_end += 5;
            insert_set_number_at(buf, laststart, emitted_end - laststart, max_val - 1)?;
        }
        Some(_) => {}
    }

    Ok(())
}
