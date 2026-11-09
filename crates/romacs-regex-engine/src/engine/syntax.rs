//! What the matcher asks its host about syntax and categories (`\w`,
//! `\sC`, `\cC`, word and symbol boundaries), and the cache keys that
//! identify a syntax table.
//!
//! GNU: the `SYNTAX` macro (syntax.h) and `CHAR_HAS_CATEGORY` (category.h)
//! as regex-emacs.c reads them, and search.c's `regexp_cache.syntax_table`.

use crate::emacs_char;
use crate::syntax_class::SyntaxClass;

/// Identity of a syntax lookup for the compiled-pattern caches.
///
/// GNU `search.c` keeps this as `regexp_cache.syntax_table` — the actual
/// syntax-table object for `used_syntax` patterns, `Qt` for
/// table-independent ones — and compares it with `BASE_EQ` against the
/// current buffer's table on every cache probe (search.c:222-224).
/// The `epoch` component stands in for GNU's `clear_regexp_cache` call
/// on `modify-syntax-entry`: bumping it strands every entry baked
/// against the old table contents.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SyntaxCacheKey {
    /// The immutable built-in standard mapping (`DefaultSyntaxLookup`).
    Standard,
    /// A host syntax table: its identity (romacs-core: the char-table's
    /// Value bits, which core's pattern caches root) + the host's
    /// syntax-table mutation epoch.  Opaque to the engine.
    Table { id: usize, epoch: u64 },
    /// Identity supplied by a Rust-owned synthetic lookup, with no Lisp object.
    #[cfg(any(test, feature = "test-hooks"))]
    External { id: usize, epoch: u64 },
}

pub trait SyntaxLookup {
    /// Return the syntax class of character `c` in the current syntax table.
    fn char_syntax(&self, c: char) -> SyntaxClass;

    /// Return the syntax class at byte `input_pos` in the matcher input.
    ///
    /// String matching and compile-time fastmaps use the position-independent
    /// default.  Buffer matching overrides this to honor `syntax-table` text
    /// properties at the corresponding absolute buffer byte.
    fn char_syntax_at(&self, c: char, input_pos: usize) -> SyntaxClass {
        let _ = input_pos;
        self.char_syntax(c)
    }

    /// Syntax of a full Emacs character. Byte8 keys must never be reduced to
    /// their corresponding Latin-1 scalar before indexing a syntax table.
    fn emacs_char_syntax(&self, c: emacs_char::EmacsChar) -> SyntaxClass {
        c.as_rust_char().map_or_else(
            || crate::standard_syntax_class_for_code(c.code()),
            |c| self.char_syntax(c),
        )
    }

    /// Position-aware syntax retaining byte8/non-Unicode table keys.
    fn emacs_char_syntax_at(&self, c: emacs_char::EmacsChar, input_pos: usize) -> SyntaxClass {
        c.as_rust_char().map_or_else(
            || self.emacs_char_syntax(c),
            |c| self.char_syntax_at(c, input_pos),
        )
    }

    fn emacs_word_boundary_between(
        &self,
        c1: emacs_char::EmacsChar,
        c2: emacs_char::EmacsChar,
    ) -> bool {
        match (c1.as_rust_char(), c2.as_rust_char()) {
            (Some(c1), Some(c2)) => self.word_boundary_between(c1, c2),
            _ => false,
        }
    }

    /// Return true if character `c` belongs to category `cat`.
    fn char_has_category(&self, c: char, cat: u8) -> bool;

    /// Return true when two adjacent word constituents have a GNU word
    /// boundary between them because of their scripts/categories.
    fn word_boundary_between(&self, _c1: char, _c2: char) -> bool {
        false
    }

    /// Cache identity of this lookup (see [`SyntaxCacheKey`]).  Used by
    /// the front-end pattern caches to key `used_syntax` entries by
    /// syntax table, mirroring GNU `compile_pattern`.
    fn cache_key(&self) -> SyntaxCacheKey;

    /// Identity of the tables behind [`Self::char_syntax`] and
    /// [`Self::char_has_category`], for caches of per-character results (the
    /// existence DFA's classes), which stay valid while this identity and the
    /// char-table write tick hold.  `None`: no such identity; do not cache.
    fn class_cache_key(&self) -> Option<LookupClassKey> {
        None
    }

