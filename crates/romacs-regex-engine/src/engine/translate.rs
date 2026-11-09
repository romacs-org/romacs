//! Case folding: the translation a case-insensitive pattern compiles and
//! matches with.
//!
//! GNU: `RE_TRANSLATE` and `TRANSLATE` in regex-emacs.c, over the buffer's
//! case canon table (casetab.c).

use crate::case_table::{CaseTable, emacs_downcase_compat};
use crate::emacs_char;

#[derive(Clone, Debug)]
pub struct CaseTranslation {
    /// Translations for codes 0..256, filled ON DEMAND when backed by a
    /// char-table (see [`CaseTranslation::from_char_table`]).
    /// [`CASE_TRANSLATION_UNFILLED`] marks a slot not yet computed.
    byte: [std::cell::Cell<u32>; 256],
    /// The host's case-canon table ([`CaseTable`]); `None` is the built-in
    /// standard canonicalization.
    pub(crate) table: Option<std::rc::Rc<dyn CaseTable>>,
    /// Memo for codes from 256 up when backed by a char-table, allocated on
    /// the first such code (see [`WideTranslationMemo`]).
    wide: std::cell::OnceCell<Box<WideTranslationMemo>>,
    /// [`CaseTranslation::ascii_preimage`] of a char-table, and the
    /// char-table write tick it was computed at.
    ascii_preimage_memo: std::cell::Cell<Option<(u64, AsciiPreimage)>>,
}

/// Direct-mapped memo of a char-table translation for codes from 256 up.
///
/// A case-folded search over Cyrillic, Greek or CJK text translates every
/// character it examines; each miss is a `translate_char` char-table walk.
/// A compiled pattern keeps its `CaseTranslation`, so the memo also carries
/// across searches.  The table can be edited in place meanwhile, which the
/// char-table write tick records: a changed tick empties the memo.
#[derive(Clone, Debug)]
struct WideTranslationMemo {
    tick: std::cell::Cell<u64>,
    /// `(code << 32) | translation`, or [`WIDE_TRANSLATION_EMPTY`].
    slots: [std::cell::Cell<u64>; WIDE_TRANSLATION_SLOTS],
}

const WIDE_TRANSLATION_SLOTS: usize = 256;
/// No character code is `u32::MAX`, so no filled slot equals this.
const WIDE_TRANSLATION_EMPTY: u64 = u64::MAX;

impl WideTranslationMemo {
    fn new(tick: u64) -> Self {
        Self {
            tick: std::cell::Cell::new(tick),
            slots: [const { std::cell::Cell::new(WIDE_TRANSLATION_EMPTY) }; WIDE_TRANSLATION_SLOTS],
        }
    }

    fn translate(&self, table: &dyn CaseTable, c: u32) -> u32 {
        let tick = table.write_tick();
        if self.tick.get() != tick {
            for slot in &self.slots {
                slot.set(WIDE_TRANSLATION_EMPTY);
            }
            self.tick.set(tick);
        }
        let slot = &self.slots[c as usize % WIDE_TRANSLATION_SLOTS];
        let packed = slot.get();
        if packed != WIDE_TRANSLATION_EMPTY && (packed >> 32) as u32 == c {
            return packed as u32;
        }
        let translated = table.translate(c);
        slot.set(((c as u64) << 32) | translated as u64);
        translated
    }
}

/// Sentinel for a `CaseTranslation::byte` slot that has not been computed.
///
/// Safe as a sentinel because a translation is always a valid character code
/// (`<= MAX_CHAR`), which `u32::MAX` is not. Named rather than a bare literal
/// because the array is otherwise indistinguishable from a filled one -- the
/// same class of mistake as the derived `Default` that let `IntervalTree`
/// report an empty memo as valid.
const CASE_TRANSLATION_UNFILLED: u32 = u32::MAX;

/// Which characters a case translation can map INTO the ASCII range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsciiPreimage {
    /// Only ASCII characters translate to ASCII, and every ASCII character
    /// translates to ASCII.  Always true of the hardwired standard
    /// translation (proven over every character code by
    /// `standard_translation_keeps_non_ascii_out_of_ascii`), and of GNU's
    /// standard case table, whose characters.el:803-812 leaves out the pairs
    /// that would break it (İ, ı, ſ, K) "because that makes searches slow".
    /// A case-canon char-table has it for as long as its entries say so.
    AsciiOnly,
    /// Some character may translate into ASCII from outside it, or out of
    /// ASCII from inside it.
    Unknown,
}

