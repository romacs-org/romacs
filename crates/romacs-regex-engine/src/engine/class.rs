use crate::emacs_char;
use crate::engine::compile::RegexCompileError;
use crate::engine::syntax::SyntaxLookup;
use crate::engine::translate::CaseTranslation;
use crate::syntax_class::SyntaxClass;

const CHARSET_CLASS_BIT_ALNUM: u32 = 1 << 0;
const CHARSET_CLASS_BIT_ALPHA: u32 = 1 << 1;
const CHARSET_CLASS_BIT_BLANK: u32 = 1 << 2;
const CHARSET_CLASS_BIT_CNTRL: u32 = 1 << 3;
const CHARSET_CLASS_BIT_DIGIT: u32 = 1 << 4;
const CHARSET_CLASS_BIT_GRAPH: u32 = 1 << 5;
const CHARSET_CLASS_BIT_LOWER: u32 = 1 << 6;
const CHARSET_CLASS_BIT_PRINT: u32 = 1 << 7;
const CHARSET_CLASS_BIT_PUNCT: u32 = 1 << 8;
pub(crate) const CHARSET_CLASS_BIT_SPACE: u32 = 1 << 9;
const CHARSET_CLASS_BIT_UPPER: u32 = 1 << 10;
const CHARSET_CLASS_BIT_XDIGIT: u32 = 1 << 11;
const CHARSET_CLASS_BIT_ASCII: u32 = 1 << 12;
pub(crate) const CHARSET_CLASS_BIT_WORD: u32 = 1 << 13;
const CHARSET_CLASS_BIT_NONASCII: u32 = 1 << 14;
const CHARSET_CLASS_BIT_UNIBYTE: u32 = 1 << 15;
const CHARSET_CLASS_BIT_MULTIBYTE: u32 = 1 << 16;

/// Compile a character class `[...]` into charset bytecode.
/// POSIX named character class kind, returned by `parse_posix_char_class`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PosixCharClassKind {
    Word,
    Space,
    Upper,
    Lower,
    Alpha,
    Alnum,
    Digit,
    Xdigit,
    Punct,
    Graph,
    Print,
    Blank,
    Cntrl,
    Ascii,
    Unibyte,
    NonAscii,
    Multibyte,
    /// `[:` was found but the name is not a valid class.
    Error,
}

impl PosixCharClassKind {
    pub(crate) fn name(self) -> Option<&'static str> {
        match self {
            Self::Word => Some("word"),
            Self::Space => Some("space"),
            Self::Upper => Some("upper"),
            Self::Lower => Some("lower"),
            Self::Alpha => Some("alpha"),
            Self::Alnum => Some("alnum"),
            Self::Digit => Some("digit"),
            Self::Xdigit => Some("xdigit"),
            Self::Punct => Some("punct"),
            Self::Graph => Some("graph"),
            Self::Print => Some("print"),
            Self::Blank => Some("blank"),
            Self::Cntrl => Some("cntrl"),
            Self::Ascii => Some("ascii"),
            Self::Unibyte => Some("unibyte"),
            Self::NonAscii => Some("nonascii"),
            Self::Multibyte => Some("multibyte"),
            Self::Error => None,
        }
    }
}

pub(crate) struct PosixCharClass {
    pub(crate) kind: PosixCharClassKind,
    pub(crate) byte_len: usize,
}

/// Parse a POSIX named character class `[:name:]` at `pattern[pos..]`.
/// Returns `None` if `pattern[pos..]` doesn't start with `[:`.
pub(crate) fn parse_posix_char_class(
    pattern: &[u8],
    pos: usize,
    plen: usize,
) -> Option<PosixCharClass> {
    if pos + 4 > plen {
        return None;
    }
    if pattern[pos] != b'[' || pattern[pos + 1] != b':' {
        return None;
    }
    // Find closing ":]"
    let name_start = pos + 2;
    let mut end = name_start;
    while end + 1 < plen {
        if pattern[end] == b':' && pattern[end + 1] == b']' {
            break;
        }
        end += 1;
    }
    if end + 1 >= plen {
        // No closing ":]" found — treat "[:" as literal characters
        return None;
    }
    let name = &pattern[name_start..end];
    let kind = match name {
        b"word" => PosixCharClassKind::Word,
        b"alnum" => PosixCharClassKind::Alnum,
        b"alpha" => PosixCharClassKind::Alpha,
        b"space" => PosixCharClassKind::Space,
        b"digit" => PosixCharClassKind::Digit,
        b"blank" => PosixCharClassKind::Blank,
        b"upper" => PosixCharClassKind::Upper,
        b"lower" => PosixCharClassKind::Lower,
        b"punct" => PosixCharClassKind::Punct,
        b"ascii" => PosixCharClassKind::Ascii,
        b"graph" => PosixCharClassKind::Graph,
        b"print" => PosixCharClassKind::Print,
        b"cntrl" => PosixCharClassKind::Cntrl,
        b"xdigit" => PosixCharClassKind::Xdigit,
        b"unibyte" => PosixCharClassKind::Unibyte,
        b"nonascii" => PosixCharClassKind::NonAscii,
        b"multibyte" => PosixCharClassKind::Multibyte,
        _ => PosixCharClassKind::Error,
    };
    Some(PosixCharClass {
        kind,
        byte_len: end + 2 - pos, // include ":]" closing
    })
}