    /// A counter that moves whenever the tables behind
    /// [`Self::class_cache_key`] may have changed (romacs-core: the global
    /// char-table write tick).  The existence DFA's per-character class memo
    /// is valid while both the key and this tick hold.  Required, so that no
    /// lookup with mutable tables can silently answer a constant.
    fn table_write_tick(&self) -> u64;

    /// Whether [`Self::char_syntax_at`] can answer differently from
    /// [`Self::char_syntax`] somewhere (`syntax-table` text properties).
    fn position_dependent(&self) -> bool {
        true
    }

    /// The end of the stretch of input positions from `pos` over which
    /// [`Self::char_syntax_at`] answers [`Self::char_syntax`] for every
    /// character (no `syntax-table` property applies there), or `pos` itself
    /// when one may apply at `pos`.  `usize::MAX` means nowhere past `pos`.
    ///
    /// The default is exact for any lookup: a position-independent one is
    /// plain everywhere, and a position-dependent one that cannot say where
    /// its properties lie is treated as propertized at every position.
    fn plain_syntax_until(&self, pos: usize) -> usize {
        if self.position_dependent() {
            pos
        } else {
            usize::MAX
        }
    }

    /// The first input position at which a syntax read is recorded for lazy
    /// `syntax-propertize` ([`PropertizeFrontier`](super::regex::PropertizeFrontier)),
    /// or `usize::MAX` when reads are not recorded.
    fn syntax_read_limit(&self) -> usize {
        usize::MAX
    }
}

/// The identity of a syntax lookup's tables (see
/// [`SyntaxLookup::class_cache_key`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookupClassKey {
    /// GNU's standard classification and the default categories: constant.
    Standard,
    /// A syntax table and a category table (0: the default categories), by
    /// object identity.
    Tables { syntax: usize, category: usize },
}

/// Default syntax lookup — uses GNU's standard syntax-table definitions.
/// This is used when no buffer-specific syntax table is available
/// (e.g. in unit tests or string-only matching).
pub struct DefaultSyntaxLookup;

impl SyntaxLookup for DefaultSyntaxLookup {
    fn char_syntax(&self, c: char) -> SyntaxClass {
        crate::standard_syntax_class_for_char(c)
    }

    fn char_has_category(&self, c: char, cat: u8) -> bool {
        default_char_has_category(c, cat)
    }

    fn cache_key(&self) -> SyntaxCacheKey {
        // `standard_syntax_class_for_char` is a hardwired mapping — no
        // chartable, no mutation — so entries baked against it stay
        // valid forever (GNU `Qt` for the standard-classification case).
        SyntaxCacheKey::Standard
    }

    fn class_cache_key(&self) -> Option<LookupClassKey> {
        Some(LookupClassKey::Standard)
    }

    fn table_write_tick(&self) -> u64 {
        // The standard classification and default categories are constants.
        0
    }

    fn position_dependent(&self) -> bool {
        false
    }
}