impl CaseTranslation {
    /// The host table behind this translation, if any.  Hosts whose table
    /// object needs tracing (romacs-core's moving collector roots the Lisp
    /// char-table) recover their own type through [`CaseTable::as_any`].
    pub fn case_table(&self) -> Option<&std::rc::Rc<dyn CaseTable>> {
        self.table.as_ref()
    }

    /// Which characters this translation maps into ASCII, as the matcher
    /// and the per-character scans read it NOW.
    ///
    /// A char-table translation reads its codes below 256 through the byte
    /// memo, which freezes a slot at first use, and the rest through the wide
    /// memo, which follows the char-table write tick; the table itself can be
    /// edited in place.  So the answer holds until the next char-table write:
    /// it is memoized by the write tick, and every search asks again.  Asking
    /// fills the ASCII slots of the byte memo.
    #[inline]
    pub(crate) fn ascii_preimage(&self) -> AsciiPreimage {
        self.ascii_preimage_for(CharTableWalk::Allowed)
    }

    /// [`Self::ascii_preimage`] for a search that only repays walking the
    /// whole char-table when it scans `span` bytes or more: a shorter one
    /// takes `Unknown` (the per-character loops) rather than walk, unless the
    /// table was already walked at this write tick.
    #[inline]
    pub(crate) fn ascii_preimage_for_span(&self, span: usize) -> AsciiPreimage {
        self.ascii_preimage_for(if span >= CHAR_TABLE_WALK_MIN_SPAN {
            CharTableWalk::Allowed
        } else {
            CharTableWalk::OnlyIfWalked
        })
    }

    #[inline]
    fn ascii_preimage_for(&self, walk: CharTableWalk) -> AsciiPreimage {
        match &self.table {
            None => AsciiPreimage::AsciiOnly,
            Some(table) => {
                let tick = table.write_tick();
                match self.ascii_preimage_memo.get() {
                    Some((at, preimage)) if at == tick => preimage,
                    _ => self.recheck_ascii_preimage(&**table, tick, walk),
                }
            }
        }
    }

    #[cold]
    #[inline(never)]
    fn recheck_ascii_preimage(
        &self,
        table: &dyn CaseTable,
        tick: u64,
        walk: CharTableWalk,
    ) -> AsciiPreimage {
        let Some(folds_into_ascii) = table.folds_into_ascii(tick, walk == CharTableWalk::Allowed)
        else {
            // Not worth a walk: answer for this search only.
            return AsciiPreimage::Unknown;
        };
        let ascii_escapes = (0..0x80u32).any(|c| self.translate(c) >= 0x80);
        // A frozen slot is what the matcher reads for these codes; an unfilled
        // one will be filled from the table as it is now, which the walk
        // below covers.
        let frozen_latin1_into_ascii = self.byte[0x80..].iter().any(|slot| {
            let translated = slot.get();
            translated != CASE_TRANSLATION_UNFILLED && translated < 0x80
        });
        let preimage = if ascii_escapes || frozen_latin1_into_ascii || folds_into_ascii {
            AsciiPreimage::Unknown
        } else {
            AsciiPreimage::AsciiOnly
        };
        tracing::debug!(
            target: "romacs::regex",
            ?preimage,
            tick,
            "case-canon char-table ASCII preimage"
        );
        self.ascii_preimage_memo.set(Some((tick, preimage)));
        preimage
    }
}

/// Whether finding a char-table translation's [`AsciiPreimage`] may walk the
/// whole table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharTableWalk {
    Allowed,
    /// Only reuse a walk already made at the current write tick.
    OnlyIfWalked,
}

/// Bytes a search must span before it walks a case-canon char-table to learn
/// its [`AsciiPreimage`].  The walk visits every slot of the table (the
/// standard canon table covers all of Unicode's case pairs), which the
/// per-character scan of this many bytes, at about 18 instructions a byte,
/// repays; it recurs only after a char-table write.
pub const CHAR_TABLE_WALK_MIN_SPAN: usize = 16 * 1024;

