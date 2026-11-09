//! The compiler: a pattern string to bytecode.
//!
//! GNU: `regex_compile` in regex-emacs.c, with its subroutines
//! `at_begline_loc_p` and `at_endline_loc_p`.  Bracket expressions,
//! repetition and buffer editing live in the submodules.

pub(super) mod charset;
pub(super) mod emit;
pub(super) mod repeat;
pub(super) mod seal;

use crate::case_table::EmacsText;
use crate::engine::class::{regex_char_to_unibyte, regex_unibyte_to_char};
use crate::engine::compile::charset::compile_charset;
use crate::engine::compile::repeat::{
    compile_interval, compile_repetition, parse_interval, split_trailing_exactn_atom_if_needed,
};
use crate::engine::compile::seal::{
    build_rewind_view, compute_pike_eligible, validate_sealed_buffer,
};
use crate::engine::fastmap::compile_fastmap;
use crate::engine::fusion::fuse_syntaxspec_alternations;
use crate::engine::linear;
use crate::engine::opcode::{RegexOp, store_number};
use crate::engine::optimize::resolve_smart_jumps;
use crate::engine::pattern::{CompiledPattern, RewindView};
use crate::engine::search::{anchor_alt_enabled, start_anchor};
use crate::engine::syntax::DefaultSyntaxLookup;
use crate::engine::translate::CaseTranslation;
use crate::syntax_class::SyntaxClass;
use crate::{case_table, emacs_char};

/// Error from regex compilation.
#[derive(Debug, Clone)]
pub struct RegexCompileError {
    pub message: String,
}

impl std::fmt::Display for RegexCompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Compile stack entry — tracks open groups during compilation.
/// Mirrors GNU's compile_stack_elt_t.
#[derive(Clone, Debug)]
struct CompileStackEntry {
    /// Bytecode position of the start of the group's alternatives.
    begalt_offset: usize,
    /// Bytecode position of the fixup jump for alternation (or 0).
    fixup_alt_jump: Option<usize>,
    /// The actual group number assigned to this \( (None for shy groups).
    assigned_group: Option<usize>,
    /// Bytecode position of the group's StartMemory (or OnFailureJump
    /// for shy groups). Used by postfix ops like ? * + after \).
    group_bytecode_start: usize,
}

/// Compile an Emacs regex pattern into bytecode.
///
/// This is the main entry point, equivalent to GNU's `regex_compile()`.
///
/// # Arguments
/// * `pattern` - The Emacs regex pattern string
/// * `posix` - If true, use POSIX backtracking semantics
/// * `case_fold` - If true, compile for case-insensitive matching
///
/// # Returns
/// A `CompiledPattern` with bytecode ready for the matcher.
pub fn regex_compile(
    pattern: &str,
    posix: bool,
    case_fold: bool,
) -> Result<CompiledPattern, RegexCompileError> {
    // A Rust string is multibyte text, as `LispString::from_utf8` made it.
    regex_compile_lisp(
        &case_table::EmacsStr::multibyte(pattern.as_bytes()),
        posix,
        case_fold,
    )
}

/// Compile a pattern given as Lisp-string text: internal-representation
/// bytes plus their multibyteness ([`EmacsText`]).
pub fn regex_compile_lisp(
    pattern: &(impl EmacsText + ?Sized),
    posix: bool,
    case_fold: bool,
) -> Result<CompiledPattern, RegexCompileError> {
    let translation = case_fold.then(CaseTranslation::standard);
    regex_compile_lisp_with_translation(pattern, posix, translation)
}