pub(crate) fn posix_class_bit(name: &str) -> Result<u32, RegexCompileError> {
    match name {
        "alnum" => Ok(CHARSET_CLASS_BIT_ALNUM),
        "alpha" => Ok(CHARSET_CLASS_BIT_ALPHA),
        "blank" => Ok(CHARSET_CLASS_BIT_BLANK),
        "cntrl" => Ok(CHARSET_CLASS_BIT_CNTRL),
        "digit" => Ok(CHARSET_CLASS_BIT_DIGIT),
        "graph" => Ok(CHARSET_CLASS_BIT_GRAPH),
        "lower" => Ok(CHARSET_CLASS_BIT_LOWER),
        "print" => Ok(CHARSET_CLASS_BIT_PRINT),
        "punct" => Ok(CHARSET_CLASS_BIT_PUNCT),
        "space" => Ok(CHARSET_CLASS_BIT_SPACE),
        "upper" => Ok(CHARSET_CLASS_BIT_UPPER),
        "xdigit" => Ok(CHARSET_CLASS_BIT_XDIGIT),
        "ascii" => Ok(CHARSET_CLASS_BIT_ASCII),
        "word" => Ok(CHARSET_CLASS_BIT_WORD),
        "nonascii" => Ok(CHARSET_CLASS_BIT_NONASCII),
        "unibyte" => Ok(CHARSET_CLASS_BIT_UNIBYTE),
        "multibyte" => Ok(CHARSET_CLASS_BIT_MULTIBYTE),
        _ => Err(RegexCompileError {
            message: format!("Invalid character class name: {}", name),
        }),
    }
}

fn unicode_blank_char(c: char) -> bool {
    matches!(
        c,
        '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}'
    )
}

fn unicode_line_or_paragraph_separator(c: char) -> bool {
    matches!(c, '\u{2028}' | '\u{2029}')
}

fn unicode_graphic_char(c: char) -> bool {
    !unicode_blank_char(c) && !unicode_line_or_paragraph_separator(c) && !c.is_control()
}

fn unicode_printable_char(c: char) -> bool {
    !c.is_control()
}

/// Emacs character code for a unibyte byte (raw bytes 0x80..=0xFF map to
/// the byte8 range). Mirrors GNU `RE_CHAR_TO_MULTIBYTE`.
pub(crate) fn regex_unibyte_to_char(byte: u8) -> u32 {
    if byte < 0x80 {
        byte as u32
    } else {
        emacs_char::unibyte_to_char(byte)
    }
}

/// Rust scalar used by Unicode predicates, scalar charset ranges and the
/// existing standalone category matcher. Syntax and word-boundary table reads
/// instead retain the full `EmacsChar`.
pub(crate) fn regex_scalar_char(code: u32) -> char {
    if emacs_char::char_byte8_p(code) {
        char::from(emacs_char::char_to_byte8(code))
    } else {
        char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER)
    }
}

/// Unibyte byte for an Emacs character code, if one exists.
/// Mirrors GNU `RE_CHAR_TO_UNIBYTE`.
pub(crate) fn regex_char_to_unibyte(code: u32) -> Option<u8> {
    if code < 0x80 || emacs_char::char_byte8_p(code) {
        Some(emacs_char::char_to_byte8(code))
    } else {
        None
    }
}

fn gnu_alphabetic_char(code: u32) -> bool {
    emacs_char::char_general_category(code).is_some_and(emacs_char::alphabeticp)
}

fn gnu_alphanumeric_char(code: u32) -> bool {
    emacs_char::char_general_category(code).is_some_and(emacs_char::alphanumericp)
}

/// Runtime POSIX character-class membership for charset `class_bits`.
///
/// Mirrors GNU `re_iswctype` (regex-emacs.c) as consulted by
/// `execute_charset` for the range-table class bits.  The syntax-table
/// dependent classes are `word` and `space` (GNU regex-emacs.c:151
/// `ISSPACE` / `ISWORD` route through `SYNTAX (c)`); everything else is
/// a fixed predicate.  Shared by the matcher (per-character class test)
/// and `compile_fastmap` (baking the ASCII members of a leading class
/// into the fastmap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PosixClassCaseMode {
    Sensitive,
    Folded,
}

impl PosixClassCaseMode {
    pub(crate) fn from_translation(translate: Option<&CaseTranslation>) -> Self {
        match translate {
            Some(_) => Self::Folded,
            None => Self::Sensitive,
        }
    }

