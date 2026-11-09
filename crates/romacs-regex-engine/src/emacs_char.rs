//! The Emacs internal character representation the matcher reads.
//!
//! Moved verbatim from romacs-core's `emacs_char` module (which re-exports
//! everything here), so the engine and its host decode text with ONE
//! implementation.  `string_char` decodes a raw-byte lead leniently and
//! `char_string` resolves modifier bits, as the matcher needs.
//!
//! GNU reference: `src/character.h`, `src/character.c`.

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum valid Emacs character code (0x3FFFFF).
pub const MAX_CHAR: u32 = 0x3F_FFFF;

/// Maximum Unicode code point (0x10FFFF).
pub const MAX_UNICODE_CHAR: u32 = 0x10_FFFF;

/// Maximum 1-byte (ASCII) Emacs character code (0x7F).
pub const MAX_1_BYTE_CHAR: u32 = 0x7F;

/// Maximum 2-byte Emacs character code (0x7FF).
pub const MAX_2_BYTE_CHAR: u32 = 0x7FF;

/// Maximum 3-byte Emacs character code (0xFFFF).
pub const MAX_3_BYTE_CHAR: u32 = 0xFFFF;

/// Maximum 4-byte Emacs character code (0x1FFFFF).
pub const MAX_4_BYTE_CHAR: u32 = 0x1F_FFFF;

/// Maximum 5-byte encoded character (0x3FFF7F).
/// Characters above this (0x3FFF80..0x3FFFFF) are raw-byte ("eight-bit") characters.
pub const MAX_5_BYTE_CHAR: u32 = 0x3F_FF7F;

/// Maximum bytes needed to encode any Emacs character in multibyte form.
pub const MAX_MULTIBYTE_LENGTH: usize = 5;

/// Minimum leading byte of a multibyte Emacs character form (0xC0).
pub const MIN_MULTIBYTE_LEADING_CODE: u8 = 0xC0;

/// Maximum leading byte of a multibyte Emacs character form (0xF8).
/// Note: this must be updated if `MAX_CHAR` is ever increased.
pub const MAX_MULTIBYTE_LEADING_CODE: u8 = 0xF8;

/// A validated Emacs character code in the 22-bit range `0..=MAX_CHAR`.
///
/// Issue #131: the historical bug class was treating an Emacs character code as
/// a Rust [`char`] (e.g. `char::from_u32(code)`) or packing it into a Rust
/// `String`. That silently succeeds for codes in the in-Unicode Private-Use
/// area — colliding a real nerd-font glyph with an eight-bit raw byte — and
/// fails outright for eight-bit (`0x3FFF80..`) and non-Unicode (`>0x10FFFF`)
/// codes, which are NOT Unicode scalar values.
///
/// `EmacsChar` makes that mistake un-writable: it is a distinct type from both
/// `char` and a bare `u32`, and the only conversions to/from a Rust `char` or a
/// raw byte are the explicit, correct ones below. The canonical serialisation
/// is the internal multibyte byte form via [`EmacsChar::char_string`] /
/// [`EmacsChar::to_emacs_bytes`] (GNU `CHAR_STRING`), never an in-Unicode
/// sentinel.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EmacsChar(u32);

impl EmacsChar {
    /// Largest valid code (`MAX_CHAR`, 22-bit).
    pub const MAX: u32 = MAX_CHAR;

    /// Wrap a code, returning `None` if it exceeds `MAX_CHAR`.
    #[inline]
    pub fn from_code(code: u32) -> Option<Self> {
        (code <= MAX_CHAR).then_some(Self(code))
    }

    /// Wrap a code without range checking (debug-asserts validity).
    #[inline]
    pub fn from_code_unchecked(code: u32) -> Self {
        debug_assert!(code <= MAX_CHAR, "EmacsChar out of range: {code:#x}");
        Self(code)
    }

    /// The eight-bit raw-byte character for `byte` (`BYTE8_TO_CHAR`).
    #[inline]
    pub fn from_byte8(byte: u8) -> Self {
        Self(byte8_to_char(byte))
    }

