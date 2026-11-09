use crate::emacs_char;
use crate::engine::class::{
    CHARSET_CLASS_BIT_SPACE, CHARSET_CLASS_BIT_WORD, parse_posix_char_class, posix_class_bit,
    regex_char_to_unibyte, regex_unibyte_to_char,
};
use crate::engine::compile::{RegexCompileError, decode_pattern_char, emacs_char_to_rust_char};
use crate::engine::opcode::RegexOp;
use crate::engine::pattern::CompiledPattern;
use crate::engine::translate::CaseTranslation;

pub(crate) fn compile_charset(
    pattern: &[u8],
    p: &mut usize,
    buf: &mut CompiledPattern,
    pattern_multibyte: bool,
) -> Result<(), RegexCompileError> {
    let plen = pattern.len();
    if *p >= plen {
        return Err(RegexCompileError {
            message: "Unmatched [ or [^".to_string(),
        });
    }

    // Check for negation
    let negate = *p < plen && pattern[*p] == b'^';
    if negate {
        *p += 1;
        if *p >= plen {
            return Err(RegexCompileError {
                message: "Unmatched [ or [^".to_string(),
            });
        }
    }

    let op = if negate {
        RegexOp::CharsetNot
    } else {
        RegexOp::Charset
    };

    // Record the bytecode position of this charset opcode for the
    // multibyte_charsets map.
    let charset_opcode_pos = buf.buffer.len();
    buf.buffer.push(op as u8);
    let _bitmap_len_pos = buf.buffer.len();
    buf.buffer.push(32); // 256 bits = 32 bytes bitmap

    // Initialize 32-byte bitmap (256 bits, one per ASCII char)
    let bitmap_start = buf.buffer.len();
    buf.buffer.extend_from_slice(&[0u8; 32]);

    // Collect multibyte (non-ASCII) ranges for this charset.
    let mut mb_ranges: Vec<(char, char)> = Vec::new();

    // Bitmask of class flags for `[[:word:]]` / `[[:space:]]`. The
    // matcher checks these against the buffer syntax table at run
    // time so per-mode word/space definitions take effect.
    let mut class_bits: u32 = 0;

    // Special case: ] at start is literal
    let mut first = true;
    let mut pending_char: Option<u32> = None;
    let mut closed = false;

    while *p < plen {
        // GNU `re_wctype_parse`: before reading a character, check if
        // we're at `[:name:]` — a POSIX named character class inside
        // the bracket expression.  The `]` in the closing `:]` must
        // not close the outer `[...]`.
        if let Some(cc) = parse_posix_char_class(pattern, *p, plen) {
            if let Some(c) = pending_char.take() {
                add_charset_char(
                    &mut buf.buffer,
                    bitmap_start,
                    c,
                    &mut mb_ranges,
                    pattern_multibyte,
                    buf.translate.as_ref(),
                );
            }
            *p += cc.byte_len;
            let Some(class_name) = cc.kind.name() else {
                return Err(RegexCompileError {
                    message: "Invalid character class name".to_string(),
                });
            };
            apply_posix_class(
                class_name,
                &mut buf.buffer,
                bitmap_start,
                &mut mb_ranges,
                &mut class_bits,
                buf.translate.as_ref(),
            )?;
            if *p >= plen {
                return Err(RegexCompileError {
                    message: "Unmatched [ or [^".to_string(),
                });
            }
            // Mark that we've consumed a character (prevents `]` from
            // being treated as literal at position 0).
            first = false;
            pending_char = None;
            continue;
        }

        let b = pattern[*p];

        // Decode a full Emacs character from the pattern.
        let (c, clen) =
            decode_pattern_char(pattern, *p, pattern_multibyte).unwrap_or((b as u32, 1));
        *p += clen;

        if b == b']' && !first {
            closed = true;
            break;
        }
        first = false;

        // GNU `regex-emacs.c`: a `-' that begins a range (a range-start char is
        // pending) with no following range-end character is a premature end of
        // the pattern -- e.g. `[a-` -- signalled as "Premature end of regular
        // expression", not an unmatched-bracket error.
        if b == b'-' && pending_char.is_some() && *p >= plen {
            return Err(RegexCompileError {
                message: "Premature end of regular expression".to_string(),
            });
        }

        if b == b'-' && *p < plen && pattern[*p] != b']' {
            if let Some(range_start) = pending_char.take() {
                // Range: range_start - next_char
                let (range_end, rlen) = decode_pattern_char(pattern, *p, pattern_multibyte)
                    .unwrap_or((pattern[*p] as u32, 1));
                *p += rlen;
                let translate = buf.translate.as_ref();
                add_charset_range(
                    &mut buf.buffer,
                    bitmap_start,
                    &mut mb_ranges,
                    range_start,
                    range_end,
                    pattern_multibyte,
                    translate,
                );
                continue;
            }
            // '-' at start or after a range → literal '-'
            pending_char = Some('-' as u32);
            continue;
        }

        // GNU `regex-emacs.c` treats backslash as a literal character
        // inside a bracket expression: the parser at lines 2055-2140
        // has no escape handling in the `[...]` loop, so `[\w]` is
        // the character class containing `\` and `w`, and `\n` is
        // the class containing `\` and `n`. Users who want a word
        // character class inside a bracket expression must use the
        // POSIX class `[[:word:]]`.
        if b == b'\\' {
            if let Some(c) = pending_char.take() {
                add_charset_char(
                    &mut buf.buffer,
                    bitmap_start,
                    c,
                    &mut mb_ranges,
                    pattern_multibyte,
                    buf.translate.as_ref(),
                );
            }
            pending_char = Some('\\' as u32);
            continue;
        }

        // POSIX named character classes (`[:alpha:]`, etc.) are now
        // handled by `parse_posix_char_class` at the top of the
        // loop, before the per-character decode.  That function
        // does not advance `p` when the `[:` prefix is a false
        // start, so a literal `[` inside a bracket expression
        // (e.g. `[[`, `[a[:c]`) is treated correctly — it falls
        // through to the character-level processing below.
        //
        // The previous inline handler that used to live here
        // unconditionally advanced `p` past the pattern end when
        // `[:` was not followed by a valid class name, causing
        // spurious "Unmatched [" errors.

        if let Some(prev) = pending_char.take() {
            add_charset_char(
                &mut buf.buffer,
                bitmap_start,
                prev,
                &mut mb_ranges,
                pattern_multibyte,
                buf.translate.as_ref(),
            );
        }
        pending_char = Some(c);
    }

    if !closed {
        return Err(RegexCompileError {
            message: "Unmatched [ or [^".to_string(),
        });
    }

    if let Some(c) = pending_char.take() {
        add_charset_char(
            &mut buf.buffer,
            bitmap_start,
            c,
            &mut mb_ranges,
            pattern_multibyte,
            buf.translate.as_ref(),
        );
    }

    // Store multibyte ranges if any were collected.
    if !mb_ranges.is_empty() {
        buf.multibyte_charsets.insert(charset_opcode_pos, mb_ranges);
    }

    // Record class flags so the matcher can consult the buffer
    // syntax table at run time for `[[:word:]]` and `[[:space:]]`.
    if class_bits != 0 {
        buf.uses_syntax = true;
        // GNU regex-emacs.c:2096-2101: only RECC_SPACE and RECC_WORD
        // hardcode syntax-table content into the compiled pattern (for
        // us: into the fastmap), so only they force the compile cache
        // to key this pattern by syntax table (`used_syntax`).
        if class_bits & (CHARSET_CLASS_BIT_WORD | CHARSET_CLASS_BIT_SPACE) != 0 {
            buf.used_syntax = true;
        }
        buf.charset_class_bits
            .insert(charset_opcode_pos, class_bits);
    }

    Ok(())
}

