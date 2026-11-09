use crate::SyntaxClass;
use crate::emacs_char;
use crate::engine::dfa::nfa::{Assertions, Nfa, Predicate};
use crate::engine::matcher::step::{
    match_anychar_at, match_categoryspec_at, match_charset_at, match_exactn_char_at,
    match_syntaxspec_at, match_syntaxspecset_at, re_text_char,
};
use crate::engine::pattern::CompiledPattern;
use crate::engine::syntax::{LookupClassKey, SyntaxCacheKey, SyntaxLookup};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Character classes (C3)
// ---------------------------------------------------------------------------

/// Facts about one character that the zero-width assertions read, besides
/// the predicates.  Only the facts a pattern's assertions read are kept
/// ([`Nfa::fact_mask`]), so they split no class needlessly.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Facts(pub(crate) u8);

impl Facts {
    /// No character: the position is the start of the text (only ever a
    /// PREVIOUS-character fact).
    pub const EDGE: Self = Self(1 << 0);
    /// The character is a newline (`^` after it, `$` before it).
    pub const NEWLINE: Self = Self(1 << 1);
    /// Word syntax (`\b \B \< \>`).
    pub const WORD: Self = Self(1 << 2);
    /// Word or symbol syntax (`\_< \_>`).
    pub const WORD_OR_SYMBOL: Self = Self(1 << 3);
    /// A word constituent above U+00FF.  Between two word constituents GNU's
    /// `WORD_BOUNDARY_P` consults scripts and categories unless both are at
    /// or below U+00FF (`WordBoundaryLookup::boundary_between`), so such a
    /// pair decides `\b` per character pair, never per class.
    pub const WIDE_WORD: Self = Self(1 << 4);

    pub const fn empty() -> Self {
        Self(0)
    }

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[inline]
    pub const fn bits(self) -> u8 {
        self.0
    }

    #[inline]
    const fn masked(self, mask: Self) -> Self {
        Self(self.0 & mask.0)
    }

    /// The facts of the Emacs character `code` at input position `at`, whose
    /// first byte is `first_byte`, under `syntax` (the base table, or the
    /// real lookup inside a `syntax-table` property run).
    fn of_char(
        code: u32,
        first_byte: u8,
        syntax: &dyn SyntaxLookup,
        at: usize,
        mask: Self,
    ) -> Self {
        let mut facts = 0u8;
        if first_byte == b'\n' {
            facts |= Self::NEWLINE.0;
        }
        if mask.0 & (Self::WORD.0 | Self::WORD_OR_SYMBOL.0 | Self::WIDE_WORD.0) != 0 {
            // Match the backtracker's full-code syntax and boundary facts.
            let ch = emacs_char::EmacsChar::from_code_unchecked(code);
            let class = syntax.emacs_char_syntax_at(ch, at);
            if class == SyntaxClass::Word {
                facts |= Self::WORD.0 | Self::WORD_OR_SYMBOL.0;
                if ch.code() > 0xFF {
                    facts |= Self::WIDE_WORD.0;
                }
            } else if class == SyntaxClass::Symbol {
                facts |= Self::WORD_OR_SYMBOL.0;
            }
        }
        Self(facts).masked(mask)
    }
}

impl std::ops::BitOr for Facts {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl Nfa {
    /// The character facts this pattern's assertions read.
    pub fn fact_mask(&self) -> Facts {
        let mut mask = Facts::empty();
        if self
            .assertions
            .intersects(Assertions::BEG_LINE | Assertions::END_LINE)
        {
            mask = mask | Facts::NEWLINE;
        }
        if self.assertions.intersects(Assertions::WORD) {
            mask = mask | Facts::WORD | Facts::WIDE_WORD;
        }
        if self.assertions.intersects(Assertions::SYMBOL) {
            mask = mask | Facts::WORD_OR_SYMBOL;
        }
        mask
    }
}

/// The class of a character: the set of predicates that accept it, and its
/// facts.  Two characters with the same key are indistinguishable to the
/// pattern, so the DFA steps them with one transition.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ClassKey {
    accepts: SmallVec<[u64; 2]>,
    pub facts: Facts,
}

impl ClassKey {
    #[inline]
    pub fn accepts(&self, predicate: u16) -> bool {
        let predicate = predicate as usize;
        self.accepts[predicate / 64] & (1 << (predicate % 64)) != 0
    }
}

/// A syntax lookup that answers every position from the base table.  The
/// memoized classes are computed through it, so they are functions of the
/// character alone; they serve only positions where no `syntax-table`
/// property applies ([`SyntaxLookup::plain_syntax_until`]), where the base
/// table is the answer.  A character inside a property run is classified
/// through the real lookup at its position
/// ([`CharClasses::class_at_position`]).
pub struct BaseTableView<'a>(pub &'a dyn SyntaxLookup);