pub fn regex_compile_lisp_with_translation(
    pattern: &(impl EmacsText + ?Sized),
    posix: bool,
    translation: Option<CaseTranslation>,
) -> Result<CompiledPattern, RegexCompileError> {
    let mut buf = CompiledPattern::new();
    buf.posix = posix;
    buf.multibyte = pattern.is_multibyte();
    buf.target_multibyte = pattern.is_multibyte();
    buf.translate = translation;
    let _case_fold = buf.translate.is_some();

    let pattern_bytes = pattern.emacs_bytes();
    let plen = pattern_bytes.len();
    let mut p = 0; // Current position in pattern

    // Compile stack for tracking open groups
    let mut compile_stack: Vec<CompileStackEntry> = Vec::new();

    // Track positions in bytecode for fixup
    let mut begalt_offset: usize = 0; // Start of current alternative
    let mut pending_exact: Option<usize> = None; // Position of current exactn being built
    let mut laststart: Option<usize> = None; // Start of last complete expression (for postfix ops)
    let mut laststart_is_group = false; // True when laststart came from a closed \( ... \).
    let mut fixup_alt_jump: Option<usize> = None; // Jump to fixup at end of alternation

    /// Helper: push a byte to the bytecode buffer
    macro_rules! emit {
        ($byte:expr) => {
            buf.buffer.push($byte);
        };
    }

    /// Helper: push an opcode
    macro_rules! emit_op {
        ($op:expr) => {
            buf.buffer.push($op as u8);
        };
    }

    /// Helper: current bytecode position
    macro_rules! bpos {
        () => {
            buf.buffer.len()
        };
    }

    // Macro to fetch next pattern byte, returning error if at end
    #[allow(unused_macros)]
    macro_rules! pat_fetch {
        () => {{
            if p >= plen {
                return Err(RegexCompileError {
                    message: "premature end of pattern".to_string(),
                });
            }
            let c = pattern_bytes[p];
            p += 1;
            c
        }};
    }

    // Main compilation loop
    while p < plen {
        let c = pattern_bytes[p];
        p += 1;

        match c {
            // ----------------------------------------------------------
            // ^ — beginning of line
            // ----------------------------------------------------------
            b'^' => {
                if !(p == 1 || at_begline_loc_p(pattern_bytes, p)) {
                    goto_normal_char(
                        c as u32,
                        &mut buf,
                        &mut pending_exact,
                        &mut laststart,
                        &mut laststart_is_group,
                    );
                    continue;
                }
                laststart = None;
                laststart_is_group = false;
                pending_exact = None;
                emit_op!(RegexOp::BegLine);
            }

            // ----------------------------------------------------------
            // $ — end of line
            // ----------------------------------------------------------
            b'$' => {
                if !(p == plen || at_endline_loc_p(pattern_bytes, p)) {
                    goto_normal_char(
                        c as u32,
                        &mut buf,
                        &mut pending_exact,
                        &mut laststart,
                        &mut laststart_is_group,
                    );
                    continue;
                }
                laststart = Some(bpos!());
                laststart_is_group = false;
                pending_exact = None;
                emit_op!(RegexOp::EndLine);
            }

            // ----------------------------------------------------------
            // . — any character
            // ----------------------------------------------------------
            b'.' => {
                laststart = Some(bpos!());
                pending_exact = None;
                emit_op!(RegexOp::AnyChar);
            }

            // ----------------------------------------------------------
            // * + ? — repetition operators
            // ----------------------------------------------------------
            b'*' | b'+' | b'?' => {
                let Some(mut last) = laststart else {
                    // No previous expression to repeat — treat as literal
                    goto_normal_char(
                        c as u32,
                        &mut buf,
                        &mut pending_exact,
                        &mut laststart,
                        &mut laststart_is_group,
                    );
                    continue;
                };
                let last_is_group = laststart_is_group;

                // GNU regex_compile: if the preceding expression was part
                // of an exactn with count > 1, split off the last character
                // so that the repetition applies only to that character.
                //
                // Do not split when the postfix applies to a just-closed shy
                // group. GNU clears `pending_exact` on \), so `\(?:ab\)?`
                // repeats the whole "ab" group, not only "b".
                if !last_is_group {
                    last = split_trailing_exactn_atom_if_needed(&mut buf, last);
                }

                // GNU regex-emacs.c: if there is a sequence of repetition
                // chars, collapse it down to just one (the right one).  We
                // track zero_times_ok / many_times_ok / greedy exactly as GNU
                // does so that stacked quantifiers like `a**`, `a*?*`, `a++`,
                // `a???` fold onto the preceding atom instead of being treated
                // as literals.  (Interval operators `\{n,m\}` are NOT folded
                // here, matching GNU.)
                let mut cur = c;
                let mut zero_times_ok = false;
                let mut many_times_ok = false;
                let mut greedy = true;
                loop {
                    if cur == b'?' && (zero_times_ok || many_times_ok) {
                        greedy = false;
                    } else {
                        zero_times_ok |= cur != b'+';
                        many_times_ok |= cur != b'?';
                    }

                    if !(p < plen
                        && (pattern_bytes[p] == b'*'
                            || pattern_bytes[p] == b'+'
                            || pattern_bytes[p] == b'?'))
                    {
                        break;
                    }
                    // Found another repeat character — consume and fold it.
                    cur = pattern_bytes[p];
                    p += 1;
                }

                // Map the collapsed flags back to a single effective postfix
                // operator for `compile_repetition`:
                //   (zero, many) = (T,T) -> `*`, (T,F) -> `?`, (F,T) -> `+`.
                let folded_op = match (zero_times_ok, many_times_ok) {
                    (true, true) => b'*',
                    (true, false) => b'?',
                    (false, true) => b'+',
                    // Unreachable: the first iteration always sets at least one
                    // flag, but fall back to a plain match if it somehow isn't.
                    (false, false) => b'+',
                };

                compile_repetition(folded_op, greedy, posix, last, &mut buf)?;

                laststart = None; // Can't apply another postfix op
                laststart_is_group = false;
                pending_exact = None;
            }

            // ----------------------------------------------------------
            // [ — character class
            // ----------------------------------------------------------
            b'[' => {
                laststart = Some(bpos!());
                pending_exact = None;
                let pattern_multibyte = buf.multibyte;
                compile_charset(pattern_bytes, &mut p, &mut buf, pattern_multibyte)?;
            }

            // ----------------------------------------------------------
            // \ — escape sequence
            // ----------------------------------------------------------
            b'\\' => {
                if p >= plen {
                    return Err(RegexCompileError {
                        message: "Trailing backslash".to_string(),
                    });
                }
                let c2 = pattern_bytes[p];
                p += 1;

                match c2 {
                    // \( — start group
                    b'(' => {
                        let mut is_shy = false;
                        let mut explicit_group: Option<usize> = None;
                        if p < plen && pattern_bytes[p] == b'?' {
                            p += 1; // skip ?
                            if p < plen && pattern_bytes[p] == b':' {
                                is_shy = true;
                                p += 1; // skip :
                            } else {
                                let num_start = p;
                                let mut n = 0usize;
                                while p < plen && pattern_bytes[p].is_ascii_digit() {
                                    if p == num_start && pattern_bytes[p] == b'0' {
                                        return Err(RegexCompileError {
                                            message: "Invalid regular expression".to_string(),
                                        });
                                    }
                                    n = n
                                        .checked_mul(10)
                                        .and_then(|value| {
                                            value.checked_add((pattern_bytes[p] - b'0') as usize)
                                        })
                                        .ok_or_else(|| RegexCompileError {
                                            message: "Regular expression too big".to_string(),
                                        })?;
                                    p += 1;
                                }
                                if p == num_start || p >= plen || pattern_bytes[p] != b':' {
                                    return Err(RegexCompileError {
                                        message: "Invalid regular expression".to_string(),
                                    });
                                }
                                explicit_group = Some(n);
                                p += 1; // skip :
                            }
                        }

                        let group_start = bpos!();
                        let assigned = if let Some(n) = explicit_group {
                            Some(n)
                        } else if !is_shy {
                            // GNU `regex-emacs.c:2246`: an auto-numbered group is
                            // `++bufp->re_nsub`, one past the highest group number
                            // seen so far, even after an explicit `\(?N:...\)`
                            // numbered a group below it (`\(?2:\(?1:\)\)\(\)`).
                            Some(buf.re_nsub + 1)
                        } else {
                            None
                        };

                        // GNU `regex-emacs.c:2250-2259`: an explicitly-numbered
                        // group `\(?N:...\)` whose number collides with a group
                        // that is still OPEN on the compile stack (an enclosing
                        // `\(...\)` that already holds N) is `REG_BADPAT`.  Note
                        // `compile_stack` here holds only the enclosing groups —
                        // the current group has not been pushed yet.  Reusing a
                        // number for a sequential/closed group stays legal (so
                        // `\(?1:\)\(?1:\)` and `\(\)\(?1:\)` remain accepted).
                        if let Some(n) = explicit_group
                            && n <= buf.re_nsub
                            && compile_stack.iter().any(|e| e.assigned_group == Some(n))
                        {
                            return Err(RegexCompileError {
                                message: "Invalid regular expression".to_string(),
                            });
                        }

                        compile_stack.push(CompileStackEntry {
                            begalt_offset,
                            fixup_alt_jump,
                            assigned_group: assigned,
                            group_bytecode_start: group_start,
                        });

                        if let Some(n) = explicit_group {
                            // Explicit numbered group: assign group number n
                            while buf.re_nsub < n {
                                buf.re_nsub += 1;
                            }
                            emit_op!(RegexOp::StartMemory);
                            emit!(n as u8);
                        } else if !is_shy {
                            // Auto group: number = ++re_nsub (GNU
                            // regex-emacs.c:2246), kept in sync with `assigned`.
                            buf.re_nsub += 1;
                            emit_op!(RegexOp::StartMemory);
                            emit!(buf.re_nsub as u8);
                        }

                        begalt_offset = bpos!();
                        laststart = None;
                        fixup_alt_jump = None;
                        pending_exact = None;
                    }

                    // \) — end group
                    b')' => {
                        let Some(entry) = compile_stack.pop() else {
                            return Err(RegexCompileError {
                                message: "Unmatched ) or \\)".to_string(),
                            });
                        };

                        // Handle pending alternation fixup
                        if let Some(fixup) = fixup_alt_jump {
                            let target = bpos!() as i16 - fixup as i16 - 2;
                            store_number(&mut buf.buffer, fixup, target);
                        }

                        // Emit StopMemory for non-shy groups.
                        if let Some(group_num) = entry.assigned_group {
                            emit_op!(RegexOp::StopMemory);
                            emit!(group_num as u8);
                        }

                        begalt_offset = entry.begalt_offset;
                        fixup_alt_jump = entry.fixup_alt_jump;
                        // After \), laststart points to the group's start
                        // so postfix operators (?, *, +) apply to the group.
                        laststart = Some(entry.group_bytecode_start);
                        laststart_is_group = true;
                        pending_exact = None;
                    }

                    // \| — alternation
                    b'|' => {
                        pending_exact = None;

                        // Emit jump past the next alternative
                        emit_op!(RegexOp::Jump);
                        let jump_pos = bpos!();
                        emit!(0);
                        emit!(0); // placeholder offset

                        // Fixup previous alternative's failure jump
                        if let Some(fixup) = fixup_alt_jump {
                            let target = bpos!() as i16 - fixup as i16 - 2;
                            store_number(&mut buf.buffer, fixup, target);
                        }

                        // Insert on_failure_jump at the start of the current alt
                        let alt_start = begalt_offset;
                        // We need to insert 3 bytes at alt_start
                        buf.splice_bytecode(alt_start, &[RegexOp::OnFailureJump as u8, 0, 0]);
                        // The failure jump target is right after the jump we just emitted
                        let target = (bpos!() - alt_start - 3) as i16;
                        store_number(&mut buf.buffer, alt_start + 1, target);

                        // Adjust jump_pos since we inserted 3 bytes
                        fixup_alt_jump = Some(jump_pos + 3);

                        begalt_offset = bpos!();
                        laststart = None;
                    }

                    // \` — beginning of buffer
                    b'`' => {
                        // GNU regex-emacs.c deliberately makes postfix operators
                        // following \` literal, matching its treatment of ^.
                        laststart = None;
                        laststart_is_group = false;
                        pending_exact = None;
                        emit_op!(RegexOp::BegBuf);
                    }

                    // \' — end of buffer
                    b'\'' => {
                        laststart = Some(bpos!());
                        pending_exact = None;
                        emit_op!(RegexOp::EndBuf);
                    }

                    // \= — at point
                    b'=' => {
                        laststart = Some(bpos!());
                        pending_exact = None;
                        emit_op!(RegexOp::AtDot);
                    }

                    // \b — word boundary
                    b'b' => {
                        laststart = Some(bpos!());
                        pending_exact = None;
                        buf.uses_syntax = true;
                        emit_op!(RegexOp::WordBound);
                    }

                    // \B — not word boundary
                    b'B' => {
                        laststart = Some(bpos!());
                        pending_exact = None;
                        buf.uses_syntax = true;
                        emit_op!(RegexOp::NotWordBound);
                    }

                    // \< — word beginning
                    b'<' => {
                        laststart = Some(bpos!());
                        pending_exact = None;
                        buf.uses_syntax = true;
                        emit_op!(RegexOp::WordBeg);
                    }

                    // \> — word end
                    b'>' => {
                        laststart = Some(bpos!());
                        pending_exact = None;
                        buf.uses_syntax = true;
                        emit_op!(RegexOp::WordEnd);
                    }

                    // \_ — symbol boundary
                    b'_' => {
                        if p >= plen {
                            return Err(RegexCompileError {
                                message: "Premature end of regular expression".to_string(),
                            });
                        }
                        let c3 = pattern_bytes[p];
                        p += 1;
                        match c3 {
                            b'<' => {
                                laststart = Some(bpos!());
                                pending_exact = None;
                                buf.uses_syntax = true;
                                emit_op!(RegexOp::SymBeg);
                            }
                            b'>' => {
                                laststart = Some(bpos!());
                                pending_exact = None;
                                buf.uses_syntax = true;
                                emit_op!(RegexOp::SymEnd);
                            }
                            _ => {
                                return Err(RegexCompileError {
                                    message: "Invalid regular expression".to_string(),
                                });
                            }
                        }
                    }

                    // \w — word constituent (syntax-table aware)
                    b'w' => {
                        laststart = Some(bpos!());
                        pending_exact = None;
                        buf.uses_syntax = true;
                        emit_op!(RegexOp::SyntaxSpec);
                        emit!(u8::from(SyntaxClass::Word));
                    }

                    // \W — not word constituent
                    b'W' => {
                        laststart = Some(bpos!());
                        pending_exact = None;
                        buf.uses_syntax = true;
                        emit_op!(RegexOp::NotSyntaxSpec);
                        emit!(u8::from(SyntaxClass::Word));
                    }

                    // \sC — syntax class C
                    b's' => {
                        if p >= plen {
                            return Err(RegexCompileError {
                                message: "Premature end of regular expression".to_string(),
                            });
                        }
                        let sc = syntax_spec_code(pattern_bytes[p]);
                        p += 1;
                        laststart = Some(bpos!());
                        pending_exact = None;
                        buf.uses_syntax = true;
                        emit_op!(RegexOp::SyntaxSpec);
                        emit!(sc);
                    }

                    // \SC — not syntax class C
                    b'S' => {
                        if p >= plen {
                            return Err(RegexCompileError {
                                message: "Premature end of regular expression".to_string(),
                            });
                        }
                        let sc = syntax_spec_code(pattern_bytes[p]);
                        p += 1;
                        laststart = Some(bpos!());
                        pending_exact = None;
                        buf.uses_syntax = true;
                        emit_op!(RegexOp::NotSyntaxSpec);
                        emit!(sc);
                    }

                    // \cC — category C
                    b'c' => {
                        if p >= plen {
                            return Err(RegexCompileError {
                                message: "\\c requires category character".to_string(),
                            });
                        }
                        let cat = pattern_bytes[p];
                        p += 1;
                        laststart = Some(bpos!());
                        pending_exact = None;
                        emit_op!(RegexOp::CategorySpec);
                        emit!(cat);
                    }

                    // \CC — not category C
                    b'C' => {
                        if p >= plen {
                            return Err(RegexCompileError {
                                message: "\\C requires category character".to_string(),
                            });
                        }
                        let cat = pattern_bytes[p];
                        p += 1;
                        laststart = Some(bpos!());
                        pending_exact = None;
                        emit_op!(RegexOp::NotCategorySpec);
                        emit!(cat);
                    }

                    // \1-\9 — backreference
                    b'1'..=b'9' => {
                        let group = (c2 - b'0') as usize;
                        if group > buf.re_nsub
                            || compile_stack
                                .iter()
                                .any(|entry| entry.assigned_group == Some(group))
                        {
                            return Err(RegexCompileError {
                                message: "Invalid back reference".to_string(),
                            });
                        }
                        laststart = Some(bpos!());
                        pending_exact = None;
                        emit_op!(RegexOp::Duplicate);
                        emit!(group as u8);
                    }

                    // \{ — interval \{n,m\}
                    b'{' => {
                        // Parse interval
                        let interval_start = p;
                        let (min_count, max_count) = parse_interval(pattern_bytes, &mut p)?;

                        let Some(mut last) = laststart else {
                            // GNU regex-emacs.c:2427 `unfetch_interval`: a
                            // syntactically valid interval without a preceding
                            // atom is literal text beginning with `{`.
                            p = interval_start;
                            goto_normal_char(
                                b'{' as u32,
                                &mut buf,
                                &mut pending_exact,
                                &mut laststart,
                                &mut laststart_is_group,
                            );
                            continue;
                        };
                        if !laststart_is_group {
                            last = split_trailing_exactn_atom_if_needed(&mut buf, last);
                        }

                        compile_interval(min_count, max_count, last, &mut buf)?;
                        laststart = Some(last);
                        laststart_is_group = false;
                        pending_exact = None;
                    }

                    // Other escaped characters — treat as literal
                    _ => {
                        if buf.multibyte && c2 >= 0x80 {
                            let char_start = p - 1;
                            let (code, len) = decode_pattern_char(pattern_bytes, char_start, true)
                                .unwrap_or((c2 as u32, 1));
                            p = char_start + len;
                            goto_normal_char(
                                code,
                                &mut buf,
                                &mut pending_exact,
                                &mut laststart,
                                &mut laststart_is_group,
                            );
                        } else {
                            goto_normal_char(
                                c2 as u32,
                                &mut buf,
                                &mut pending_exact,
                                &mut laststart,
                                &mut laststart_is_group,
                            );
                        }
                    }
                }
            }

            // ----------------------------------------------------------
            // Normal character — add to exactn
            // ----------------------------------------------------------
            _ => {
                if buf.multibyte && c >= 0x80 {
                    let char_start = p - 1;
                    let (code, len) = decode_pattern_char(pattern_bytes, char_start, true)
                        .unwrap_or((c as u32, 1));
                    p = char_start + len;
                    goto_normal_char(
                        code,
                        &mut buf,
                        &mut pending_exact,
                        &mut laststart,
                        &mut laststart_is_group,
                    );
                } else {
                    goto_normal_char(
                        c as u32,
                        &mut buf,
                        &mut pending_exact,
                        &mut laststart,
                        &mut laststart_is_group,
                    );
                }
            }
        }
    }

    // Check for unmatched \(
    if !compile_stack.is_empty() {
        return Err(RegexCompileError {
            message: "Unmatched ( or \\(".to_string(),
        });
    }

    // Handle final alternation fixup
    if let Some(fixup) = fixup_alt_jump {
        let target = bpos!() as i16 - fixup as i16 - 2;
        store_number(&mut buf.buffer, fixup, target);
    }

    // Emit final succeed — but only for non-POSIX patterns.
    //
    // GNU regex-emacs.c:2683-2686:
    //
    //     /* If we don't want backtracking, force success
    //        the first time we reach the end of the compiled pattern.  */
    //     if (!posix_backtracking)
    //       BUF_PUSH (succeed);
    //
    // When `posix_backtracking` is true the matcher must see the
    // natural "fell off the end of the bytecode" path so the POSIX
    // longest-match logic at regex-emacs.c:4272-4344 can run. Emitting
    // `succeed` unconditionally (as an earlier version of this file
    // did) made every pattern jump to `succeed_label`, bypassing the
    // longest-match code entirely.
    if !posix {
        emit_op!(RegexOp::Succeed);
    } else {
        // Exit jumps fixed up above target the old buffer end, which is
        // exactly where this terminal op now sits — no target can point
        // past it (the sealed matcher's no-end-check proof relies on
        // every pattern ending in Succeed or PosixEnd).
        emit_op!(RegexOp::PosixEnd);
    }

    // Resolve on_failure_jump_smart loops (GNU does this lazily at first
    // execution; our bytecode is immutable and shared after this point).
    resolve_smart_jumps(&mut buf);

    // Fuse alternations of single positive syntax-class branches
    // (`\w\|\s_` and friends) into one `SyntaxSpecSet` op.  Runs after
    // smart-jump resolution so it sees final `on_failure_jump`s, and before
    // fastmap population so the fastmap walk observes the fused op.
    fuse_syntaxspec_alternations(&mut buf);

    // Populate the fastmap for search-time position skipping.  The
    // standard syntax table stands in for `[[:word:]]`/`[[:space:]]`
    // ASCII baking here; callers searching under a buffer-local table
    // rebake via `recompute_fastmap` and key their cache by table
    // (GNU `search.c:compile_pattern` + `used_syntax`).
    compile_fastmap(&mut buf, &DefaultSyntaxLookup);

    // The multi-literal SIMD prefilter (AUGMENTS the fastmap; the
    // backtracker still verifies every candidate) is built at the first
    // search that can use it (`CompiledPattern::literal_prefilter`). It
    // depends only on the bytecode's required-literal structure, which is
    // syntax-table independent, so it survives `recompute_fastmap` cloning.

    // The rewind view of any keep-string loops, then whether the
    // non-backtracking Pike VM can simulate this bytecode byte-exactly (see
    // `compute_pike_eligible`); it runs on the view, so a keep-string jump of
    // an unexpected shape fails closed (ineligible).
    buf.rewind = build_rewind_view(&buf.buffer);
    buf.pike_eligible = buf.rewind != RewindView::Unavailable && compute_pike_eligible(&buf);
    buf.linear = match buf.rewind_bytecode() {
        Some(rewind) => linear::classify(&buf, rewind),
        None => linear::Linear::Ineligible,
    };

    // Seal: prove the buffer safe for unchecked backtracker dispatch. Runs
    // LAST — after every post-compile rewrite (fusion, splices) — so the
    // validated bytes are exactly what the matcher executes. A compiler bug
    // fails closed to the checked loop (and loudly in debug builds).
    buf.buffer_sealed = validate_sealed_buffer(&buf);
    debug_assert!(
        buf.buffer_sealed,
        "regex compiler produced an unsealable buffer for this pattern"
    );

    // Anchors behind alternations (P3.3 Stage 0), for the search's
    // line-start scan; opt-in until measured.
    if anchor_alt_enabled() {
        buf.start_anchor = start_anchor(&buf.buffer);
    }

    Ok(buf)
}

