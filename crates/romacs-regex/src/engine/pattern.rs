//! The compiled pattern and the match registers a match fills.
//!
//! GNU: `struct re_pattern_buffer` and `struct re_registers` in
//! regex-emacs.h.

use crate::engine::dfa;
use crate::engine::fastmap::recompute_fastmap;
use crate::engine::linear;
use crate::engine::scan::{
    FoldedScan, FoldedScans, SparseAsciiFastmap, TextRepr, build_folded_scan,
    build_literal_prefilter, sparse_ascii_fastmap,
};
use crate::engine::search::StartAnchor;
use crate::engine::syntax::SyntaxLookup;
use crate::engine::translate::CaseTranslation;
use regex_automata::util::prefilter::Prefilter as RaPrefilter;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

// The compiled-pattern layout is the backtracker's own; it is not part of
// the crate's API, so its fields are crate-private.
/// A compiled regex pattern — the output of `regex_compile()`.
///
/// Mirrors GNU's `struct re_pattern_buffer` from regex-emacs.h.
/// Bytecode and its seal are private to the compiler/matcher module: callers
/// cannot modify the bytes or forge the seal. Matching borrows the bytes
/// immutably; cloned patterns retain the same validated bytes and seal.
/// Threading assumption: each evaluator owns its pattern cache; the existing
/// interior-mutable search caches do not permit concurrent shared mutation.
#[derive(Clone)]
pub struct CompiledPattern {
    /// Bytecode buffer.
    pub(crate) buffer: Vec<u8>,

    /// Number of subexpressions (groups).
    pub(crate) re_nsub: usize,

    /// Fast rejection map: fastmap[c] is true if the pattern can start
    /// with byte c.  Used by `re_search` to skip non-matching positions.
    pub(crate) fastmap: [bool; 256],

    /// The fastmap a case-folded search over MULTIBYTE text consults, indexed
    /// by the leading code of the TRANSLATED character (GNU `re_search_2`).
    /// It differs from `fastmap` only for a leading multibyte literal: its
    /// characters are stored translated, so the one leading code of the
    /// stored character is the only candidate, where `fastmap` -- indexed by
    /// a translated BYTE -- has to admit every non-ASCII leading byte.
    pub(crate) fastmap_translated: [bool; 256],

    /// Whether the fastmap is valid (needs recomputation after compile).
    pub(crate) fastmap_accurate: bool,

    /// The tiny ASCII candidate set used by memchr, derived on first use.
    /// Rebuilt with the fastmap, including syntax-table recomputation.
    pub(crate) sparse_ascii_fastmap: std::cell::OnceCell<Option<SparseAsciiFastmap>>,

    /// How a case-folded search finds its next candidate: `TRANSLATE` folded
    /// into the fastmap once, per text representation (see
    /// [`CompiledPattern::folded_scan`]).  Rebuilt with the fastmap.
    pub(crate) folded_scans: FoldedScans,

    /// True if the pattern was compiled for POSIX backtracking.
    pub(crate) posix: bool,

    /// True if the source regexp string was multibyte.
    pub(crate) multibyte: bool,

    /// True if the current search target is multibyte.
    pub(crate) target_multibyte: bool,

    /// True if the pattern can match the empty string.
    pub(crate) can_be_null: bool,

    /// True if matching this pattern consults the syntax table at MATCH
    /// time (`\b \B \< \> \_< \_> \w \W \sC \SC` and POSIX classes).
    ///
    /// This is informational only: the matcher resolves per-character
    /// syntax through the `SyntaxLookup` passed to each `re_match` call,
    /// so the compiled bytecode itself stays table-independent.  In
    /// particular it does NOT disable the search fastmap — GNU gates the
    /// fastmap skip solely on `fastmap_accurate && !can_be_null`
    /// (regex-emacs.c:3483 `if (fastmap && startpos < total_size &&
    /// !bufp->can_be_null)`), and syntax-dependent *leading* atoms
    /// (`\w`, `\sC`) set `can_be_null` via `analyze_first` instead.
    pub(crate) uses_syntax: bool,