impl SyntaxLookup for BaseTableView<'_> {
    fn emacs_char_syntax(&self, c: emacs_char::EmacsChar) -> SyntaxClass {
        self.0.emacs_char_syntax(c)
    }

    fn emacs_char_syntax_at(&self, c: emacs_char::EmacsChar, _input_pos: usize) -> SyntaxClass {
        self.0.emacs_char_syntax(c)
    }

    fn emacs_word_boundary_between(
        &self,
        c1: emacs_char::EmacsChar,
        c2: emacs_char::EmacsChar,
    ) -> bool {
        self.0.emacs_word_boundary_between(c1, c2)
    }

    fn char_syntax(&self, c: char) -> SyntaxClass {
        self.0.char_syntax(c)
    }

    fn char_syntax_at(&self, c: char, _input_pos: usize) -> SyntaxClass {
        self.0.char_syntax(c)
    }

    fn char_has_category(&self, c: char, cat: u8) -> bool {
        self.0.char_has_category(c, cat)
    }

    fn word_boundary_between(&self, c1: char, c2: char) -> bool {
        self.0.word_boundary_between(c1, c2)
    }

    fn cache_key(&self) -> SyntaxCacheKey {
        self.0.cache_key()
    }

    fn class_cache_key(&self) -> Option<LookupClassKey> {
        self.0.class_cache_key()
    }

    fn table_write_tick(&self) -> u64 {
        self.0.table_write_tick()
    }

    fn position_dependent(&self) -> bool {
        false
    }
}

impl Nfa {
    /// Whether `predicate` accepts the character at `d` (`len` bytes): the
    /// matcher's own per-character test.
    fn predicate_accepts(
        &self,
        predicate: Predicate,
        pattern: &CompiledPattern,
        text: &[u8],
        d: usize,
        len: usize,
        syntax: &dyn SyntaxLookup,
    ) -> bool {
        let stop = d + len;
        let target_multibyte = pattern.target_multibyte;
        let accepted = match predicate {
            Predicate::Literal { pc, lit_off } => {
                let pc = pc as usize;
                let count = self.bytecode[pc + 1] as usize;
                let literal = &self.bytecode[pc + 2..pc + 2 + count];
                match_exactn_char_at(
                    literal,
                    lit_off as usize,
                    pattern.multibyte,
                    target_multibyte,
                    &pattern.translate,
                    text,
                    d,
                    stop,
                )
                .map(|(_, text_advance)| text_advance)
            }
            Predicate::AnyChar => {
                match_anychar_at(text, d, stop, target_multibyte, &pattern.translate)
            }
            Predicate::Charset { pc } => match_charset_at(
                pattern,
                pc as usize,
                text,
                d,
                stop,
                target_multibyte,
                &pattern.translate,
                syntax,
            ),
            Predicate::Syntax { class, negate } => {
                match_syntaxspec_at(class, negate, text, d, stop, target_multibyte, syntax)
            }
            Predicate::SyntaxSet { mask } => {
                match_syntaxspecset_at(mask, text, d, stop, target_multibyte, syntax)
            }
            Predicate::Category { category, negate } => {
                match_categoryspec_at(category, negate, text, d, stop, target_multibyte, syntax)
            }
        };
        debug_assert!(accepted.is_none_or(|advance| advance == len));
        accepted.is_some()
    }

    /// The class of the character at `d` of `text`, `len` bytes long, under
    /// `syntax` (the base table, or the real lookup at `d`).
    #[allow(clippy::too_many_arguments)]
    pub fn class_key_at(
        &self,
        pattern: &CompiledPattern,
        text: &[u8],
        d: usize,
        code: u32,
        len: usize,
        syntax: &dyn SyntaxLookup,
        mask: Facts,
    ) -> ClassKey {
        let mut accepts: SmallVec<[u64; 2]> =
            SmallVec::from_elem(0, self.predicates.len().div_ceil(64).max(1));
        for (i, &predicate) in self.predicates.iter().enumerate() {
            if self.predicate_accepts(predicate, pattern, text, d, len, syntax) {
                accepts[i / 64] |= 1 << (i % 64);
            }
        }
        ClassKey {
            accepts,
            facts: Facts::of_char(code, text[d], syntax, d, mask),
        }
    }
}