    fn matches_lower(self, ch: char) -> bool {
        match self {
            Self::Sensitive => ch.is_lowercase(),
            Self::Folded => ch.is_lowercase() || ch.is_uppercase(),
        }
    }

    fn matches_upper(self, ch: char) -> bool {
        match self {
            Self::Sensitive => ch.is_uppercase(),
            Self::Folded => ch.is_uppercase() || ch.is_lowercase(),
        }
    }
}

/// Whether [`posix_class_matches`] can read the syntax table for class bits
/// `bits`: `[:space:]` and `[:word:]` always, `[:punct:]` for non-ASCII.
pub(crate) fn posix_class_bits_read_syntax(bits: u32) -> bool {
    bits & (CHARSET_CLASS_BIT_SPACE | CHARSET_CLASS_BIT_WORD | CHARSET_CLASS_BIT_PUNCT) != 0
}

/// GNU `re_wctype`: the class bit of an ISO C class NAME, for a host that
/// tests characters against classes outside a pattern (`skip-chars-forward`).
pub fn char_class_bit(name: &str) -> Option<u32> {
    posix_class_bit(name).ok()
}

/// GNU `re_iswctype` over a set of class bits: the predicate a pattern's
/// bracket expression uses, reading SYNTAX's table, not its text properties.
pub fn char_in_classes(code: u32, bits: u32, syntax: &dyn SyntaxLookup) -> bool {
    posix_class_matches(code, bits, None, syntax, PosixClassCaseMode::Sensitive)
}

pub(crate) fn posix_class_matches(
    code: u32,
    bits: u32,
    input_pos: Option<usize>,
    syntax: &dyn SyntaxLookup,
    case_mode: PosixClassCaseMode,
) -> bool {
    let byte = regex_char_to_unibyte(code);
    let ch = regex_scalar_char(code);
    let emacs_ch = emacs_char::EmacsChar::from_code_unchecked(code);
    let char_syntax = || {
        input_pos.map_or_else(
            || syntax.emacs_char_syntax(emacs_ch),
            |position| syntax.emacs_char_syntax_at(emacs_ch, position),
        )
    };
    let is_real_ascii = code < 0x80;
    let ascii_alnum = |b: u8| b.is_ascii_alphabetic() || b.is_ascii_digit();
    let ascii_alpha = |b: u8| b.is_ascii_alphabetic();

    (bits & CHARSET_CLASS_BIT_ALNUM != 0
        && if is_real_ascii {
            ascii_alnum(code as u8)
        } else {
            gnu_alphanumeric_char(code)
        })
        || (bits & CHARSET_CLASS_BIT_ALPHA != 0
            && if is_real_ascii {
                ascii_alpha(code as u8)
            } else {
                gnu_alphabetic_char(code)
            })
        || (bits & CHARSET_CLASS_BIT_BLANK != 0
            && if is_real_ascii {
                matches!(code, 0x20 | 0x09)
            } else {
                unicode_blank_char(ch)
            })
        || (bits & CHARSET_CLASS_BIT_CNTRL != 0 && code < 0x20)
        || (bits & CHARSET_CLASS_BIT_DIGIT != 0 && is_real_ascii && (code as u8).is_ascii_digit())
        || (bits & CHARSET_CLASS_BIT_GRAPH != 0
            && byte.map_or_else(
                || unicode_graphic_char(ch),
                |b| b > b' ' && !(0x7f..=0xa0).contains(&b),
            ))
        || (bits & CHARSET_CLASS_BIT_LOWER != 0 && case_mode.matches_lower(ch))
        || (bits & CHARSET_CLASS_BIT_PRINT != 0
            && byte.map_or_else(
                || unicode_printable_char(ch),
                |b| b >= b' ' && !(0x7f..=0x9f).contains(&b),
            ))
        || (bits & CHARSET_CLASS_BIT_PUNCT != 0
            && if is_real_ascii {
                let b = code as u8;
                b > b' ' && b < 0x7f && !ascii_alnum(b)
            } else {
                char_syntax() != SyntaxClass::Word
            })
        || (bits & CHARSET_CLASS_BIT_SPACE != 0 && char_syntax() == SyntaxClass::Whitespace)
        || (bits & CHARSET_CLASS_BIT_UPPER != 0 && case_mode.matches_upper(ch))
        || (bits & CHARSET_CLASS_BIT_XDIGIT != 0
            && is_real_ascii
            && (code as u8).is_ascii_hexdigit())
        || (bits & CHARSET_CLASS_BIT_ASCII != 0 && is_real_ascii)
        || (bits & CHARSET_CLASS_BIT_WORD != 0 && char_syntax() == SyntaxClass::Word)
        || (bits & CHARSET_CLASS_BIT_NONASCII != 0 && !is_real_ascii)
        || (bits & CHARSET_CLASS_BIT_UNIBYTE != 0 && byte.is_some())
        || (bits & CHARSET_CLASS_BIT_MULTIBYTE != 0 && byte.is_none())
}