    /// A unibyte byte made multibyte: ASCII stays itself, high bytes become
    /// eight-bit raw-byte characters (`UNIBYTE_TO_CHAR`).
    #[inline]
    pub fn from_unibyte_byte(byte: u8) -> Self {
        Self(unibyte_to_char(byte))
    }

    /// A Rust scalar value as an Emacs character (always in range).
    #[inline]
    pub fn from_char(c: char) -> Self {
        Self(c as u32)
    }

    /// The raw 22-bit code.
    #[inline]
    pub fn code(self) -> u32 {
        self.0
    }

    /// Whether this is a Unicode scalar value (representable as a Rust `char`).
    /// False for eight-bit raw bytes and non-Unicode codes.
    #[inline]
    pub fn is_unicode_scalar(self) -> bool {
        char::from_u32(self.0).is_some()
    }

    /// Whether this is an eight-bit raw-byte character (`CHAR_BYTE8_P`).
    #[inline]
    pub fn is_byte8(self) -> bool {
        self.0 > MAX_5_BYTE_CHAR
    }

    /// The underlying byte for an eight-bit (or ASCII) character, else `None`
    /// (`CHAR_TO_BYTE_SAFE`).
    #[inline]
    pub fn to_byte8(self) -> Option<u8> {
        char_to_byte_safe(self.0)
    }

    /// As a Rust `char`, or `None` for eight-bit/non-Unicode codes.
    #[inline]
    pub fn as_rust_char(self) -> Option<char> {
        char::from_u32(self.0)
    }

    /// Encode into `buf` in the canonical internal multibyte form
    /// (`CHAR_STRING`); returns the number of bytes written.
    #[inline]
    pub fn char_string(self, buf: &mut [u8]) -> usize {
        char_string(self.0, buf)
    }

    /// The canonical internal multibyte bytes for this character.
    #[inline]
    pub fn to_emacs_bytes(self) -> Vec<u8> {
        let mut buf = [0u8; MAX_MULTIBYTE_LENGTH];
        let len = char_string(self.0, &mut buf);
        buf[..len].to_vec()
    }
}

/// Start of the raw-byte character range.
pub const BYTE8_OFFSET: u32 = 0x3F_FF00;

// ---------------------------------------------------------------------------
// Character modifier bits (mirrors GNU `lisp.h` `CHAR_ALT` ... `CHAR_META`).
//
// Romacs already defines the same bit values under `KEY_CHAR_*` names in
// `crate::emacs_core::keyboard::pure`. The aliases below give callers in the
// character/encoding layer the GNU spelling without duplicating values, so
// `char_resolve_modifier_mask` and `char_string` can be ported verbatim.
// ---------------------------------------------------------------------------

/// `CHAR_ALT` modifier bit (0x0400000).
pub const CHAR_ALT: u32 = 0x0400000;

/// `CHAR_SUPER` modifier bit (0x0800000).
pub const CHAR_SUPER: u32 = 0x0800000;

/// `CHAR_HYPER` modifier bit (0x1000000).
pub const CHAR_HYPER: u32 = 0x1000000;

/// `CHAR_SHIFT` modifier bit (0x2000000).
pub const CHAR_SHIFT: u32 = 0x2000000;

/// `CHAR_CTL` modifier bit (0x4000000).
pub const CHAR_CTL: u32 = 0x4000000;

/// `CHAR_META` modifier bit (0x8000000).
pub const CHAR_META: u32 = 0x8000000;

/// Bitmask of all character modifier bits.
pub const CHAR_MODIFIER_MASK: u32 =
    CHAR_ALT | CHAR_SUPER | CHAR_HYPER | CHAR_SHIFT | CHAR_CTL | CHAR_META;