/// `byte_class` value of a byte whose class is not known yet (or, for the
/// bytes of a non-ASCII character of multibyte text, is never kept there).
pub const UNKNOWN_CLASS: u8 = 0xFF;
/// Class ids are `u8` below [`UNKNOWN_CLASS`].
pub const MAX_CLASSES: usize = UNKNOWN_CLASS as usize;
const WIDE_SLOTS: usize = 512;
const WIDE_EMPTY: u32 = u32::MAX;
const POSITIONAL_SLOTS: usize = 64;
/// `byte_facts` value of a byte whose facts are not known yet (facts use
/// the low five bits).
pub const NO_FACTS: u8 = 0xFF;

/// What the character-to-class maps of one pattern are valid for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClassContext {
    /// The tables behind the syntax lookup, when the classes read it.
    lookup: Option<LookupClassKey>,
    /// [`SyntaxLookup::table_write_tick`] when the classes read the syntax
    /// or category tables, else 0.  In romacs-core this is the global
    /// char-table write tick, which moves with every char-table write and
    /// allocation.
    pub tick: u64,
    /// [`crate::CaseTable::write_tick`] when a case table's non-ASCII
    /// translations are read, else 0 (romacs-core: the same global tick).
    pub case_tick: u64,
    target_multibyte: bool,
}

impl ClassContext {
    /// The context of a search with `syntax`, or `None` when the lookup's
    /// tables have no identity to key a cache by.
    pub fn of_search(
        pattern: &CompiledPattern,
        nfa: &Nfa,
        syntax: &dyn SyntaxLookup,
    ) -> Option<Self> {
        // The NFA's own record, not the pattern's `uses_syntax`, which any
        // bracket class sets: `[:alnum:]' and the other classes but
        // `[:space:]', `[:word:]' and `[:punct:]' read no table.
        let reads_lookup = nfa.reads_syntax || nfa.uses_categories;
        let lookup = if reads_lookup {
            Some(syntax.class_cache_key()?)
        } else {
            None
        };
        let case_table = pattern
            .translate
            .as_ref()
            .and_then(|translate| translate.table.as_ref());
        Some(Self {
            lookup,
            tick: if reads_lookup {
                syntax.table_write_tick()
            } else {
                0
            },
            case_tick: case_table.map_or(0, |table| table.write_tick()),
            target_multibyte: pattern.target_multibyte,
        })
    }
}

/// Too many distinct classes for `u8` ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooManyClasses;

/// The classes of one pattern and the character-to-class maps.
///
/// Class ids name [`ClassKey`]s, which do not depend on the search context,
/// so a context change (another syntax table, a char-table write) only
/// empties the maps from characters to ids; the DFA's transitions, keyed by
/// class id, stay valid.
pub struct CharClasses {
    /// Class of each byte: every byte of unibyte text, ASCII of multibyte.
    pub byte_class: [u8; 256],
    /// Direct-mapped `(character code, class)` memo for the other characters.
    wide: Box<[(u32, u8)]>,
    /// Direct-mapped `(character code, syntax class there, class)` memo for
    /// characters inside `syntax-table` property runs.
    positional: Box<[(u32, u8, u8)]>,
    /// The facts of each tabled byte's character ([`NO_FACTS`]: not known
    /// yet), for the character before a candidate, which the DFA need not
    /// have classified.
    pub byte_facts: [u8; 256],
    keys: Vec<ClassKey>,
    ids: FxHashMap<ClassKey, u8>,
    context: Option<ClassContext>,
    mask: Facts,
    /// Context changes that emptied the maps.
    pub resets: u64,
}

impl CharClasses {
    pub fn new(mask: Facts) -> Self {
        Self {
            byte_class: [UNKNOWN_CLASS; 256],
            wide: vec![(WIDE_EMPTY, 0); WIDE_SLOTS].into_boxed_slice(),
            positional: vec![(WIDE_EMPTY, 0, 0); POSITIONAL_SLOTS].into_boxed_slice(),
            byte_facts: [NO_FACTS; 256],
            keys: Vec::new(),
            ids: FxHashMap::default(),
            context: None,
            mask,
            resets: 0,
        }
    }

    /// Make the maps valid for a search in `context`.
    pub fn sync(&mut self, context: ClassContext) {
        if self.context == Some(context) {
            return;
        }
        if self.context.is_some() {
            self.resets += 1;
        }
        self.byte_class = [UNKNOWN_CLASS; 256];
        self.wide.fill((WIDE_EMPTY, 0));
        self.positional.fill((WIDE_EMPTY, 0, 0));
        self.byte_facts = [NO_FACTS; 256];
        self.context = Some(context);
    }

    #[inline]
    pub fn key(&self, class: u8) -> &ClassKey {
        &self.keys[class as usize]
    }

    #[inline]
    pub fn facts(&self, class: u8) -> Facts {
        self.keys[class as usize].facts
    }