fn add_charset_char(
    buffer: &mut [u8],
    bitmap_start: usize,
    c: u32,
    mb_ranges: &mut Vec<(char, char)>,
    pattern_multibyte: bool,
    translate: Option<&CaseTranslation>,
) {
    if c < 0x80 {
        set_bitmap_bit(buffer, bitmap_start, c as u8, translate);
    } else if emacs_char::char_byte8_p(c) {
        set_bitmap_raw_bit(buffer, bitmap_start, emacs_char::char_to_byte8(c));
    } else if !pattern_multibyte {
        add_charset_range(
            buffer,
            bitmap_start,
            mb_ranges,
            c,
            c,
            pattern_multibyte,
            translate,
        );
    } else {
        let ch = emacs_char_to_rust_char(c);
        add_multibyte_range(buffer, bitmap_start, mb_ranges, ch, ch, translate);
    }
}

fn add_charset_range(
    buffer: &mut [u8],
    bitmap_start: usize,
    mb_ranges: &mut Vec<(char, char)>,
    mut start: u32,
    end: u32,
    pattern_multibyte: bool,
    translate: Option<&CaseTranslation>,
) {
    if emacs_char::char_byte8_p(end) && start >= 0x80 && !emacs_char::char_byte8_p(start) {
        return;
    }

    if start > end {
        return;
    }

    if start < 0x80 {
        let ascii_end = end.min(0x7f);
        for ch in start..=ascii_end {
            set_bitmap_bit(buffer, bitmap_start, ch as u8, translate);
        }
        start = ascii_end + 1;
        if emacs_char::char_byte8_p(end) {
            start = emacs_char::unibyte_to_char(0x80);
        }
    }

    if start > end {
        return;
    }

    if emacs_char::char_byte8_p(start) {
        let start_byte = emacs_char::char_to_byte8(start);
        let end_byte = emacs_char::char_to_byte8(end);
        for byte in start_byte..=end_byte {
            set_bitmap_raw_bit(buffer, bitmap_start, byte);
        }
    } else if pattern_multibyte {
        let start_ch = emacs_char_to_rust_char(start);
        let end_ch = emacs_char_to_rust_char(end);
        add_multibyte_range(buffer, bitmap_start, mb_ranges, start_ch, end_ch, translate);
    } else {
        if start > 0xFF || end > 0xFF {
            return;
        }
        let start_byte = start as u8;
        let end_byte = end as u8;
        for byte in start_byte..=end_byte {
            let c = regex_unibyte_to_char(byte);
            if emacs_char::char_byte8_p(c) {
                set_bitmap_raw_bit(buffer, bitmap_start, byte);
            } else {
                let translated = translate.map_or(c, |table| table.translate(c));
                if let Some(translated_byte) = regex_char_to_unibyte(translated) {
                    set_bitmap_raw_bit(buffer, bitmap_start, translated_byte);
                } else {
                    set_bitmap_raw_bit(buffer, bitmap_start, byte);
                }
                if let Some(ch) = char::from_u32(translated) {
                    push_or_merge_multibyte_range(mb_ranges, ch);
                }
            }
        }
    }
}