/// Resolve modifier bits on character code `c` in the same way GNU does.
///
/// Mirrors `char_resolve_modifier_mask` in GNU `src/character.c:51`:
///
/// * Non-ASCII base characters are returned unchanged.
/// * `S-A`..`S-Z` lose `CHAR_SHIFT`; `S-a`..`S-z` are converted to the
///   corresponding upper-case letter and lose `CHAR_SHIFT`; `S-` on a
///   control character or SPC is dropped.
/// * `C-SPC` becomes `C-@` (NUL) with `CHAR_CTL` cleared; `C-?` becomes
///   DEL (0177) with `CHAR_CTL` cleared; letters and `@`..`_` get masked
///   with 0o37 and lose `CHAR_CTL`.
/// * `CHAR_META` is intentionally left alone (GNU bug#4751).
pub fn char_resolve_modifier_mask(mut c: i64) -> i64 {
    let mask = CHAR_MODIFIER_MASK as i64;
    let base = c & !mask;
    if !(0..=0x7F).contains(&base) {
        return c;
    }

    if c & CHAR_SHIFT as i64 != 0 {
        let low = c & 0o377;
        if (b'A' as i64..=b'Z' as i64).contains(&low) {
            c &= !(CHAR_SHIFT as i64);
        } else if (b'a' as i64..=b'z' as i64).contains(&low) {
            c = (c & !(CHAR_SHIFT as i64)) - (b'a' as i64 - b'A' as i64);
        } else if (c & !mask) <= 0x20 {
            c &= !(CHAR_SHIFT as i64);
        }
    }
    if c & CHAR_CTL as i64 != 0 {
        let low = c & 0o377;
        if low == b' ' as i64 {
            c &= !0o177 & !(CHAR_CTL as i64);
        } else if low == b'?' as i64 {
            c = 0o177 | (c & !0o177 & !(CHAR_CTL as i64));
        } else if (c & 0o137) >= 0o101 && (c & 0o137) <= 0o132
            || (c & 0o177) >= 0o100 && (c & 0o177) <= 0o137
        {
            c &= 0o37 | (!0o177 & !(CHAR_CTL as i64));
        }
    }
    c
}

// ---------------------------------------------------------------------------
// Named Unicode character codes used throughout the codebase
// (mirrors GNU `character.h` `enum`).
// ---------------------------------------------------------------------------

pub const NO_BREAK_SPACE: u32 = 0x00A0;
pub const SOFT_HYPHEN: u32 = 0x00AD;
pub const ZERO_WIDTH_NON_JOINER: u32 = 0x200C;
pub const ZERO_WIDTH_JOINER: u32 = 0x200D;
pub const HYPHEN: u32 = 0x2010;
pub const NON_BREAKING_HYPHEN: u32 = 0x2011;
pub const LEFT_SINGLE_QUOTATION_MARK: u32 = 0x2018;
pub const RIGHT_SINGLE_QUOTATION_MARK: u32 = 0x2019;
pub const PARAGRAPH_SEPARATOR: u32 = 0x2029;
pub const LEFT_POINTING_ANGLE_BRACKET: u32 = 0x2329;
pub const RIGHT_POINTING_ANGLE_BRACKET: u32 = 0x232A;
pub const LEFT_ANGLE_BRACKET: u32 = 0x3008;
pub const RIGHT_ANGLE_BRACKET: u32 = 0x3009;
pub const OBJECT_REPLACEMENT_CHARACTER: u32 = 0xFFFC;
pub const TAG_SPACE: u32 = 0xE0020;
pub const CANCEL_TAG: u32 = 0xE007F;

