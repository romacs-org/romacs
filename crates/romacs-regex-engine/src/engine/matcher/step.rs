//! The per-character tests every engine shares (the backtracker, the Pike
//! VM and the DFA): reading a character, `anychar`, `exactn`, charsets,
//! syntax and category specs, and the word and symbol assertions.  One
//! implementation of each predicate is what keeps the Pike VM and the DFA
//! byte-exact with the backtracker on case folding, multibyte text,
//! charsets, syntax classes, categories and boundaries.
//!
//! GNU: `re_match_2_internal`'s character macros (`PREFETCH`,
//! `WORDCHAR_P`, `AT_WORD_BOUNDARY`) and `execute_charset` in
//! regex-emacs.c.

use crate::emacs_char;
use crate::engine::class::{
    PosixClassCaseMode, posix_class_matches, regex_char_to_unibyte, regex_scalar_char,
    regex_unibyte_to_char,
};
use crate::engine::opcode::RegexOp;
use crate::engine::pattern::CompiledPattern;
use crate::engine::syntax::SyntaxLookup;
use crate::engine::translate::{CaseTranslation, re_tr};
use crate::syntax_class::SyntaxClass;

/// Decode the Emacs character at `pos` (mirrors the `text_char` closure).
#[inline]
pub fn re_text_char(text: &[u8], pos: usize, target_multibyte: bool) -> Option<(u32, usize)> {
    if pos >= text.len() {
        return None;
    }
    if target_multibyte {
        Some(emacs_char::string_char(&text[pos..]))
    } else {
        Some((regex_unibyte_to_char(text[pos]), 1))
    }
}

/// Start byte of the character before `pos` (mirrors `prev_char_start`).
#[inline]
pub(crate) fn re_prev_char_start(text: &[u8], pos: usize, target_multibyte: bool) -> Option<usize> {
    if pos == 0 {
        return None;
    }
    if !target_multibyte {
        return Some(pos - 1);
    }
    let mut p = pos - 1;
    while p > 0 && (text[p] & 0xC0) == 0x80 {
        p -= 1;
    }
    Some(p)
}

/// `AnyChar` (`.`): match any character except newline.  Returns the byte
/// length of the consumed char on success.  Mirrors the `AnyChar` arm.
#[inline]
pub fn match_anychar_at(
    text: &[u8],
    d: usize,
    stop: usize,
    target_multibyte: bool,
    translate: &Option<CaseTranslation>,
) -> Option<usize> {
    if d >= stop {
        return None;
    }
    let (buf_ch, buf_len) = re_text_char(text, d, target_multibyte)?;
    if re_tr(translate, buf_ch) == '\n' as u32 {
        return None;
    }
    Some(buf_len)
}

/// Match ONE character of an `Exactn` literal at literal byte offset
/// `lit_off` against the input char at `d`.  Returns `(pattern_advance,
/// text_advance)` on success.  Mirrors one iteration of the `Exactn` arm's
/// inner loop, including the unibyte/multibyte + case-fold subtleties.
#[inline]
#[allow(clippy::too_many_arguments)] // mirrors the backtracker's Exactn arm state
pub fn match_exactn_char_at(
    lit: &[u8],
    lit_off: usize,
    pattern_multibyte: bool,
    target_multibyte: bool,
    translate: &Option<CaseTranslation>,
    text: &[u8],
    d: usize,
    stop: usize,
) -> Option<(usize, usize)> {
    if d >= stop {
        return None;
    }
    // ASCII fast path — an ASCII literal byte is the same character in
    // every representation mix (pattern/text, unibyte/multibyte), always
    // one byte long on both sides; when the text byte's case translation
    // also stays ASCII the whole test is one translate + one compare.
    // Translations that leave ASCII (exotic case tables) fall through to
    // the full path below, which recomputes from scratch.
    let ascii_pat_byte = lit[lit_off];
    if ascii_pat_byte < 0x80
        && let Some(&text_byte) = text.get(d)
        && text_byte < 0x80
    {
        let translated = re_tr(translate, text_byte as u32);
        if translated < 0x80 {
            return if translated as u8 == ascii_pat_byte {
                Some((1, 1))
            } else {
                None
            };
        }
    }
    let (buf_ch, buf_len) = re_text_char(text, d, target_multibyte)?;
    if target_multibyte {
        let (pat_ch, pat_len) = if pattern_multibyte {
            emacs_char::string_char(&lit[lit_off..])
        } else {
            (regex_unibyte_to_char(lit[lit_off]), 1)
        };
        if re_tr(translate, buf_ch) != pat_ch {
            return None;
        }
        Some((pat_len, buf_len))
    } else {
        let (pat_byte, pat_advance) = if pattern_multibyte {
            let (pat_ch, pat_len) = emacs_char::string_char(&lit[lit_off..]);
            let byte = regex_char_to_unibyte(pat_ch)?;
            (byte, pat_len)
        } else {
            (lit[lit_off], 1)
        };
        let buf_byte = text[d];
        let mut translated = regex_unibyte_to_char(buf_byte);
        if !emacs_char::char_byte8_p(translated) {
            translated = re_tr(translate, translated);
            if let Some(byte) = regex_char_to_unibyte(translated) {
                translated = byte as u32;
            } else {
                translated = buf_byte as u32;
            }
        } else {
            translated = buf_byte as u32;
        }
        if translated as u8 != pat_byte {
            return None;
        }
        Some((pat_advance, 1))
    }
}