fn push_or_merge_multibyte_range(ranges: &mut Vec<(char, char)>, ch: char) {
    for (lo, hi) in ranges.iter_mut().rev() {
        let ch_u = ch as u32;
        let lo_u = *lo as u32;
        let hi_u = *hi as u32;
        if ch_u >= lo_u.saturating_sub(1) && ch_u <= hi_u.saturating_add(1) {
            if ch_u < lo_u {
                *lo = ch;
            } else if ch_u > hi_u {
                *hi = ch;
            }
            return;
        }
    }
    ranges.push((ch, ch));
}

/// Add a multibyte character range and its case-canonical image.
///
/// Mirrors GNU `SETUP_MULTIBYTE_RANGE`: the original `[FROM, TO]` is stored
/// first, then every character in that range is translated through the active
/// case table.  Translated multibyte characters outside the original range are
/// merged into the range table; translated unibyte characters are reflected in
/// the bitmap.
fn add_multibyte_range(
    buffer: &mut [u8],
    bitmap_start: usize,
    ranges: &mut Vec<(char, char)>,
    start: char,
    end: char,
    translate: Option<&CaseTranslation>,
) {
    ranges.push((start, end));
    let Some(translate) = translate else {
        return;
    };

    let start_u = start as u32;
    let end_u = end as u32;
    for code in start_u..=end_u {
        let translated = translate.translate(code);
        if let Some(byte) = regex_char_to_unibyte(translated) {
            set_bitmap_raw_bit(buffer, bitmap_start, byte);
        }
        if translated >= start_u && translated <= end_u {
            continue;
        }
        if let Some(ch) = char::from_u32(translated) {
            push_or_merge_multibyte_range(ranges, ch);
        }
    }
}