// ---------------------------------------------------------------------------
// Unicode general category (mirrors GNU `character.h` `enum unicode_category`).
//
// The numeric values match GNU exactly so that lookups in
// `unicode-category-table` (whose values are populated from UnicodeData.txt
// during character.c init) yield comparable results.
// ---------------------------------------------------------------------------

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UnicodeCategory {
    /// Lu - Uppercase Letter
    UppercaseLetter = 0,
    /// Ll - Lowercase Letter
    LowercaseLetter,
    /// Lt - Titlecase Letter
    TitlecaseLetter,
    /// Lm - Modifier Letter
    ModifierLetter,
    /// Lo - Other Letter
    OtherLetter,
    /// Mn - Nonspacing Mark
    NonspacingMark,
    /// Mc - Spacing Mark
    SpacingMark,
    /// Me - Enclosing Mark
    EnclosingMark,
    /// Nd - Decimal Number
    DecimalNumber,
    /// Nl - Letter Number
    LetterNumber,
    /// No - Other Number
    OtherNumber,
    /// Pc - Connector Punctuation
    ConnectorPunctuation,
    /// Pd - Dash Punctuation
    DashPunctuation,
    /// Ps - Open Punctuation
    OpenPunctuation,
    /// Pe - Close Punctuation
    ClosePunctuation,
    /// Pi - Initial Punctuation
    InitialPunctuation,
    /// Pf - Final Punctuation
    FinalPunctuation,
    /// Po - Other Punctuation
    OtherPunctuation,
    /// Sm - Math Symbol
    MathSymbol,
    /// Sc - Currency Symbol
    CurrencySymbol,
    /// Sk - Modifier Symbol
    ModifierSymbol,
    /// So - Other Symbol
    OtherSymbol,
    /// Zs - Space Separator
    SpaceSeparator,
    /// Zl - Line Separator
    LineSeparator,
    /// Zp - Paragraph Separator
    ParagraphSeparator,
    /// Cc - Control
    Control,
    /// Cf - Format
    Format,
    /// Cs - Surrogate
    Surrogate,
    /// Co - Private Use
    PrivateUse,
    /// Cn - Unassigned
    Unassigned,
}

impl From<unicode_general_category::GeneralCategory> for UnicodeCategory {
    fn from(gc: unicode_general_category::GeneralCategory) -> Self {
        use unicode_general_category::GeneralCategory as G;
        match gc {
            G::UppercaseLetter => UnicodeCategory::UppercaseLetter,
            G::LowercaseLetter => UnicodeCategory::LowercaseLetter,
            G::TitlecaseLetter => UnicodeCategory::TitlecaseLetter,
            G::ModifierLetter => UnicodeCategory::ModifierLetter,
            G::OtherLetter => UnicodeCategory::OtherLetter,
            G::NonspacingMark => UnicodeCategory::NonspacingMark,
            G::SpacingMark => UnicodeCategory::SpacingMark,
            G::EnclosingMark => UnicodeCategory::EnclosingMark,
            G::DecimalNumber => UnicodeCategory::DecimalNumber,
            G::LetterNumber => UnicodeCategory::LetterNumber,
            G::OtherNumber => UnicodeCategory::OtherNumber,
            G::ConnectorPunctuation => UnicodeCategory::ConnectorPunctuation,
            G::DashPunctuation => UnicodeCategory::DashPunctuation,
            G::OpenPunctuation => UnicodeCategory::OpenPunctuation,
            G::ClosePunctuation => UnicodeCategory::ClosePunctuation,
            G::InitialPunctuation => UnicodeCategory::InitialPunctuation,
            G::FinalPunctuation => UnicodeCategory::FinalPunctuation,
            G::OtherPunctuation => UnicodeCategory::OtherPunctuation,
            G::MathSymbol => UnicodeCategory::MathSymbol,
            G::CurrencySymbol => UnicodeCategory::CurrencySymbol,
            G::ModifierSymbol => UnicodeCategory::ModifierSymbol,
            G::OtherSymbol => UnicodeCategory::OtherSymbol,
            G::SpaceSeparator => UnicodeCategory::SpaceSeparator,
            G::LineSeparator => UnicodeCategory::LineSeparator,
            G::ParagraphSeparator => UnicodeCategory::ParagraphSeparator,
            G::Control => UnicodeCategory::Control,
            G::Format => UnicodeCategory::Format,
            G::Surrogate => UnicodeCategory::Surrogate,
            G::PrivateUse => UnicodeCategory::PrivateUse,
            G::Unassigned => UnicodeCategory::Unassigned,
            // `GeneralCategory` is marked `#[non_exhaustive]`; treat any
            // future category as Unassigned (the most conservative choice,
            // matching GNU's "not graphic/printable" fallback).
            _ => UnicodeCategory::Unassigned,
        }
    }
}