    // Internal (reachable only through the test seams); never empty in use.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    fn intern(&mut self, key: ClassKey) -> Result<u8, TooManyClasses> {
        if let Some(&id) = self.ids.get(&key) {
            return Ok(id);
        }
        if self.keys.len() >= MAX_CLASSES {
            return Err(TooManyClasses);
        }
        let id = self.keys.len() as u8;
        self.keys.push(key.clone());
        self.ids.insert(key, id);
        Ok(id)
    }

    /// The class and byte length of the character at `d` (`d < text.len()`),
    /// from the maps or computed now.
    pub fn class_at(
        &mut self,
        nfa: &Nfa,
        pattern: &CompiledPattern,
        text: &[u8],
        d: usize,
        base: &BaseTableView<'_>,
    ) -> Result<(u8, usize), TooManyClasses> {
        let byte = text[d];
        let tabled = !pattern.target_multibyte || byte < 0x80;
        if tabled {
            let class = self.byte_class[byte as usize];
            if class != UNKNOWN_CLASS {
                return Ok((class, 1));
            }
        }
        let (code, len) = re_text_char(text, d, pattern.target_multibyte)
            .expect("a class is asked for a character inside the text");
        if !tabled {
            let slot = self.wide[code as usize % WIDE_SLOTS];
            if slot.0 == code {
                return Ok((slot.1, len));
            }
        }
        let key = nfa.class_key_at(pattern, text, d, code, len, base, self.mask);
        let class = self.intern(key)?;
        if tabled {
            self.byte_class[byte as usize] = class;
        } else {
            self.wide[code as usize % WIDE_SLOTS] = (code, class);
        }
        Ok((class, len))
    }

    /// The class and byte length of the character at `d` (`d < text.len()`)
    /// where a `syntax-table` property may apply: what the matcher's tests
    /// answer there through `syntax` at `d`.
    ///
    /// Every syntax read a class makes is the syntax of this one character
    /// at `d` (`match_syntaxspec_at`, `posix_class_matches`, the facts), so
    /// the class is a function of the character and its syntax class there:
    /// that pair keys the memo, and the base-table memo serves the pair
    /// whenever the property leaves the table's answer unchanged.
    pub fn class_at_position(
        &mut self,
        nfa: &Nfa,
        pattern: &CompiledPattern,
        text: &[u8],
        d: usize,
        base: &BaseTableView<'_>,
        syntax: &dyn SyntaxLookup,
    ) -> Result<(u8, usize), TooManyClasses> {
        let (code, len) = re_text_char(text, d, pattern.target_multibyte)
            .expect("a class is asked for a character inside the text");
        let ch = emacs_char::EmacsChar::from_code_unchecked(code);
        let here = syntax.emacs_char_syntax_at(ch, d);
        let slot_index = (code as usize ^ ((here as usize) << 4)) % POSITIONAL_SLOTS;
        let slot = self.positional[slot_index];
        if slot.0 == code && slot.1 == here as u8 {
            return Ok((slot.2, len));
        }
        let class = if here == base.emacs_char_syntax(ch) {
            self.class_at(nfa, pattern, text, d, base)?.0
        } else {
            let key = nfa.class_key_at(pattern, text, d, code, len, syntax, self.mask);
            self.intern(key)?
        };
        self.positional[slot_index] = (code, here as u8, class);
        Ok((class, len))
    }

    /// The facts of the tabled byte `text[at]` (a byte of unibyte text, or
    /// ASCII) where the base table applies: its class's facts, or memoized.
    #[inline]
    pub(crate) fn byte_facts_at(
        &mut self,
        text: &[u8],
        at: usize,
        target_multibyte: bool,
        base: &BaseTableView<'_>,
    ) -> Facts {
        let byte = text[at];
        let class = self.byte_class[byte as usize];
        if class != UNKNOWN_CLASS {
            return self.facts(class);
        }
        let memo = self.byte_facts[byte as usize];
        if memo != NO_FACTS {
            return Facts(memo);
        }
        let facts = self.facts_at(text, at, target_multibyte, base);
        self.byte_facts[byte as usize] = facts.0;
        facts
    }

    /// The facts of the character at `at` (the matcher's view of the
    /// character before a candidate), read through `syntax` at `at`.  A
    /// memoized class carries the same facts where the base table applies.
    pub fn facts_at(
        &self,
        text: &[u8],
        at: usize,
        target_multibyte: bool,
        syntax: &dyn SyntaxLookup,
    ) -> Facts {
        let (code, _) =
            re_text_char(text, at, target_multibyte).expect("the previous character exists");
        Facts::of_char(code, text[at], syntax, at, self.mask)
    }
}