    /// True if the compiled artifacts hardcode syntax-table CONTENT, so
    /// the pattern is only valid for the syntax table it was compiled
    /// against and the compile cache must key such entries by table.
    ///
    /// Mirrors GNU `re_pattern_buffer.used_syntax` (regex-emacs.h:112),
    /// which is set only for `[[:space:]]` / `[[:word:]]` inside a
    /// charset (regex-emacs.c:2096-2101: "In most cases the matching
    /// rule for char classes only uses the syntax table for multibyte
    /// chars ... SPACE and WORD are the two exceptions") and makes
    /// `search.c:compile_pattern` key the cache entry by
    /// `BVAR (current_buffer, syntax_table)`.
    ///
    /// In romacs the charset MATCH path re-derives ASCII class
    /// membership per call from the active table, so only the FASTMAP
    /// bakes table content (see `compile_fastmap`); the caching rule is
    /// the same.
    pub(crate) used_syntax: bool,

    /// Character translation table for case-folding.
    ///
    /// GNU stores the active case-canon char-table in `re_pattern_buffer.translate`.
    /// Keep the same shape at the matcher boundary: literal compilation and input
    /// matching both call through this translator, with a 256-entry byte fast path
    /// only for fastmap/bitmap operations.
    pub(crate) translate: Option<CaseTranslation>,

    /// Multibyte (non-ASCII) character ranges for Charset/CharsetNot opcodes.
    /// Key = bytecode position of the Charset/CharsetNot opcode.
    /// Value = list of inclusive (start_char, end_char) character ranges.
    pub(crate) multibyte_charsets: FxHashMap<usize, Vec<(char, char)>>,

    /// Per-charset POSIX character class flags.
    ///
    /// GNU stores POSIX classes in the charset range-table bits
    /// (`regex-emacs.c:re_wctype_to_bit`) and checks `re_iswctype`
    /// while executing the charset.  The ASCII bitmap remains the fast path,
    /// but these bits preserve the runtime predicate for multibyte characters
    /// and for syntax-table-sensitive classes such as `word` and `space`.
    /// Fx-hashed: probed per charset op while matching (57K probes per
    /// five org font-lock operations); SipHash on a `usize` key was 1.7M Ir/op.
    pub(crate) charset_class_bits: FxHashMap<usize, u32>,

    /// True if the pattern used a non-greedy optional `??`.  Set at the sole
    /// `??` emit site, which produces GNU's `on_failure_jump` + `jump` shape
    /// (regex-emacs.c:2009-2015; no keep-string jump).  It keeps the pattern
    /// off the Pike VM (see [`compute_pike_eligible`](crate::engine::compile::seal::compute_pike_eligible)), whose priority order
    /// for that skip-first split has not been validated.
    pub(crate) has_nongreedy_optional: bool,

    /// True if this pattern is eligible for the non-backtracking Pike VM
    /// fast path (see [`pike_match`](crate::engine::matcher::pike_match)).  Computed once at compile time by
    /// [`compute_pike_eligible`](crate::engine::compile::seal::compute_pike_eligible): the bytecode must contain no
    /// backreference (`Duplicate`), no interval-counter op
    /// (`SucceedN`/`JumpN`/`SetNumberAt` from `\{n,m\}`), and the pattern
    /// must not be POSIX-longest — every remaining op is simulated
    /// byte-exactly by the Pike VM.  Ineligible patterns fall back to the
    /// backtracker with identical output.
    pub(crate) pike_eligible: bool,

    /// How the linear-time matcher may run this pattern ([`linear::Linear`],
    /// computed at compile time by [`linear::classify`]): the routing for
    /// the budgeted backtracker's fallback and for `re_search`'s
    /// one-pass forward search.  A superset of `pike_eligible`, which
    /// keeps its step-1 meaning (the fuzzers select cases by it): the
    /// linear matcher also runs a capture inside a nullable loop, with
    /// GNU's empty-loop rule.
    pub(crate) linear: linear::Linear,

    /// True once [`validate_sealed_buffer`](crate::engine::compile::seal::validate_sealed_buffer) proved every opcode byte valid,
    /// every operand span in-bounds, and every jump target on an op
    /// boundary — the backtracker then dispatches with unchecked fetches
    /// (the seal-then-trust protocol the bytecode VM uses for `seal_ops`).
    /// False (e.g. a hand-assembled test buffer) keeps the checked loop.
    pub(crate) buffer_sealed: bool,