/// Return the Unicode general category code for codepoint `c`, matching the
/// integer values GNU stores in `unicode-category-table`.
///
/// Returns `None` for codepoints outside the Unicode scalar range (e.g. the
/// eight-bit raw-byte range), mirroring GNU's `CHAR_TABLE_REF` returning nil
/// (a non-fixnum) for which the graphic/printable predicates yield `false`.
pub fn char_general_category(c: u32) -> Option<i64> {
    let ch = char::from_u32(c)?;
    Some(UnicodeCategory::from(unicode_general_category::get_general_category(ch)) as i64)
}

// ---------------------------------------------------------------------------
// Raw-byte predicates and converters
// ---------------------------------------------------------------------------

/// Return `true` if `c` is a raw-byte ("eight-bit") character,
/// i.e. in the range 0x3FFF00..0x3FFFFF.
#[inline]
pub fn char_byte8_p(c: u32) -> bool {
    c > MAX_5_BYTE_CHAR
}

/// Strict conversion of a raw byte to its eight-bit Emacs character code.
///
/// Always returns `byte + 0x3FFF00`, even for ASCII bytes 0x00..0x7F (the
/// result is then in the eight-bit range, *not* an ASCII char). This mirrors
/// GNU `BYTE8_TO_CHAR` exactly.
///
/// For "make this byte a character, treating ASCII as ASCII", use
/// [`unibyte_to_char`] instead.
#[inline]
pub fn byte8_to_char(byte: u8) -> u32 {
    byte as u32 + BYTE8_OFFSET
}

/// Convert a unibyte byte (0x00..0xFF) to its multibyte character code.
///
/// For ASCII bytes (0x00..0x7F) the result is the byte itself; for high
/// bytes (0x80..0xFF) the result is the corresponding eight-bit raw-byte
/// character (`byte + 0x3FFF00`). This mirrors GNU `UNIBYTE_TO_CHAR`.
#[inline]
pub fn unibyte_to_char(byte: u8) -> u32 {
    if byte < 0x80 {
        byte as u32
    } else {
        byte8_to_char(byte)
    }
}

/// If `c` is not ASCII, make it multibyte. Assumes `c < 256`.
///
/// Mirrors GNU `make_char_multibyte` (`character.h`). This is identical to
/// [`unibyte_to_char`] but documents the intent (turning a unibyte char
/// stored in a multibyte buffer into its raw-byte representation).
#[inline]
pub fn make_char_multibyte(c: i32) -> u32 {
    debug_assert!((0..0x100).contains(&c));
    unibyte_to_char(c as u8)
}

/// Convert a raw-byte character code back to its byte value.
///
/// The caller must ensure `c` is a raw-byte character (i.e. `char_byte8_p(c)`
/// is true). For ASCII-range characters this also works (returns low byte).
/// In debug builds, asserts that `c` is either ASCII or a byte8 character to
/// catch silent low-byte truncation of unrelated multibyte characters.
#[inline]
pub fn char_to_byte8(c: u32) -> u8 {
    if char_byte8_p(c) {
        (c - BYTE8_OFFSET) as u8
    } else {
        debug_assert!(
            c < 0x80,
            "char_to_byte8 called on non-byte8 non-ASCII char {:#x}",
            c
        );
        c as u8
    }
}

/// Return the raw 8-bit byte for character `c`, or `None` if `c` doesn't
/// correspond to a single byte.
///
/// Mirrors GNU `CHAR_TO_BYTE_SAFE`. ASCII chars (0..0x7F) return themselves;
/// raw-byte chars (0x3FFF80..0x3FFFFF) return the underlying byte; anything
/// else returns `None`.
#[inline]
pub fn char_to_byte_safe(c: u32) -> Option<u8> {
    if c < 0x80 {
        Some(c as u8)
    } else if char_byte8_p(c) {
        Some((c - BYTE8_OFFSET) as u8)
    } else {
        None
    }
}