// ---------------------------------------------------------------------------
// Compiler Helpers
// ---------------------------------------------------------------------------

/// GNU `regex-emacs.c:2765` context check for `^`.
///
/// `p` points just after the `^` byte in `pattern`.  GNU treats `^` as a
/// beginning-of-line assertion only at pattern start, after an alternative
/// (`\|`), or after an opening group (`\(` / `\(?:` / `\(?N:`).  Everywhere
/// else it is a literal character.
fn at_begline_loc_p(pattern: &[u8], p: usize) -> bool {
    if p < 2 {
        return false;
    }

    let mut prev = p - 2;
    match pattern[prev] {
        b'(' | b'|' => {}
        b':' => {
            while prev > 0 && pattern[prev - 1].is_ascii_digit() {
                prev -= 1;
            }
            if !(prev > 1 && pattern[prev - 1] == b'?' && pattern[prev - 2] == b'(') {
                return false;
            }
            prev -= 2;
        }
        _ => return false,
    }

    let slash_end = prev;
    while prev > 0 && pattern[prev - 1] == b'\\' {
        prev -= 1;
    }
    ((slash_end - prev) & 1) != 0
}

/// GNU `regex-emacs.c:2801` context check for `$`.
///
/// `p` points just after the `$` byte in `pattern`.  `$` is an end-of-line
/// assertion only at pattern end, before a closing group (`\)`) or before an
/// alternative (`\|`).
fn at_endline_loc_p(pattern: &[u8], p: usize) -> bool {
    p + 1 < pattern.len() && pattern[p] == b'\\' && matches!(pattern[p + 1], b')' | b'|')
}