    /// The rewind view of the bytecode, for the engines that step every
    /// thread one position at a time (the Pike VM, the existence DFA): see
    /// [`RewindView`].  Computed for every pattern at compile time.
    pub(crate) rewind: RewindView,

    /// Multi-literal SIMD prefilter that AUGMENTS (never replaces) the
    /// backtracker's search skip loop.  When `Some`, `re_search`'s forward
    /// path uses `regex-automata`'s `Prefilter` (memchr / Teddy /
    /// Aho-Corasick, auto-selected) to jump straight to the next position
    /// whose text provably contains a required match literal, instead of the
    /// single-byte fastmap scan.  The backtracker still verifies every
    /// candidate, so the ONLY correctness requirement is that the literals be
    /// SOUND: every match must contain one of them at `LiteralPrefilter.offset`
    /// bytes from the match start (see `build_literal_prefilter`).  `None` when
    /// no such literal set could be proven (fall back to the fastmap — always
    /// correct).  A case-folded pattern gets the ASCII case variants of its
    /// literals, used only while its translation folds nothing non-ASCII into
    /// ASCII (see [`fold_prefix_literals`](crate::engine::scan::fold_prefix_literals)).  Never set for patterns whose
    /// only required literals are single bytes (the fastmap's memchr already
    /// handles those).
    ///
    /// Built on first use by a search long enough to use it
    /// ([`CompiledPattern::literal_prefilter`]): most compiled patterns only
    /// ever match short strings, and building the scanner cost the byte
    /// compiler 6M instructions per file for 100 patterns that never
    /// searched a buffer.
    pub(crate) prefilter: std::cell::OnceCell<Option<LiteralPrefilter>>,

    /// The zero-width anchor every path from the start meets before it
    /// consumes anything ([`start_anchor`](crate::engine::search::start_anchor)), when that is more than the first
    /// opcode shows (`\(?:^a\|^b\)`).  Computed only under
    /// `ROMACS_REGEX_ANCHOR_ALT` ([`anchor_alt_enabled`](crate::engine::search::anchor_alt_enabled)); `None` otherwise.
    pub(crate) start_anchor: StartAnchor,

    /// The existence DFA's slot (`ROMACS_REGEX_DFA`, [`dfa::filter::DfaCell`]): built
    /// lazily by the searches that use it; a clone starts cold.
    pub(crate) dfa: dfa::filter::DfaCell,
}

/// Bytes of text below which a forward search does not build a pattern's
/// literal prefilter (it still uses one already built).
pub(crate) const PREFILTER_MIN_BUILD_SPAN: usize = 256;

impl CompiledPattern {
    pub(crate) fn sparse_ascii_fastmap(&self) -> Option<SparseAsciiFastmap> {
        *self
            .sparse_ascii_fastmap
            .get_or_init(|| sparse_ascii_fastmap(&self.fastmap))
    }

    /// The multi-literal prefilter, built now if it was not yet.
    pub(crate) fn literal_prefilter(&self) -> Option<&LiteralPrefilter> {
        self.prefilter
            .get_or_init(|| build_literal_prefilter(self))
            .as_ref()
    }

    /// The case-folded candidate scan for the text representation this
    /// pattern searches, built by the first search long enough to use it (the
    /// literal prefilter's policy).  `None`: not built yet and this search is
    /// short -- use the per-character loop.
    #[inline]
    pub fn folded_scan(&self, table: &CaseTranslation, may_build: bool) -> Option<&FoldedScan> {
        let cell = match TextRepr::of(self.target_multibyte) {
            TextRepr::Unibyte => &self.folded_scans.unibyte,
            TextRepr::Multibyte => &self.folded_scans.multibyte,
        };
        match cell.get() {
            Some(scan) => Some(scan),
            None if may_build => Some(cell.get_or_init(|| build_folded_scan(self, table))),
            None => None,
        }
    }
}