/// Return the leading code of the multibyte form of `c`.
///
/// Mirrors GNU `CHAR_LEADING_CODE`.
#[inline]
pub fn char_leading_code(c: u32) -> u8 {
    if c <= MAX_1_BYTE_CHAR {
        c as u8
    } else if c <= MAX_2_BYTE_CHAR {
        0xC0 | (c >> 6) as u8
    } else if c <= MAX_3_BYTE_CHAR {
        0xE0 | (c >> 12) as u8
    } else if c <= MAX_4_BYTE_CHAR {
        0xF0 | (c >> 18) as u8
    } else if c <= MAX_5_BYTE_CHAR {
        0xF8
    } else {
        // Raw byte: overlong 2-byte lead (0xC0 or 0xC1).
        0xC0 | ((c >> 6) & 0x01) as u8
    }
}

/// Encode character `c` into `buf` and return the number of bytes written.
///
/// `buf` must be at least [`MAX_MULTIBYTE_LENGTH`] bytes long.
/// This mirrors GNU `char_string` (character.c:101): if `c` carries any
/// modifier bits they are first resolved by [`char_resolve_modifier_mask`]
/// and then stripped before encoding. `c` (after modifier stripping) must
/// be in `0..=MAX_CHAR`; out-of-range values panic in debug builds.
pub fn char_string(mut c: u32, buf: &mut [u8]) -> usize {
    if c & CHAR_MODIFIER_MASK != 0 {
        let resolved = char_resolve_modifier_mask(c as i64);
        c = (resolved as u32) & !CHAR_MODIFIER_MASK;
    }
    debug_assert!(c <= MAX_CHAR, "char_string: invalid character 0x{:X}", c);
    if c < 0x80 {
        // ASCII
        buf[0] = c as u8;
        1
    } else if c < 0x800 {
        // 2-byte standard UTF-8
        buf[0] = 0xC0 | (c >> 6) as u8;
        buf[1] = 0x80 | (c & 0x3F) as u8;
        2
    } else if c < 0x1_0000 {
        // 3-byte
        buf[0] = 0xE0 | (c >> 12) as u8;
        buf[1] = 0x80 | ((c >> 6) & 0x3F) as u8;
        buf[2] = 0x80 | (c & 0x3F) as u8;
        3
    } else if c < 0x20_0000 {
        // 4-byte
        buf[0] = 0xF0 | (c >> 18) as u8;
        buf[1] = 0x80 | ((c >> 12) & 0x3F) as u8;
        buf[2] = 0x80 | ((c >> 6) & 0x3F) as u8;
        buf[3] = 0x80 | (c & 0x3F) as u8;
        4
    } else if c <= MAX_5_BYTE_CHAR {
        // 5-byte (extended Emacs range, not raw byte)
        buf[0] = 0xF8;
        buf[1] = 0x80 | ((c >> 18) & 0x3F) as u8;
        buf[2] = 0x80 | ((c >> 12) & 0x3F) as u8;
        buf[3] = 0x80 | ((c >> 6) & 0x3F) as u8;
        buf[4] = 0x80 | (c & 0x3F) as u8;
        5
    } else {
        // Raw byte (0x3FFF80..0x3FFFFF) → overlong 2-byte.
        // Raw bytes 0x80..0xFF are encoded as if they were 0x00..0x7F in
        // overlong form:  byte 0x80 → [C0,80], byte 0xBF → [C0,BF],
        //                 byte 0xC0 → [C1,80], byte 0xFF → [C1,BF].
        let val = char_to_byte8(c).wrapping_sub(0x80);
        buf[0] = 0xC0 | (val >> 6);
        buf[1] = 0x80 | (val & 0x3F);
        2
    }
}

/// Return `true` if `lead` is the start of a raw-byte overlong sequence.
///
/// Raw bytes are encoded as `[0xC0, 0x80..0xBF]` or `[0xC1, 0x80..0xBF]`,
/// which are the overlong representations of U+0000..U+007F and U+0040..U+007F
/// respectively. In Emacs encoding these represent raw bytes 0x80..0xFF.
#[inline]
pub fn is_raw_byte_lead(lead: u8) -> bool {
    lead == 0xC0 || lead == 0xC1
}