impl CaseTranslation {
    pub fn standard() -> Self {
        // The canonicalization of bytes 0..256 is a constant
        // (`downcase_char_code_emacs_compat`), but a case-insensitive regex is
        // recompiled on every `re-search-forward`, and recomputing this table
        // via Unicode case folding each time was ~15% of a search.  GNU keeps
        // its case-canon table precomputed; build it once per thread and copy.
        thread_local! {
            static STANDARD_BYTE: [u32; 256] = {
                let mut byte = [0u32; 256];
                for i in 0..=255u32 {
                    byte[i as usize] = CaseTranslation::canonicalize_char(i);
                }
                byte
            };
        }
        // A copy loop, not `from_fn': its closure, called out of line for
        // each byte, cost more than many searches that build this.
        let byte = [const { std::cell::Cell::new(0) }; 256];
        STANDARD_BYTE.with(|b| byte.iter().zip(b).for_each(|(cell, &code)| cell.set(code)));
        Self {
            byte,
            table: None,
            wide: std::cell::OnceCell::new(),
            ascii_preimage_memo: std::cell::Cell::new(None),
        }
    }

    /// A translation backed by a buffer's case-canon char-table.
    ///
    /// Slots are filled ON DEMAND. Eagerly translating all 256 codes here cost
    /// 256 `ct_lookup` calls per construction, and `buffer_search_translation`
    /// builds one of these on EVERY case-folded `looking_at` /
    /// `re-search-forward` -- which font-lock issues thousands of times, each
    /// typically examining a handful of characters. A profile put
    /// `ct_lookup` at 4.29% of a font-lock scroll with this its largest
    /// single consumer.
    ///
    /// Filling on demand is strictly less work at every match length (k
    /// distinct codes examined costs k lookups, never 256) and is exactly as
    /// sound as the eager fill: the memo lives and dies with this instance, so
    /// it cannot outlive the table the way an identity-keyed cache could.
    /// `standard()` above already applied the same lesson to the constant
    /// table; this path was simply missed.
    pub fn from_case_table(table: std::rc::Rc<dyn CaseTable>) -> Self {
        Self {
            byte: [const { std::cell::Cell::new(CASE_TRANSLATION_UNFILLED) }; 256],
            table: Some(table),
            wide: std::cell::OnceCell::new(),
            ascii_preimage_memo: std::cell::Cell::new(None),
        }
    }

    /// Translation for a code below 256, computing and memoizing it on a miss.
    #[inline]
    fn byte_slot(&self, c: usize) -> Option<u32> {
        let slot = self.byte.get(c)?;
        let cached = slot.get();
        if cached != CASE_TRANSLATION_UNFILLED {
            return Some(cached);
        }
        let translated = match &self.table {
            Some(table) => table.translate(c as u32),
            None => Self::canonicalize_char(c as u32),
        };
        slot.set(translated);
        Some(translated)
    }

    /// Inline for the codes below 256 (the per-byte search scans and the
    /// matcher ask for these constantly); the rest go out of line.
    #[inline]
    pub fn translate(&self, c: u32) -> u32 {
        if let Some(translated) = self.byte_slot(c as usize) {
            return translated;
        }
        self.translate_wide(c)
    }

    #[inline(never)]
    fn translate_wide(&self, c: u32) -> u32 {
        if let Some(table) = &self.table {
            return self
                .wide
                .get_or_init(|| Box::new(WideTranslationMemo::new(table.write_tick())))
                .translate(&**table, c);
        }
        Self::canonicalize_char(c)
    }

    pub(crate) fn translate_byte(&self, c: u8) -> u8 {
        self.byte_slot(c as usize).unwrap_or(c as u32) as u8
    }

    fn canonicalize_char(c: u32) -> u32 {
        emacs_downcase_compat(c as i64) as u32
    }
}

/// Case-fold translate, mirroring the `tr` closure in `re_match_internal`.
#[inline]
pub(crate) fn re_tr(translate: &Option<CaseTranslation>, c: u32) -> u32 {
    match translate {
        Some(table) => table.translate(c),
        None => c,
    }
}

/// The leading code of the translation of the ASCII byte `byte`: its own
/// translation in every standard table.  Inlined into the scan loops, which
/// meet an ASCII byte at almost every position.
#[inline(always)]
pub(crate) fn ascii_translated_leading_code(table: &CaseTranslation, byte: u8) -> usize {
    let translated = table.translate(byte as u32);
    if translated < 0x80 {
        translated as usize
    } else {
        emacs_char::char_leading_code(translated) as usize
    }
}

/// The leading code of the translation of the non-ASCII character starting
/// at `pos` (a character boundary) of multibyte text, and its length.
#[inline(never)]
pub(crate) fn multibyte_translated_leading_code(
    table: &CaseTranslation,
    text: &[u8],
    pos: usize,
) -> (usize, usize) {
    let (c, len) = emacs_char::string_char(&text[pos..]);
    (
        emacs_char::char_leading_code(table.translate(c)) as usize,
        len.max(1),
    )
}