/// `Charset` / `CharsetNot`: match the char at `d` against the bitmap +
/// range-table + POSIX class bits of the charset opcode at
/// `charset_op_pos`.  Returns consumed byte length on success.  Mirrors
/// the `Charset | CharsetNot` arm exactly.
/// Charset test at `d`. The ASCII case — GNU's charset op shape: read the
/// byte, case-translate it, bit-test the bitmap — inlines into both matcher
/// call sites; everything else goes through the outlined
/// [`match_charset_at_slow`]. The call boundary itself measured ~40 of the
/// ~67 Ir per test, on millions of tests per workload.
#[allow(clippy::too_many_arguments)] // mirrors GNU execute_charset's inputs
#[inline(always)]
pub fn match_charset_at(
    pattern: &CompiledPattern,
    charset_op_pos: usize,
    text: &[u8],
    d: usize,
    stop: usize,
    target_multibyte: bool,
    translate: &Option<CaseTranslation>,
    syntax: &dyn SyntaxLookup,
) -> Option<usize> {
    let bytecode = &pattern.buffer;

    if d >= stop {
        return None;
    }

    // ASCII fast path — an ASCII byte decodes to itself in both text
    // representations, always has length 1, and (when its translation
    // stays ASCII) needs none of the slow path's decode /
    // unibyte-conversion / case-mode machinery. The class check mirrors
    // the slow path exactly: classes test the UNTRANSLATED character.
    // (`stop` may exceed `text.len()`; a past-the-end read is a plain
    // no-match, matching `re_text_char`'s bounds behavior.)
    let &first_byte = text.get(d)?;
    if first_byte < 0x80 {
        let translated = re_tr(translate, first_byte as u32);
        if translated < 0x80 {
            let negate = bytecode[charset_op_pos] == RegexOp::CharsetNot as u8;
            let bitmap_len = bytecode[charset_op_pos + 1] as usize & 0x7F;
            let bitmap_start = charset_op_pos + 2;
            let c = translated as usize;
            let bitmap_hit =
                (c / 8) < bitmap_len && (bytecode[bitmap_start + c / 8] >> (c % 8)) & 1 != 0;
            let in_set = bitmap_hit
                || pattern
                    .charset_class_bits
                    .get(&charset_op_pos)
                    .copied()
                    .map(|bits| {
                        let class_case_mode =
                            PosixClassCaseMode::from_translation(translate.as_ref());
                        posix_class_matches(
                            first_byte as u32,
                            bits,
                            Some(d),
                            syntax,
                            class_case_mode,
                        )
                    })
                    .unwrap_or(false);
            return if negate != in_set { Some(1) } else { None };
        }
    }

    match_charset_at_slow(
        pattern,
        charset_op_pos,
        text,
        d,
        stop,
        target_multibyte,
        translate,
        syntax,
    )
}