/// Decode one character from `bytes`, returning `(char_code, bytes_consumed)`.
///
/// This mirrors GNU `STRING_CHAR_AND_LENGTH`. The input must be a valid Emacs
/// multibyte sequence (no bounds checking on continuation bytes beyond what the
/// lead byte promises).
pub fn string_char(bytes: &[u8]) -> (u32, usize) {
    debug_assert!(!bytes.is_empty());
    let b0 = bytes[0];

    if b0 < 0x80 {
        // ASCII
        (b0 as u32, 1)
    } else if is_raw_byte_lead(b0) && bytes.len() >= 2 && (bytes[1] & 0xC0) == 0x80 {
        // Raw-byte overlong: 2 bytes → eight-bit character.
        let raw = ((b0 & 0x01) << 6) | (bytes[1] & 0x3F);
        (byte8_to_char(raw | 0x80), 2)
    } else if b0 < 0xE0 && bytes.len() >= 2 && (bytes[1] & 0xC0) == 0x80 {
        // 2-byte standard UTF-8
        let c = ((b0 as u32 & 0x1F) << 6) | (bytes[1] as u32 & 0x3F);
        (c, 2)
    } else if b0 < 0xF0
        && bytes.len() >= 3
        && (bytes[1] & 0xC0) == 0x80
        && (bytes[2] & 0xC0) == 0x80
    {
        // 3-byte
        let c =
            ((b0 as u32 & 0x0F) << 12) | ((bytes[1] as u32 & 0x3F) << 6) | (bytes[2] as u32 & 0x3F);
        (c, 3)
    } else if b0 < 0xF8
        && bytes.len() >= 4
        && (bytes[1] & 0xC0) == 0x80
        && (bytes[2] & 0xC0) == 0x80
        && (bytes[3] & 0xC0) == 0x80
    {
        // 4-byte
        let c = ((b0 as u32 & 0x07) << 18)
            | ((bytes[1] as u32 & 0x3F) << 12)
            | ((bytes[2] as u32 & 0x3F) << 6)
            | (bytes[3] as u32 & 0x3F);
        (c, 4)
    } else if b0 == 0xF8
        && bytes.len() >= 5
        && (bytes[1] & 0xC0) == 0x80
        && (bytes[2] & 0xC0) == 0x80
        && (bytes[3] & 0xC0) == 0x80
        && (bytes[4] & 0xC0) == 0x80
    {
        // 5-byte (F8 lead, Emacs extension)
        let c = ((bytes[1] as u32 & 0x3F) << 18)
            | ((bytes[2] as u32 & 0x3F) << 12)
            | ((bytes[3] as u32 & 0x3F) << 6)
            | (bytes[4] as u32 & 0x3F);
        (c, 5)
    } else {
        // Invalid or truncated sequence — treat lead byte as raw byte
        (byte8_to_char(b0), 1)
    }
}

#[inline]
pub fn cat_eq(cat: i64, v: UnicodeCategory) -> bool {
    cat == v as i64
}

/// True if `cat` denotes an alphabetic character (UTS #18).
///
/// Mirrors GNU `alphabeticp` (character.c:956): accepts Lu, Ll, Lt,
/// Lm, Lo, Mn, Mc, Me, Nl.
pub fn alphabeticp(cat: i64) -> bool {
    use UnicodeCategory::*;
    cat_eq(cat, UppercaseLetter)
        || cat_eq(cat, LowercaseLetter)
        || cat_eq(cat, TitlecaseLetter)
        || cat_eq(cat, ModifierLetter)
        || cat_eq(cat, OtherLetter)
        || cat_eq(cat, NonspacingMark)
        || cat_eq(cat, SpacingMark)
        || cat_eq(cat, EnclosingMark)
        || cat_eq(cat, LetterNumber)
}

/// True if `cat` denotes an alphabetic-or-decimal character.
///
/// Mirrors GNU `alphanumericp` (character.c:979): adds Nd to the set.
pub fn alphanumericp(cat: i64) -> bool {
    alphabeticp(cat) || cat_eq(cat, UnicodeCategory::DecimalNumber)
}