/// Set a bit in the charset bitmap, translating through TRANSLATE if
/// supplied.
///
/// GNU `regex-emacs.c:SETUP_ASCII_RANGE` (lines 1397-1412) runs
/// `C1 = TRANSLATE(C0)` and then `SET_LIST_BIT(C1)` — it translates
/// each individual character as the range is walked and only stores
/// the translated bit. The matcher at regex-emacs.c:4553 does the
/// same TRANSLATE on the input character before the bitmap lookup,
/// so matches work out for any case-equivalent input.
///
/// Earlier versions of this function instead set the bit for both
/// the raw character and its Rust-derived upper/lower partners,
/// regardless of what translate table the pattern was compiled with.
/// That was audit finding #9 in `drafts/regex-search-audit.md`:
/// "charset case-fold range translation is eager (not lazy)". The
/// practical difference only shows up when Rust's Unicode case
/// mapping disagrees with Emacs's case canon table, but the GNU-
/// parity fix is to consult the same translate table both sides.
fn set_bitmap_bit(
    buffer: &mut [u8],
    bitmap_start: usize,
    c: u8,
    translate: Option<&CaseTranslation>,
) {
    let target = match translate {
        Some(table) => table.translate_byte(c),
        None => c,
    };
    set_bitmap_raw_bit(buffer, bitmap_start, target);
}

fn set_bitmap_raw_bit(buffer: &mut [u8], bitmap_start: usize, c: u8) {
    let byte_idx = bitmap_start + (c as usize / 8);
    let bit_idx = c as usize % 8;
    if byte_idx < buffer.len() {
        buffer[byte_idx] |= 1 << bit_idx;
    }
}