/// Non-ASCII / translation-escapes-ASCII remainder of the charset test.
#[allow(clippy::too_many_arguments)] // mirrors the wrapper's seam
#[inline(never)]
fn match_charset_at_slow(
    pattern: &CompiledPattern,
    charset_op_pos: usize,
    text: &[u8],
    d: usize,
    stop: usize,
    target_multibyte: bool,
    translate: &Option<CaseTranslation>,
    syntax: &dyn SyntaxLookup,
) -> Option<usize> {
    let bytecode = &pattern.buffer;
    let negate = bytecode[charset_op_pos] == RegexOp::CharsetNot as u8;
    let bitmap_len = bytecode[charset_op_pos + 1] as usize & 0x7F;
    let bitmap_start = charset_op_pos + 2;

    if d >= stop {
        return None;
    }

    let class_case_mode = PosixClassCaseMode::from_translation(translate.as_ref());
    let (orig_ch, ch_len) = re_text_char(text, d, target_multibyte)?;
    let mut ch = orig_ch;
    let mut unibyte_char = false;

    if target_multibyte {
        ch = re_tr(translate, ch);
        if let Some(byte) = regex_char_to_unibyte(ch) {
            unibyte_char = true;
            ch = byte as u32;
        }
    } else {
        let mut converted = regex_unibyte_to_char(text[d]);
        if !emacs_char::char_byte8_p(converted) {
            converted = re_tr(translate, converted);
            if let Some(byte) = regex_char_to_unibyte(converted) {
                unibyte_char = true;
                ch = byte as u32;
            }
        } else {
            unibyte_char = true;
            ch = text[d] as u32;
        }
    }

    let in_set = if unibyte_char {
        let c = ch as usize;
        let bitmap_hit = if (c / 8) < bitmap_len {
            let byte = bytecode[bitmap_start + c / 8];
            (byte >> (c % 8)) & 1 != 0
        } else {
            false
        };
        bitmap_hit
            || (ch < 0x80
                && pattern
                    .charset_class_bits
                    .get(&charset_op_pos)
                    .copied()
                    .map(|bits| {
                        posix_class_matches(orig_ch, bits, Some(d), syntax, class_case_mode)
                    })
                    .unwrap_or(false))
    } else {
        let range_hit = pattern
            .multibyte_charsets
            .get(&charset_op_pos)
            .map(|ranges| {
                let ch = regex_scalar_char(ch);
                ranges.iter().any(|&(lo, hi)| ch >= lo && ch <= hi)
            })
            .unwrap_or(false);
        range_hit
            || pattern
                .charset_class_bits
                .get(&charset_op_pos)
                .copied()
                .map(|bits| posix_class_matches(orig_ch, bits, Some(d), syntax, class_case_mode))
                .unwrap_or(false)
    };

    let matched = if negate { !in_set } else { in_set };
    if matched { Some(ch_len) } else { None }
}

/// `SyntaxSpec` (`negate=false`) / `NotSyntaxSpec` (`negate=true`).
#[inline]
pub fn match_syntaxspec_at(
    class_byte: u8,
    negate: bool,
    text: &[u8],
    d: usize,
    stop: usize,
    target_multibyte: bool,
    syntax: &dyn SyntaxLookup,
) -> Option<usize> {
    if d >= stop {
        return None;
    }
    let (c, len) = re_text_char(text, d, target_multibyte)?;
    let is = syntax.emacs_char_syntax_at(emacs_char::EmacsChar::from_code_unchecked(c), d) as u8
        == class_byte;
    if is != negate { Some(len) } else { None }
}

/// `SyntaxSpecSet`: char's syntax class is in the bitmask.
#[inline]
pub fn match_syntaxspecset_at(
    mask: u16,
    text: &[u8],
    d: usize,
    stop: usize,
    target_multibyte: bool,
    syntax: &dyn SyntaxLookup,
) -> Option<usize> {
    if d >= stop {
        return None;
    }
    let (c, len) = re_text_char(text, d, target_multibyte)?;
    let class =
        syntax.emacs_char_syntax_at(emacs_char::EmacsChar::from_code_unchecked(c), d) as u16;
    if (mask >> class) & 1 != 0 {
        Some(len)
    } else {
        None
    }
}

