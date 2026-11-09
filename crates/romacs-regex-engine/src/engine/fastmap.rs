use crate::emacs_char;
use crate::engine::class::{PosixClassCaseMode, posix_class_matches};
use crate::engine::opcode::{RegexOp, extract_number};
use crate::engine::pattern::CompiledPattern;
use crate::engine::scan::FoldedScans;
use crate::engine::syntax::SyntaxLookup;
use std::collections::HashSet;

/// Recompute the fastmap of an already-compiled pattern against a
/// (possibly buffer-local) syntax table.
///
/// GNU bakes `[[:word:]]` / `[[:space:]]` ASCII membership into the
/// charset bitmap with the buffer's syntax table at compile time and
/// keys the compiled-pattern cache by that table
/// (`search.c:compile_pattern`, `cp->syntax_table`).  Romacs defers
/// the class membership test to match time, so only the FASTMAP bakes
/// table content; the front-end cache calls this to rebuild the
/// fastmap for the active table before caching a `used_syntax` entry
/// under that table's key.
pub fn recompute_fastmap(pattern: &mut CompiledPattern, syntax: &dyn SyntaxLookup) {
    compile_fastmap(pattern, syntax);
}

/// Translated from GNU regex-emacs.c `re_compile_fastmap` /
/// `analyze_first`.
///
/// `syntax` is consulted only to bake the ASCII members of
/// syntax-table-dependent POSIX classes (`[[:word:]]`, `[[:space:]]`)
/// of a *leading* charset into the fastmap — the analog of GNU baking
/// them into the charset bitmap at compile time (regex-emacs.c:2081).
/// Patterns whose fastmap took that path are flagged `used_syntax` at
/// compile and must be cache-keyed by syntax table.
pub(crate) fn compile_fastmap(pattern: &mut CompiledPattern, syntax: &dyn SyntaxLookup) {
    // A cached skip set describes this exact map, including the active syntax
    // table. Leave it empty until a search actually needs the ASCII shortcut.
    pattern.sparse_ascii_fastmap.take();
    pattern.folded_scans = FoldedScans::default();
    let mut folded_multibyte_literal = false;
    compile_fastmap_walk(pattern, syntax, &mut folded_multibyte_literal);
    // `fastmap_translated` is the walk's own map; the byte-indexed `fastmap`
    // additionally admits every non-ASCII leading byte when a leading
    // multibyte literal is case-folded (see the `Exactn` arm).
    pattern.fastmap_translated = pattern.fastmap;
    if folded_multibyte_literal {
        for c in 128..256usize {
            pattern.fastmap[c] = true;
        }
    }
}

