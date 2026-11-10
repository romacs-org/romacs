//! The host-facing side of case folding and of pattern text.
//!
//! The engine never holds a host object.  A case-canon table reaches it as a
//! [`CaseTable`] trait object that the host implements; a pattern reaches it
//! as [`EmacsText`] (bytes plus multibyteness).  Neither trait mentions a
//! Lisp value, so nothing the engine stores can be a stale handle into a
//! moving heap: a host whose table object moves keeps that object in its own
//! implementation and traces it itself (romacs-core recovers its type through
//! [`CaseTable::as_any`] when it enumerates regexp-cache GC roots).

use std::any::Any;

/// A host case-canon table: GNU's `case-canon-table` as read through
/// `RE_TRANSLATE` (regex-emacs.c, `TRANSLATE`).
///
/// [`crate::CaseTranslation`] memoizes what it reads through this table; the
/// memos are invalidated by [`Self::write_tick`], so an implementation whose
/// entries can change in place MUST move the tick on every change.
pub trait CaseTable: std::fmt::Debug {
    /// The canonical (case-folded) form of character code `c`
    /// (GNU `translate_char` on the canon table).
    fn translate(&self, c: u32) -> u32;

    /// A counter that moves whenever this table's entries may have changed.
    /// Memos taken at one tick are discarded when it differs.
    fn write_tick(&self) -> u64;

    /// Whether this table translates some character from 0x80 up INTO the
    /// ASCII range, as of `tick` (which the caller just read from
    /// [`Self::write_tick`]).  Answering may cost a walk of the whole table;
    /// with `allow_walk` false the implementation answers only from what it
    /// already knows at `tick` and returns `None` otherwise.  With
    /// `allow_walk` true it must answer.
    fn folds_into_ascii(&self, tick: u64, allow_walk: bool) -> Option<bool>;

    /// The implementation itself, so the host that created it can recover
    /// its own type (for example to trace a heap object it holds).
    fn as_any(&self) -> &dyn Any;
}

/// Text in Emacs's internal representation: the bytes of a string or buffer
/// span, and whether they are multibyte (GNU's internal UTF-8 superset, see
/// [`crate::emacs_char`]) or unibyte (one raw byte per character).
pub trait EmacsText {
    /// The text's bytes in internal representation.
    fn emacs_bytes(&self) -> &[u8];
    /// Whether [`Self::emacs_bytes`] is multibyte.
    fn is_multibyte(&self) -> bool;
}

/// A borrowed [`EmacsText`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmacsStr<'a> {
    bytes: &'a [u8],
    multibyte: bool,
}

impl<'a> EmacsStr<'a> {
    /// `bytes` as multibyte text.
    pub fn multibyte(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            multibyte: true,
        }
    }

    /// `bytes` as unibyte text.
    pub fn unibyte(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            multibyte: false,
        }
    }

    /// `bytes` with the given multibyteness.
    pub fn new(bytes: &'a [u8], multibyte: bool) -> Self {
        Self { bytes, multibyte }
    }
}

impl EmacsText for EmacsStr<'_> {
    fn emacs_bytes(&self) -> &[u8] {
        self.bytes
    }

    fn is_multibyte(&self) -> bool {
        self.multibyte
    }
}

/// The built-in downcase the matcher's standard canon uses: Unicode
/// lowercase (first char of the mapping), except for the listed codes, which
/// stay themselves.
///
/// Moved verbatim from romacs-core's `downcase_char_code_emacs_compat`,
/// which now delegates here, so the standard canon and Lisp `downcase` agree
/// by construction.
pub fn emacs_downcase_compat(code: i64) -> i64 {
    if preserve_emacs_downcase_payload(code) {
        return code;
    }
    if let Some(c) = u32::try_from(code).ok().and_then(char::from_u32) {
        c.to_lowercase().next().unwrap_or(c) as i64
    } else {
        code
    }
}

fn preserve_emacs_downcase_payload(code: i64) -> bool {
    matches!(
        code,
        304 | 7305
            | 8490
            | 42955
            | 42956
            | 42958
            | 42962
            | 42964
            | 42970
            | 42972
            | 68944..=68965
            | 93856..=93880
    )
}
