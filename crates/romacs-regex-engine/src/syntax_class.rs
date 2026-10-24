//! GNU syntax classes as the matcher reads them.
//!
//! Moved verbatim from romacs-core's syntax module (which re-exports these),
//! because `\sC`, `\w`, and the syntax-reading assertions compare against
//! them.  GNU reference: `src/syntax.h` (`enum syntaxcode`), `src/syntax.c`
//! (`syntax_spec_code`, `init_syntax_once`).

use num_enum::{IntoPrimitive, TryFromPrimitive};

/// Emacs syntax classes, matching GNU's `enum syntaxcode` from `syntax.h`.
///
/// Discriminant values match the GNU numbering (0–15) so the enum can be
/// cast to `u8` and used directly in bytecode (e.g. the regex engine's
/// `SyntaxSpec` / `NotSyntaxSpec` opcodes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, IntoPrimitive, TryFromPrimitive)]
#[repr(u8)]
pub enum SyntaxClass {
    /// ' ' — Whitespace (Swhitespace = 0)
    Whitespace = 0,
    /// '.' — Punctuation (Spunct = 1)
    Punctuation = 1,
    /// 'w' — Word constituent (Sword = 2)
    Word = 2,
    /// '_' — Symbol constituent (Ssymbol = 3)
    Symbol = 3,
    /// '(' — Open parenthesis/bracket (Sopen = 4)
    Open = 4,
    /// ')' — Close parenthesis/bracket (Sclose = 5)
    Close = 5,
    /// '\'' — Expression prefix (Squote = 6)
    Quote = 6,
    /// '"' — String delimiter (Sstring = 7)
    StringDelim = 7,
    /// '$' — Math delimiter, paired (Smath = 8)
    Math = 8,
    /// '\\' — Escape character (Sescape = 9)
    Escape = 9,
    /// '/' — Character quote, only quotes the next character (Scharquote = 10)
    CharQuote = 10,
    /// '<' — Comment starter (Scomment = 11)
    Comment = 11,
    /// '>' — Comment ender (Sendcomment = 12)
    EndComment = 12,
    /// '@' — Inherit from standard syntax table (Sinherit = 13)
    InheritStd = 13,
    /// '!' — Generic comment delimiter / comment fence (Scomment_fence = 14)
    CommentFence = 14,
    /// '|' — Generic string fence (Sstring_fence = 15)
    StringFence = 15,
}

pub const SYNTAX_CLASS_COUNT: usize = 16;
const SYNTAX_CLASS_DESIGNATORS: [char; SYNTAX_CLASS_COUNT] = [
    ' ', '.', 'w', '_', '(', ')', '\'', '"', '$', '\\', '/', '<', '>', '@', '!', '|',
];

impl SyntaxClass {
    /// Parse a GNU syntax descriptor byte, matching
    /// `src/syntax.c:syntax_spec_code`.
    pub fn from_syntax_spec_byte(byte: u8) -> Option<SyntaxClass> {
        match byte {
            b' ' | b'-' => Some(SyntaxClass::Whitespace),
            b'w' => Some(SyntaxClass::Word),
            b'_' => Some(SyntaxClass::Symbol),
            b'.' => Some(SyntaxClass::Punctuation),
            b'(' => Some(SyntaxClass::Open),
            b')' => Some(SyntaxClass::Close),
            b'\'' => Some(SyntaxClass::Quote),
            b'"' => Some(SyntaxClass::StringDelim),
            b'$' => Some(SyntaxClass::Math),
            b'\\' => Some(SyntaxClass::Escape),
            b'/' => Some(SyntaxClass::CharQuote),
            b'<' => Some(SyntaxClass::Comment),
            b'>' => Some(SyntaxClass::EndComment),
            b'@' => Some(SyntaxClass::InheritStd),
            b'!' => Some(SyntaxClass::CommentFence),
            b'|' => Some(SyntaxClass::StringFence),
            _ => None,
        }
    }

    /// Parse a syntax class from its single-character designator.
    pub fn from_char(ch: char) -> Option<SyntaxClass> {
        let byte = u8::try_from(u32::from(ch)).ok()?;
        SyntaxClass::from_syntax_spec_byte(byte)
    }

    /// Return the canonical single-character designator for this class.
    #[inline]
    pub fn to_char(self) -> char {
        SYNTAX_CLASS_DESIGNATORS[usize::from(u8::from(self))]
    }

    /// Return the integer code Emacs uses for this syntax class
    /// (used in the cons cell returned by `string-to-syntax`).
    #[inline]
    pub fn code(self) -> i64 {
        i64::from(u8::from(self))
    }

    #[inline]
    fn from_gnu_discriminant(code: u8) -> Option<SyntaxClass> {
        SyntaxClass::try_from(code).ok()
    }

    /// Parse a syntax class from a syntax table entry code.
    ///
    /// GNU syntax table entries store flags above the low 8 bits; the class is
    /// extracted with `code & 0377`.
    pub fn from_code(n: i64) -> Option<SyntaxClass> {
        SyntaxClass::from_gnu_discriminant((n & 0xFF) as u8)
    }

    /// Parse a public syntax class integer, as accepted by
    /// `syntax-class-to-char`.  Unlike syntax table entries, GNU rejects raw
    /// integers outside 0..Smax here instead of masking flag bits.
    pub fn from_plain_code(n: i64) -> Option<SyntaxClass> {
        let code = u8::try_from(n).ok()?;
        SyntaxClass::from_gnu_discriminant(code)
    }
}

/// Return the GNU standard syntax-table class for an Emacs character
/// code, mirroring GNU `src/syntax.c:init_syntax_once`.
pub fn standard_syntax_class_for_code(code: u32) -> SyntaxClass {
    match code {
        0x00..=0x08 | 0x0b | 0x0e..=0x1f | 0x7f => SyntaxClass::Punctuation,
        0x09 | 0x0a | 0x0c | 0x0d | 0x20 => SyntaxClass::Whitespace,
        0x30..=0x39 | 0x41..=0x5a | 0x61..=0x7a | 0x24 | 0x25 => SyntaxClass::Word,
        0x28 | 0x5b | 0x7b => SyntaxClass::Open,
        0x29 | 0x5d | 0x7d => SyntaxClass::Close,
        0x22 => SyntaxClass::StringDelim,
        0x5c => SyntaxClass::Escape,
        0x26 | 0x2a | 0x2b | 0x2d | 0x2f | 0x3c | 0x3d | 0x3e | 0x5f | 0x7c => SyntaxClass::Symbol,
        0x21 | 0x23 | 0x27 | 0x2c | 0x2e | 0x3a | 0x3b | 0x3f | 0x40 | 0x5e | 0x60 | 0x7e => {
            SyntaxClass::Punctuation
        }
        0x80..=0x3F_FFFF => SyntaxClass::Word,
        _ => SyntaxClass::Whitespace,
    }
}

#[inline]
pub fn standard_syntax_class_for_char(ch: char) -> SyntaxClass {
    standard_syntax_class_for_code(ch as u32)
}