fn compile_fastmap_walk(
    pattern: &mut CompiledPattern,
    syntax: &dyn SyntaxLookup,
    folded_multibyte_literal: &mut bool,
) {
    pattern.fastmap = [false; 256];
    pattern.can_be_null = false;

    let bytecode = &pattern.buffer;
    if bytecode.is_empty() {
        pattern.can_be_null = true;
        pattern.fastmap_accurate = true;
        return;
    }

    let case_fold = pattern.translate.is_some();

    // Worklist of bytecode positions still to process.
    let mut worklist: Vec<usize> = vec![0];
    let mut visited: HashSet<usize> = HashSet::new();

    while let Some(pc) = worklist.pop() {
        let mut pc = pc;

        loop {
            if !visited.insert(pc) {
                // Already processed this position — avoid infinite loops.
                break;
            }

            if pc >= bytecode.len() {
                // Fell off the end of bytecode — pattern can match empty string.
                pattern.can_be_null = true;
                break;
            }

            let Some(op) = RegexOp::from_byte(bytecode[pc]) else {
                break;
            };
            pc += 1;

            match op {
                RegexOp::Succeed | RegexOp::PosixEnd => {
                    // Pattern can succeed here (may match empty string).
                    pattern.can_be_null = true;
                    break;
                }

                RegexOp::Exactn => {
                    if pc >= bytecode.len() {
                        break;
                    }
                    let count = bytecode[pc] as usize;
                    pc += 1;
                    if count == 0 || pc >= bytecode.len() {
                        break;
                    }
                    let first = bytecode[pc];
                    pattern.fastmap[first as usize] = true;
                    // GNU `analyze_first': "Cover the case of matching a raw
                    // char in a multibyte regexp against unibyte" -- the
                    // eight-bit character's own byte is a candidate too.
                    if pattern.multibyte && (first == 0xC0 || first == 0xC1) {
                        let (c, _) = emacs_char::string_char(&bytecode[pc..]);
                        if emacs_char::char_byte8_p(c) {
                            pattern.fastmap[emacs_char::char_to_byte8(c) as usize] = true;
                        }
                    }
                    if !pattern.multibyte && first >= 0x80 {
                        // GNU `analyze_first': a unibyte pattern's byte
                        // matches multibyte text as its eight-bit character,
                        // so its leading code is a candidate too.
                        let byte8 = emacs_char::byte8_to_char(first);
                        pattern.fastmap[emacs_char::char_leading_code(byte8) as usize] = true;
                    }
                    if case_fold {
                        if first >= 0x80 {
                            // Multibyte character: the case-folded form may have
                            // a different leading byte (e.g. Cyrillic
                            // т = D1 82 vs Т = D0 A2), so byte-level case-folding
                            // of `first` is meaningless and would wrongly exclude
                            // the other case's lead byte. The byte-indexed
                            // fastmap conservatively allows all multibyte leading
                            // bytes, matching the Charset path; the translated
                            // map keeps just `first` -- the stored character is
                            // already translated -- and `compile_fastmap` widens
                            // the byte map afterwards.  A unibyte pattern's
                            // byte is no leading code, so both maps widen.
                            if pattern.multibyte {
                                *folded_multibyte_literal = true;
                            } else {
                                for c in 128..256usize {
                                    pattern.fastmap[c] = true;
                                }
                            }
                        } else {
                            let upper = (first as char)
                                .to_uppercase()
                                .next()
                                .unwrap_or(first as char)
                                as u8;
                            let lower = (first as char)
                                .to_lowercase()
                                .next()
                                .unwrap_or(first as char)
                                as u8;
                            pattern.fastmap[upper as usize] = true;
                            pattern.fastmap[lower as usize] = true;
                        }
                    }
                    break; // This opcode consumes input — done on this path.
                }

                RegexOp::AnyChar => {
                    // Matches any character except newline.
                    for c in 0..256usize {
                        if c != b'\n' as usize {
                            pattern.fastmap[c] = true;
                        }
                    }
                    break;
                }

                RegexOp::Charset => {
                    if pc >= bytecode.len() {
                        break;
                    }
                    let charset_op_pos = pc - 1;
                    let bitmap_len = bytecode[pc] as usize & 0x7F;
                    pc += 1;
                    for c in 0..256usize {
                        if c / 8 < bitmap_len
                            && pc + c / 8 < bytecode.len()
                            && (bytecode[pc + c / 8] >> (c % 8)) & 1 != 0
                        {
                            pattern.fastmap[c] = true;
                            // GNU `analyze_first': "To match raw bytes (in
                            // the 80..ff range) against multibyte strings,
                            // add their leading bytes to the fastmap."
                            if c >= 0x80 {
                                let byte8 = emacs_char::byte8_to_char(c as u8);
                                pattern.fastmap[emacs_char::char_leading_code(byte8) as usize] =
                                    true;
                            }
                        }
                    }
                    // If this charset has multibyte ranges, conservatively
                    // allow all non-ASCII leading bytes in the fastmap.
                    if pattern.multibyte_charsets.contains_key(&charset_op_pos) {
                        for c in 128..256usize {
                            pattern.fastmap[c] = true;
                        }
                    }
                    // POSIX classes.  The bitmap only holds the fixed-set
                    // classes' ASCII members (`apply_posix_class`); the
                    // syntax-dependent ones (`[[:word:]]`, `[[:space:]]`)
                    // are re-derived per match from the active table, so
                    // bake their current ASCII members here — the analog
                    // of GNU compiling them into the bitmap with
                    // `re_iswctype` (regex-emacs.c:2081-2092).  Also set
                    // every translated partner, mirroring GNU's
                    // `SET_LIST_BIT (TRANSLATE (c))`, since the search
                    // skip loop indexes the fastmap by translated input.
                    // And a class can match multibyte characters, so all
                    // non-ASCII leading bytes become candidates — GNU's
                    // "If we can match a character class, we can match
                    // any multibyte characters" (analyze_first).
                    if let Some(&bits) = pattern.charset_class_bits.get(&charset_op_pos) {
                        for c in 0u8..0x80 {
                            if posix_class_matches(
                                c as u32,
                                bits,
                                None,
                                syntax,
                                PosixClassCaseMode::Sensitive,
                            ) {
                                pattern.fastmap[c as usize] = true;
                                if let Some(table) = &pattern.translate {
                                    pattern.fastmap[table.translate_byte(c) as usize] = true;
                                }
                            }
                        }
                        for c in 128..256usize {
                            pattern.fastmap[c] = true;
                        }
                    }
                    break;
                }

                RegexOp::CharsetNot => {
                    if pc >= bytecode.len() {
                        break;
                    }
                    let bitmap_len = bytecode[pc] as usize & 0x7F;
                    pc += 1;
                    for c in 0..256usize {
                        let in_set = if c / 8 < bitmap_len && pc + c / 8 < bytecode.len() {
                            (bytecode[pc + c / 8] >> (c % 8)) & 1 != 0
                        } else {
                            false
                        };
                        if !in_set {
                            pattern.fastmap[c] = true;
                        }
                    }
                    // GNU `analyze_first_fastmap` treats any `charset_not`
                    // as capable of matching multibyte characters beyond the
                    // bitmap, even when the bitmap covers every raw byte.
                    // The search loop will only try real character
                    // boundaries, so this conservative non-ASCII coverage is
                    // enough to avoid skipping candidates like `é` for
                    // `[^\x00-\xff]`.
                    for c in 128..256usize {
                        pattern.fastmap[c] = true;
                    }
                    break;
                }

                RegexOp::SyntaxSpec | RegexOp::NotSyntaxSpec | RegexOp::SyntaxSpecSet => {
                    // GNU `analyze_first` gives up on `\sX` / `\SX` in
                    // first-character position: "This match depends on
                    // text properties.  These end with aborting
                    // optimizations" (regex-emacs.c:3186-3190, returning
                    // failure so `re_compile_fastmap` sets
                    // `bufp->can_be_null = 1`).  The syntax class of a
                    // byte is a property of the *buffer's* table at match
                    // time, so no compile-time fastmap can restrict the
                    // candidate set without hardcoding a table.  Setting
                    // `can_be_null` disables the search-time fastmap skip
                    // exactly as in GNU `re_search_2`
                    // (regex-emacs.c:3483).
                    pattern.can_be_null = true;
                    pattern.fastmap_accurate = true;
                    return;
                }

                RegexOp::CategorySpec | RegexOp::NotCategorySpec => {
                    // Categories are too dynamic to predict — allow all bytes.
                    pattern.fastmap = [true; 256];
                    break;
                }

                // Zero-width assertions: they don't consume input, so we
                // continue to the next opcode to find what actually starts
                // the match.
                RegexOp::BegLine
                | RegexOp::EndLine
                | RegexOp::BegBuf
                | RegexOp::EndBuf
                | RegexOp::AtDot
                | RegexOp::WordBound
                | RegexOp::NotWordBound
                | RegexOp::WordBeg
                | RegexOp::WordEnd
                | RegexOp::SymBeg
                | RegexOp::SymEnd => {
                    // Continue to the next opcode.
                }

                RegexOp::StartMemory | RegexOp::StopMemory => {
                    // Skip the group number byte, continue.
                    pc += 1;
                }

                RegexOp::Duplicate => {
                    // Backreferences can match anything — set all.
                    pattern.fastmap = [true; 256];
                    break;
                }

                RegexOp::NoOp => {
                    // Continue to next opcode.
                }

                RegexOp::Jump => {
                    if pc + 1 >= bytecode.len() {
                        break;
                    }
                    let offset = extract_number(bytecode, pc);
                    pc = ((pc as i64) + 2 + (offset as i64)) as usize;
                    // Continue walking from the jump target (don't break).
                }

                RegexOp::OnFailureJump
                | RegexOp::OnFailureKeepStringJump
                | RegexOp::OnFailureJumpLoop
                | RegexOp::OnFailureJumpNastyloop
                | RegexOp::OnFailureJumpSmart => {
                    if pc + 1 >= bytecode.len() {
                        break;
                    }
                    let offset = extract_number(bytecode, pc);
                    pc += 2;
                    // Both the fallthrough (next opcode) and the jump target
                    // can start a match.  Push the jump target onto the
                    // worklist and continue with the fallthrough.
                    let target = ((pc as i64) + (offset as i64)) as usize;
                    worklist.push(target);
                    // Continue with the next opcode (fallthrough path).
                }

                RegexOp::SucceedN => {
                    // succeed_n <offset:2> <counter:2>
                    // When counter > 0, acts like a mandatory match of what follows.
                    // When counter == 0, acts like on_failure_jump.
                    // For fastmap purposes, both paths are possible.
                    if pc + 3 >= bytecode.len() {
                        break;
                    }
                    let offset = extract_number(bytecode, pc);
                    let target = ((pc as i64) + 2 + (offset as i64)) as usize;
                    worklist.push(target);
                    pc += 4; // skip offset + counter, continue with fallthrough
                }

                RegexOp::JumpN => {
                    // jump_n <offset:2> <counter:2>
                    // If counter > 0, jumps; if counter == 0, falls through.
                    // For fastmap, both paths are possible.
                    if pc + 3 >= bytecode.len() {
                        break;
                    }
                    let offset = extract_number(bytecode, pc);
                    let target = ((pc as i64) + 2 + (offset as i64)) as usize;
                    worklist.push(target);
                    pc += 4; // fallthrough
                }

                RegexOp::SetNumberAt => {
                    // set_number_at <offset:2> <value:2> — no input consumed.
                    // GNU `forall_firstchar_1': when it sets the counter of
                    // the `succeed_n' right after it (offset 5; an interval
                    // with a lower bound of at least 1), that `succeed_n'
                    // only decrements and falls through into the body, so
                    // its skip branch is no first-character path.  Walking
                    // it made `[0-9]\{3\}' look nullable and turned the
                    // fastmap off.
                    if pc + 3 < bytecode.len()
                        && extract_number(bytecode, pc) == 5
                        && bytecode.get(pc + 4) == Some(&(RegexOp::SucceedN as u8))
                    {
                        pc += 4 + 5;
                    } else {
                        pc += 4;
                    }
                }
            }
        }
    }

    pattern.fastmap_accurate = true;
}