/// `CategorySpec` (`negate=false`) / `NotCategorySpec` (`negate=true`).
#[inline]
pub fn match_categoryspec_at(
    cat: u8,
    negate: bool,
    text: &[u8],
    d: usize,
    stop: usize,
    target_multibyte: bool,
    syntax: &dyn SyntaxLookup,
) -> Option<usize> {
    if d >= stop {
        return None;
    }
    let (c, len) = re_text_char(text, d, target_multibyte)?;
    let has = syntax.char_has_category(regex_scalar_char(c), cat);
    if has != negate { Some(len) } else { None }
}

/// Character and syntax class at `pos`.  Keeping the character lets GNU word
/// boundaries distinguish adjacent word constituents from different scripts.
#[inline]
fn re_char_and_syntax(
    text: &[u8],
    pos: usize,
    target_multibyte: bool,
    syntax: &dyn SyntaxLookup,
) -> Option<(emacs_char::EmacsChar, SyntaxClass)> {
    re_text_char(text, pos, target_multibyte).map(|(c, _)| {
        let c = emacs_char::EmacsChar::from_code_unchecked(c);
        (c, syntax.emacs_char_syntax_at(c, pos))
    })
}

/// Is the char at `pos` a word OR symbol constituent?  (`false` past the ends.)
#[inline]
fn re_char_is_symbol(
    text: &[u8],
    pos: usize,
    target_multibyte: bool,
    syntax: &dyn SyntaxLookup,
) -> bool {
    re_text_char(text, pos, target_multibyte)
        .map(|(c, _)| {
            let s = syntax.emacs_char_syntax_at(emacs_char::EmacsChar::from_code_unchecked(c), pos);
            s == SyntaxClass::Word || s == SyntaxClass::Symbol
        })
        .unwrap_or(false)
}

/// `\b` word boundary (mirrors GNU `regex-emacs.c` `case wordbound`).
#[inline]
fn assert_word_boundary(
    text: &[u8],
    d: usize,
    target_multibyte: bool,
    syntax: &dyn SyntaxLookup,
) -> bool {
    // GNU Case 1 (regex-emacs.c:4972-4974): a position at the beginning or end
    // of the searched region is *unconditionally* a word boundary — `\b`
    // succeeds and `\B` fails there, regardless of the adjacent character's
    // word syntax.  The previous neighbour-only computation treated the missing
    // edge character as non-word, so `\b` wrongly failed (and `\B` matched) at
    // the edges of a string / accessible buffer region whenever the adjacent
    // character was not a word constituent (e.g. `\b` on "." or the empty
    // string).  `d == 0` is AT_STRINGS_BEG and `d == text.len()` is
    // AT_STRINGS_END (the same edges `BegBuf` / `EndBuf` use).
    if d == 0 || d == text.len() {
        return true;
    }
    let previous = re_prev_char_start(text, d, target_multibyte)
        .and_then(|p| re_char_and_syntax(text, p, target_multibyte, syntax));
    let current = re_char_and_syntax(text, d, target_multibyte, syntax);
    match (previous, current) {
        (Some((c1, SyntaxClass::Word)), Some((c2, SyntaxClass::Word))) => {
            syntax.emacs_word_boundary_between(c1, c2)
        }
        (Some((_, previous)), Some((_, current))) => {
            (previous == SyntaxClass::Word) != (current == SyntaxClass::Word)
        }
        _ => false,
    }
}

/// `\<` word beginning.
#[inline]
fn assert_word_beg(
    text: &[u8],
    d: usize,
    target_multibyte: bool,
    syntax: &dyn SyntaxLookup,
) -> bool {
    let Some((c2, SyntaxClass::Word)) = re_char_and_syntax(text, d, target_multibyte, syntax)
    else {
        return false;
    };
    let Some(previous) = re_prev_char_start(text, d, target_multibyte)
        .and_then(|p| re_char_and_syntax(text, p, target_multibyte, syntax))
    else {
        return true;
    };
    match previous {
        (_, class) if class != SyntaxClass::Word => true,
        (c1, SyntaxClass::Word) => syntax.emacs_word_boundary_between(c1, c2),
        _ => false,
    }
}