/// The bytecode with every smart keep-string loop (`OnFailureKeepStringJump`
/// whose jump-back targets the loop body) de-optimized back to the equivalent
/// REWIND loop (`OnFailureJump` with the jump-back targeting the split).
///
/// Keep-string jumps resume the loop EXIT at the position where the body
/// fails, which an engine that keeps each thread pinned to the current
/// position cannot represent.  The rewind form re-evaluates the exit at every
/// position, which such an engine handles correctly and which is semantically
/// identical (the compiler only applies the keep-string optimization when body
/// and continuation are mutually exclusive, GNU `mutually_exclusive_p`, or
/// when the loop exits straight to the final non-POSIX `Succeed`, where the
/// greedy longest run is the rewind loop's first success too).  The
/// transform preserves byte LENGTH and all opcode positions, so the
/// position-keyed charset side tables stay valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RewindView {
    /// No keep-string loop: the rewind view is `buffer` itself.
    Buffer,
    /// `buffer` with its keep-string loops rewritten.
    Rewritten(Box<[u8]>),
    /// A keep-string jump of an unexpected shape (fail closed: no engine
    /// that needs the view may run this pattern).
    Unavailable,
}

impl CompiledPattern {
    /// The rewind view's bytecode, or `None` when it is [`RewindView::Unavailable`].
    #[inline]
    pub(crate) fn rewind_bytecode(&self) -> Option<&[u8]> {
        match &self.rewind {
            RewindView::Buffer => Some(&self.buffer),
            RewindView::Rewritten(bytecode) => Some(bytecode),
            RewindView::Unavailable => None,
        }
    }
}

/// A sound multi-literal prefilter for a compiled pattern.  See
/// [`CompiledPattern::prefilter`] and [`build_literal_prefilter`].
#[derive(Clone)]
pub struct LiteralPrefilter {
    /// `regex-automata`'s SIMD literal scanner over the needle set.
    pub(crate) pf: RaPrefilter,
    /// Byte offset of the found literal within the match: a literal found at
    /// text index `j` means a candidate match may start at `j - offset`.
    /// Currently always `0` (see `build_literal_prefilter`).
    pub(crate) offset: usize,
}

// ---------------------------------------------------------------------------
// Match Registers
// ---------------------------------------------------------------------------

/// Match result — stores group start/end positions.
///
/// Mirrors GNU's `struct re_registers` from regex-emacs.h.
#[derive(Clone, Debug, Default)]
pub struct MatchRegisters {
    /// Start positions for each group (group 0 = full match).
    /// -1 means group did not participate in match.
    /// Inline up to 8 groups: GNU reuses one global `search_regs`, so a heap
    /// Vec pair per match would be allocator churn GNU never pays.
    pub start: SmallVec<[i64; 8]>,

    /// End positions for each group.
    pub end: SmallVec<[i64; 8]>,
}

impl MatchRegisters {
    pub fn new(num_groups: usize) -> Self {
        Self {
            start: SmallVec::from_elem(-1, num_groups),
            end: SmallVec::from_elem(-1, num_groups),
        }
    }

    pub fn num_regs(&self) -> usize {
        self.start.len()
    }
}

impl CompiledPattern {
    /// The case translation this pattern folds with, or `None` for a
    /// case-sensitive pattern.
    #[inline]
    pub fn translation(&self) -> Option<&CaseTranslation> {
        self.translate.as_ref()
    }

    /// Whether matching reads the syntax lookup at all (`\\w`, `\\sC`,
    /// `\\b`, `[[:word:]]`, ...), so a result depends on the syntax table.
    #[inline]
    pub fn uses_syntax(&self) -> bool {
        self.uses_syntax
    }

    /// GNU `used_syntax` (regex-emacs.c): the compiled artifact itself baked
    /// syntax-table content, so a host cache must key it by the table and
    /// call [`Self::rebind_syntax_table`] before caching it under a
    /// non-standard one.
    #[inline]
    pub fn used_syntax(&self) -> bool {
        self.used_syntax
    }

    /// Whether the text this pattern will be matched against is multibyte.
    /// Compilation defaults it to the pattern's own multibyteness.
    #[inline]
    pub fn set_target_multibyte(&mut self, target_multibyte: bool) {
        self.target_multibyte = target_multibyte;
    }

    /// Re-derive whatever this compiled pattern precomputed from a syntax
    /// table, against `syntax`.  A pattern whose [`Self::used_syntax`] is set
    /// was compiled against the standard table; a host caching it under a
    /// buffer-local table calls this first (GNU bakes the buffer's table at
    /// compile time and keys its cache by it, search.c `compile_pattern`).
    pub fn rebind_syntax_table(&mut self, syntax: &dyn SyntaxLookup) {
        recompute_fastmap(self, syntax);
    }
}