/// Return whether character `c` belongs to the GNU regex category
/// `cat` (`\cX`).
///
/// GNU's category mechanism (`src/category.c`) gives each character
/// a 128-bit set of category memberships, populated at startup
/// time from `lisp/international/characters.el`. We don't ship the
/// full table; instead we hardcode the most common categories using
/// Unicode block ranges. The category mnemonics here are taken
/// directly from `lisp/international/characters.el` (the GNU
/// `(define-category ?X "...")` lines starting at line 37).
///
/// Audit finding #6 in `drafts/regex-search-audit.md` flagged that
/// only `\c|` worked. This implementation covers the categories the
/// CJK font-lock and bidi paths actually use.
pub fn default_char_has_category(c: char, cat: u8) -> bool {
    let cp = c as u32;
    match cat {
        // |  -- "line breakable". GNU's `characters.el` adds this
        // for most CJK and fullwidth ranges; we use the practical
        // shortcut of "any non-ASCII char" which is what romacs
        // historically returned.
        b'|' => !c.is_ascii(),

        // a  -- ASCII. GNU `lisp/international/characters.el` assigns
        // category `a` to codepoints 32..127, not ASCII controls.
        b'a' => (0x20..=0x7f).contains(&cp),

        // A  -- 2-byte alnum. GNU populates this from CJK Latin /
        // fullwidth ASCII ranges. The practical shortcut is the
        // fullwidth ASCII alphanumeric block.
        b'A' => matches!(cp, 0xFF10..=0xFF19 | 0xFF21..=0xFF3A | 0xFF41..=0xFF5A),

        // l  -- Latin (a-z, A-Z and Latin-1/Extended letters).
        // r  -- Roman (Japanese context, same effective range).
        b'l' | b'r' => {
            c.is_ascii_alphabetic()
                || matches!(cp, 0x00C0..=0x00FF | 0x0100..=0x024F | 0x1E00..=0x1EFF)
        }

        // g  -- Greek (Greek and Coptic block).
        b'g' => matches!(cp, 0x0370..=0x03FF | 0x1F00..=0x1FFF),

        // G  -- 2-byte Greek (fullwidth Greek). Rare; use the
        // same practical bounds as `g` for now.
        b'G' => matches!(cp, 0x0370..=0x03FF | 0x1F00..=0x1FFF),

        // y  -- Cyrillic.
        b'y' | b'Y' => matches!(cp, 0x0400..=0x052F),

        // b  -- Arabic.
        b'b' => matches!(cp, 0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF),

        // w  -- Hebrew.
        b'w' => matches!(cp, 0x0590..=0x05FF | 0xFB1D..=0xFB4F),

        // t  -- Thai.
        b't' => matches!(cp, 0x0E00..=0x0E7F),

        // o  -- Lao.
        b'o' => matches!(cp, 0x0E80..=0x0EFF),

        // q  -- Tibetan.
        b'q' => matches!(cp, 0x0F00..=0x0FFF),

        // i  -- Indian (Devanagari + related). GNU's actual table
        // covers more scripts; this is the most common one.
        b'i' => matches!(cp, 0x0900..=0x097F),

        // I  -- Indian glyphs (broader Indic blocks).
        b'I' => matches!(cp, 0x0900..=0x0DFF),

        // e  -- Ethiopic (Ge'ez).
        b'e' => matches!(cp, 0x1200..=0x137F),

        // v  -- Vietnamese (Latin Extended Additional).
        b'v' => matches!(cp, 0x1E00..=0x1EFF),

        // h  -- Korean (Hangul Syllables + Jamo).
        // N  -- 2-byte Korean (same range here).
        b'h' | b'N' => {
            matches!(cp, 0x1100..=0x11FF | 0xAC00..=0xD7A3 | 0xA960..=0xA97F | 0xD7B0..=0xD7FF)
        }

        // c  -- Chinese / Han ideographs (broad).
        // C  -- 2-byte han (slightly narrower set).
        b'c' | b'C' => matches!(
            cp,
            0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xF900..=0xFAFF
                | 0x20000..=0x2FFFF
                | 0x30000..=0x323AF
        ),

        // H  -- Hiragana (Japanese).
        b'H' => matches!(cp, 0x3040..=0x309F | 0x1B000..=0x1B16F),

        // K  -- Katakana (Japanese).
        b'K' => matches!(
            cp,
            0x3099..=0x309C | 0x30A0..=0x30FF | 0x31F0..=0x31FF | 0x1AFF0..=0x1B16F
        ),

        // k  -- Katakana (lowercase mnemonic, same coverage).
        b'k' => matches!(cp, 0x30A0..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F),

        // j  -- Japanese (Hiragana + Katakana + half-width Katakana
        // + CJK punctuation + fullwidth ASCII).
        b'j' => matches!(
            cp,
            0x3000..=0x303F
                | 0x3040..=0x309F
                | 0x30A0..=0x30FF
                | 0xFF00..=0xFFEF
        ),

        // .  -- Base (Unicode L,N,P,S,Zs).
        b'.' => match c.is_ascii() {
            true => c.is_ascii_graphic() || c == ' ',
            false => {
                !matches!(cp, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
            }
        },

        // ^  -- Combining diacritic / mark (Unicode M).
        b'^' => {
            matches!(cp, 0x0300..=0x036F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
        }

        // R  -- Strong R2L (right-to-left). Practical heuristic:
        // Hebrew and Arabic ranges.
        b'R' => matches!(cp, 0x0590..=0x05FF | 0x0600..=0x06FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF),

        // L  -- Strong L2R (everything else).
        b'L' => {
            !matches!(cp, 0x0590..=0x05FF | 0x0600..=0x06FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
        }

        // 6  -- digit (numeric).
        b'6' => c.is_numeric(),

        // Categories we don't recognize fall through as "no
        // membership" — same as GNU's behavior for an unset bit.
        _ => false,
    }
}