/// `\>` word end.
#[inline]
fn assert_word_end(
    text: &[u8],
    d: usize,
    target_multibyte: bool,
    syntax: &dyn SyntaxLookup,
) -> bool {
    let Some(previous) = re_prev_char_start(text, d, target_multibyte)
        .and_then(|p| re_char_and_syntax(text, p, target_multibyte, syntax))
    else {
        return false;
    };
    let (c1, SyntaxClass::Word) = previous else {
        return false;
    };
    let Some(current) = re_char_and_syntax(text, d, target_multibyte, syntax) else {
        return true;
    };
    match current {
        (_, class) if class != SyntaxClass::Word => true,
        (c2, SyntaxClass::Word) => syntax.emacs_word_boundary_between(c1, c2),
        _ => false,
    }
}

/// `\_<` symbol beginning.
#[inline]
fn assert_sym_beg(
    text: &[u8],
    d: usize,
    target_multibyte: bool,
    syntax: &dyn SyntaxLookup,
) -> bool {
    let prev_sym = re_prev_char_start(text, d, target_multibyte)
        .map(|p| re_char_is_symbol(text, p, target_multibyte, syntax))
        .unwrap_or(false);
    let curr_sym = re_char_is_symbol(text, d, target_multibyte, syntax);
    !prev_sym && curr_sym
}

/// `\_>` symbol end.
#[inline]
fn assert_sym_end(
    text: &[u8],
    d: usize,
    target_multibyte: bool,
    syntax: &dyn SyntaxLookup,
) -> bool {
    let prev_sym = re_prev_char_start(text, d, target_multibyte)
        .map(|p| re_char_is_symbol(text, p, target_multibyte, syntax))
        .unwrap_or(false);
    let curr_sym = re_char_is_symbol(text, d, target_multibyte, syntax);
    prev_sym && !curr_sym
}

/// Zero-width syntax assertions and their GNU match-limit protocol.
///
/// GNU's `wordbeg` and `symbeg` opcodes use the ordinary `PREFETCH`, so
/// inspecting the current character fails when `d == stop`.  Boundaries and
/// end assertions use `PREFETCH_NOLIMIT` (or only inspect the previous
/// character).  Keeping that distinction in one exhaustive enum evaluator
/// prevents the backtracking and Pike engines from choosing independently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SyntaxAssertion {
    WordBoundary,
    NotWordBoundary,
    WordBeginning,
    WordEnd,
    SymbolBeginning,
    SymbolEnd,
}

impl SyntaxAssertion {
    pub(crate) fn from_regex_op(op: RegexOp) -> Self {
        match op {
            RegexOp::WordBound => Self::WordBoundary,
            RegexOp::NotWordBound => Self::NotWordBoundary,
            RegexOp::WordBeg => Self::WordBeginning,
            RegexOp::WordEnd => Self::WordEnd,
            RegexOp::SymBeg => Self::SymbolBeginning,
            RegexOp::SymEnd => Self::SymbolEnd,
            _ => unreachable!("{op:?} is not a zero-width syntax assertion"),
        }
    }
}

#[inline]
pub(crate) fn evaluate_syntax_assertion(
    assertion: SyntaxAssertion,
    text: &[u8],
    d: usize,
    stop: usize,
    target_multibyte: bool,
    syntax: &dyn SyntaxLookup,
) -> bool {
    match assertion {
        SyntaxAssertion::WordBoundary => assert_word_boundary(text, d, target_multibyte, syntax),
        SyntaxAssertion::NotWordBoundary => {
            !assert_word_boundary(text, d, target_multibyte, syntax)
        }
        SyntaxAssertion::WordBeginning => {
            d < stop && assert_word_beg(text, d, target_multibyte, syntax)
        }
        SyntaxAssertion::WordEnd => assert_word_end(text, d, target_multibyte, syntax),
        SyntaxAssertion::SymbolBeginning => {
            d < stop && assert_sym_beg(text, d, target_multibyte, syntax)
        }
        SyntaxAssertion::SymbolEnd => assert_sym_end(text, d, target_multibyte, syntax),
    }
}