/// Apply a POSIX character class to the bitmap and multibyte range list.
///
/// Mirrors GNU `regex-emacs.c:re_wctype_parse` (lines 1525-1601) and
/// `re_iswctype` (lines 1603-1630). The full set of 17 classes is:
/// `alnum`, `alpha`, `blank`, `cntrl`, `digit`, `graph`, `lower`,
/// `print`, `punct`, `space`, `upper`, `xdigit`, `ascii`, `word`,
/// `nonascii`, `unibyte`, `multibyte`.
///
/// Semantics are taken from GNU's header macros at `regex-emacs.c:98-153`:
///
/// - `IS_REAL_ASCII(c)` is `c < 0x80`.
/// - `ISBLANK(c)` for ASCII is `c == ' ' || c == '\t'` only
///   (space and tab; NOT newline, formfeed, carriage return).
/// - `ISSPACE(c)` is `BUFFER_SYNTAX(c) == Swhitespace`; GNU's default
///   standard syntax table treats space, tab, newline, formfeed, and
///   carriage return as whitespace.
/// - `ISGRAPH(c)` for single-byte is `c > ' '` AND NOT in
///   `[0x7F..=0xA0]`.
/// - `ISPRINT(c)` for single-byte is `c >= ' '` AND NOT in
///   `[0x7F..=0x9F]`.
/// - `ISWORD(c)` is `BUFFER_SYNTAX(c) == Sword`; GNU's default treats
///   ASCII letters and digits as word constituents.
/// - `IS_REAL_ASCII(c)` covers 0x00..=0x7F for `ascii`.
/// - `nonascii` = `!IS_REAL_ASCII(c)` (>= 0x80).
/// - `unibyte` matches any single-byte character (bytes 0x00..=0xFF
///   in the bitmap, plus 8-bit raw byte chars).
/// - `multibyte` = `!ISUNIBYTE(c)`; matches multibyte characters
///   only (non-ASCII range via the multibyte range list).
///
/// Unknown class names mirror GNU's `RECC_ERROR` (regex-emacs.c:1600,
/// consumed as `REG_ECTYPE` at line 2071). We signal the same error
/// rather than silently ignoring the class as before.
///
/// Note: `word` and `space` semantically depend on the buffer's
/// syntax table (see audit finding #8 in
/// `drafts/regex-search-audit.md`). For now we bake in the standard
/// default; threading the per-buffer syntax table through charset
/// compilation is tracked as audit #8.
fn apply_posix_class(
    name: &str,
    buffer: &mut [u8],
    bitmap_start: usize,
    mb_ranges: &mut Vec<(char, char)>,
    class_bits: &mut u32,
    translate: Option<&CaseTranslation>,
) -> Result<(), RegexCompileError> {
    *class_bits |= posix_class_bit(name)?;
    // --- ASCII bitmap bits ------------------------------------------------
    //
    // GNU `regex_compile` (regex-emacs.c:2081-2092) sets bitmap (list) bits
    // ONLY for ASCII characters `c < 0x80` where `re_iswctype(c, cc)` is true;
    // the non-ASCII / multibyte side is recorded SOLELY as a range-table bit
    // (`re_wctype_to_bit`) and consulted later by `execute_charset` for chars
    // `c >= 256` (our `class_bits` / `mb_ranges`, the multibyte dispatch path).
    //
    // Therefore the bitmap NEVER contains bits for bytes 0x80..=0xFF.  A raw
    // high byte 0x80..=0xFF in a UNIBYTE target hits the bitmap-only branch of
    // `execute_charset` (`unibyte && c < 256`, regex-emacs.c:3773), where these
    // bits are absent, so it matches NO POSIX class — exactly GNU's behavior
    // (e.g. `[[:nonascii:]]`, `[[:print:]]`, even `[[:unibyte:]]` do NOT match
    // a raw high byte in a unibyte string).  Each arm below enumerates only the
    // ASCII bytes for which `re_iswctype` is true.
    let ascii_bytes: Vec<u8> = match name {
        "alpha" => (b'A'..=b'Z').chain(b'a'..=b'z').collect(),
        "digit" => (b'0'..=b'9').collect(),
        "alnum" => (b'A'..=b'Z')
            .chain(b'a'..=b'z')
            .chain(b'0'..=b'9')
            .collect(),
        // GNU `ISSPACE(c)` is `BUFFER_SYNTAX(c) == Swhitespace`
        // (regex-emacs.c:151): the `[[:space:]]` class consults the ACTIVE
        // syntax table's whitespace class, NOT a fixed isspace set.  GNU builds
        // the ASCII bitmap at compile time from that syntax table; romacs
        // re-derives the syntax-sensitive ASCII membership at MATCH time via
        // `posix_class_matches` (which tests `char_syntax(ch) == Whitespace`),
        // so we contribute NO fixed bitmap bytes here.  Baking in space/tab/
        // LF/CR/FF would make `[[:space:]]` match LF/CR even when the buffer's
        // syntax table classifies them otherwise (e.g. `\n` = comment-end and
        // `\r` = symbol in `emacs-lisp-mode`), which is exactly the GNU
        // divergence this avoids.
        "space" => Vec::new(),
        // GNU ISBLANK is strictly ASCII space and tab.
        "blank" => vec![b' ', b'\t'],
        "upper" => (b'A'..=b'Z').collect(),
        "lower" => (b'a'..=b'z').collect(),
        "punct" => b"!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~".to_vec(),
        // GNU ISPRINT for ASCII (c < 0x80): c >= ' ' and not 0x7F, i.e.
        // 0x20..=0x7E.  The high-byte printable range (0xA0..=0xFF) is NOT a
        // bitmap bit; multibyte printable chars match via `class_bits` in the
        // multibyte path of `execute_charset`.
        "print" => (0x20u8..=0x7E).collect(),
        // GNU ISGRAPH for ASCII (c < 0x80): c > ' ' and not 0x7F, i.e.
        // 0x21..=0x7E.  High bytes go through the multibyte path only.
        "graph" => (0x21u8..=0x7E).collect(),
        // GNU `ISCNTRL(c)` is `((c) < ' ')` (regex-emacs.c:108), i.e. only
        // 0x00..=0x1F.  DEL (0x7F) is NOT a control char for Emacs regexp
        // `[[:cntrl:]]`, unlike the C-locale `iscntrl`.  Including 0x7F here
        // made json.el's `(rx (in cntrl))` escape DEL as `` instead of
        // emitting it literally (matching GNU `json-encode-string`).
        "cntrl" => (0x00u8..=0x1F).collect(),
        "xdigit" => (b'0'..=b'9')
            .chain(b'A'..=b'F')
            .chain(b'a'..=b'f')
            .collect(),
        "ascii" => (0x00u8..=0x7F).collect(),
        // GNU ISWORD(c) = BUFFER_SYNTAX(c) == Sword. Default standard
        // syntax table has ASCII letters and digits as word
        // constituents. Per-buffer syntax tables are audit #8.
        "word" => (b'A'..=b'Z')
            .chain(b'a'..=b'z')
            .chain(b'0'..=b'9')
            .collect(),
        // `re_iswctype(c, RECC_NONASCII)` = `!IS_REAL_ASCII(c)` is FALSE for
        // every ASCII byte, so nonascii sets NO bitmap bits; non-ASCII chars
        // match via the `BIT_MULTIBYTE` range-table bit (the multibyte path).
        "nonascii" => Vec::new(),
        // `re_iswctype(c, RECC_UNIBYTE)` = `ISUNIBYTE(c)` is true for all ASCII
        // bytes (c < 0x80) only at compile time (the loop runs c < 0x80); high
        // bytes 0x80..=0xFF are NOT set, so `[[:unibyte:]]` matches an ASCII
        // byte but NOT a raw high byte in a unibyte string (matching GNU).
        // RECC_UNIBYTE has no range-table bit (`re_wctype_to_bit` returns 0).
        "unibyte" => (0x00u8..=0x7F).collect(),
        // !ISUNIBYTE(c): only multibyte (non-ASCII) characters.
        // Nothing in the bitmap; everything in the multibyte range.
        "multibyte" => Vec::new(),
        // GNU `re_wctype_parse` returns RECC_ERROR (regex-emacs.c:1600)
        // for unknown names; the caller at regex-emacs.c:2071 then
        // signals REG_ECTYPE. We raise the equivalent compile error
        // here rather than silently continuing.
        _ => {
            return Err(RegexCompileError {
                message: format!("Invalid character class name: {}", name),
            });
        }
    };

    // GNU regex-emacs.c:2081-2092 sets the bit for the raw class
    // member AND also the bit for its TRANSLATE-mapped partner when
    // a translate table is in effect. Our set_bitmap_bit always
    // applies the translation (so it already sets the translated
    // bit); we additionally set the raw bit here to cover inputs
    // that match the raw form without going through the translate.
    for c in ascii_bytes {
        // Raw bit (no translation).
        set_bitmap_bit(buffer, bitmap_start, c, None);
        // Translated bit, when a translate table is active. This is
        // a no-op when `translate` is `None` or `translate[c] == c`.
        if translate.is_some() {
            set_bitmap_bit(buffer, bitmap_start, c, translate);
        }
    }

    // --- Multibyte coverage ----------------------------------------------
    //
    // GNU does not expand multibyte POSIX classes into concrete ranges at
    // compile time.  It records class bits and calls `re_iswctype` while
    // executing the charset.  `class_bits` above is the Romacs equivalent.
    // These explicit ranges remain only for the two classes that are pure
    // broad range tests and were represented this way historically.
    match name {
        // Non-ASCII entirely: 0x80..=max Unicode scalar.
        "nonascii" | "multibyte" => {
            mb_ranges.push(('\u{80}', '\u{10FFFF}'));
        }
        _ => {}
    }

    Ok(())
}