fn syntax_spec_code(c: u8) -> u8 {
    SyntaxClass::from_syntax_spec_byte(c)
        .map(u8::from)
        .unwrap_or(0o377)
}

/// Emit a literal character as part of an `exactn` sequence.
///
/// GNU `regex-emacs.c` applies `RE_TRANSLATE (translate, c)` before
/// buffering the char, so a pattern like `"C"` compiled with
/// case-fold on is stored in the bytecode as `'c'`. At match time the
/// buffer char is also `tr()`-translated, so both sides are
/// case-folded to the canonical (lowercase) form. Without the
/// translate-on-compile step here, the pattern byte stays as `'C'`
/// while the matched text byte becomes `'c'` and they fail to compare
/// equal.
fn goto_normal_char(
    c: u32,
    buf: &mut CompiledPattern,
    pending_exact: &mut Option<usize>,
    laststart: &mut Option<usize>,
    laststart_is_group: &mut bool,
) {
    let c = if buf.multibyte {
        buf.translate.as_ref().map_or(c, |table| table.translate(c))
    } else {
        // GNU `regex-emacs.c:normal_char` promotes a unibyte pattern byte
        // before consulting the case table.  High bytes become byte8
        // characters and deliberately bypass translation; otherwise values
        // such as 0xDB are folded as Latin-1 letters even though they denote
        // opaque raw bytes in the regexp.
        let mut byte = c as u8;
        let promoted = regex_unibyte_to_char(byte);
        if !emacs_char::char_byte8_p(promoted)
            && let Some(table) = buf.translate.as_ref()
        {
            let translated = table.translate(promoted);
            if promoted != translated
                && let Some(translated_byte) = regex_char_to_unibyte(translated)
            {
                byte = translated_byte;
            }
        }
        byte as u32
    };

    let mut encoded = [0u8; emacs_char::MAX_MULTIBYTE_LENGTH];
    let encoded_len = if buf.multibyte {
        emacs_char::char_string(c, &mut encoded)
    } else {
        encoded[0] = c as u8;
        1
    };

    // If we have a pending exactn and it hasn't reached max length (255),
    // just append to it
    if let Some(exact_pos) = *pending_exact {
        let count = buf.buffer[exact_pos] as usize;
        if count + encoded_len <= 255 {
            buf.buffer[exact_pos] += encoded_len as u8;
            buf.buffer.extend_from_slice(&encoded[..encoded_len]);
            *laststart_is_group = false;
            return;
        }
    }

    // Start a new exactn
    *laststart = Some(buf.buffer.len());
    *laststart_is_group = false;
    buf.buffer.push(RegexOp::Exactn as u8);
    *pending_exact = Some(buf.buffer.len());
    buf.buffer.push(encoded_len as u8);
    buf.buffer.extend_from_slice(&encoded[..encoded_len]);
}

/// Decode one Emacs character from a pattern byte slice starting at `pos`.
///
/// Multibyte patterns use Emacs internal encoding; unibyte patterns map each
/// byte to a single character code directly.
fn decode_pattern_char(bytes: &[u8], pos: usize, multibyte: bool) -> Option<(u32, usize)> {
    if pos >= bytes.len() {
        return None;
    }
    if multibyte {
        Some(emacs_char::string_char(&bytes[pos..]))
    } else {
        Some((bytes[pos] as u32, 1))
    }
}

fn emacs_char_to_rust_char(code: u32) -> char {
    if emacs_char::char_byte8_p(code) {
        char::from(emacs_char::char_to_byte8(code))
    } else {
        char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER)
    }
}
