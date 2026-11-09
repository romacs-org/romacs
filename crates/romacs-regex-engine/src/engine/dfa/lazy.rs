//! The lazy DFA: states are an NFA kernel plus facts about the previous
//! character, and transitions are cached per (state, class) as they are
//! first taken, within a memory cap.
//!
//! GNU has no counterpart.

use crate::engine::dfa::class::{
    BaseTableView, CharClasses, Facts, NO_FACTS, TooManyClasses, UNKNOWN_CLASS,
};
use crate::engine::dfa::filter::first_step_enabled;
use crate::engine::dfa::nfa::{Assertions, ClosureScratch, KernelItem, Nfa, Place, kernel_item};
use crate::engine::pattern::CompiledPattern;
use crate::engine::syntax::SyntaxLookup;
use rustc_hash::FxHashMap;

/// The DFA's verdict on one candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exists {
    /// No match starts at the candidate: every path died having consumed at
    /// most `consumed` bytes (the backtracker's fail stack is bounded by it).
    No { consumed: usize },
    /// Some match starts at the candidate.
    Yes,
    /// Not decided (quit pending, or a state cap was hit): run the matcher.
    Unknown,
}

/// Transition-table entries below the stride are sentinels; real entries are
/// row offsets, `(state index + 1) * stride`.
pub(crate) const UNKNOWN: u32 = 0;
const DEAD: u32 = 1;
pub(crate) const MATCH: u32 = 2;
/// The transition depends on the character pair (two word constituents, one
/// above U+00FF): computed at every use, never cached.
pub(crate) const SLOW: u32 = 3;
const MIN_STRIDE_SHIFT: u32 = 3;

/// The cache is cleared at the next candidate once it holds this many bytes.
pub(crate) const MEMORY_CAP: usize = 64 * 1024;
/// Within one candidate the cache may grow to this before the candidate is
/// left undecided.
const MEMORY_HARD_CAP: usize = 4 * MEMORY_CAP;
/// A quit is polled every this many bytes stepped.
const QUIT_POLL_BYTES: usize = 64 * 1024;
/// Bound the duplicated work for a cached prefix that eventually succeeds.
const CACHED_PREFIX_BYTES: usize = 8;

/// A DFA state: the NFA kernel (where threads resume, before the closure)
/// and the facts about the character before the position.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct StateKey {
    kernel: Box<[KernelItem]>,
    prev: Facts,
}

/// Counters of one DFA (reported under `ROMACS_REGEX_DFA_STATS`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DfaCounters {
    pub yes: u64,
    pub no: u64,
    pub unknown: u64,
    pub states: u64,
    pub clears: u64,
    pub slow_transitions: u64,
    pub bytes: u64,
    /// Characters classified at their position, inside a `syntax-table`
    /// property run.
    pub positional_chars: u64,
    /// Property-free stretches looked up (`plain_syntax_until`).
    pub plain_runs: u64,
    /// Candidates left to the matcher because the DFA would have read syntax
    /// at or past the lazy `syntax-propertize` frontier.
    pub frontier_unknown: u64,
}

/// What a search's syntax lookup means for the DFA, fixed for the search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchSyntax {
    /// The pattern reads syntax and `syntax-table` properties may apply: the
    /// cached loop stops at property runs, and syntax is read through the
    /// real lookup at the real position.
    pub positional: bool,
    /// The first input position whose syntax the matcher records for lazy
    /// `syntax-propertize` (`usize::MAX`: none).  A candidate the DFA cannot
    /// decide below it is left to the matcher.
    pub read_limit: usize,
}

impl SearchSyntax {
    const PLAIN: Self = Self {
        positional: false,
        read_limit: usize::MAX,
    };
}

/// Why a DFA gave up on its pattern for good.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DfaGaveUp {
    TooManyClasses,
    /// The cache kept overflowing (4 clears in one search).
    StateExplosion,
}

/// The anchored existence DFA of one pattern (see the module docs).
pub struct ExistenceDfa {
    nfa: Nfa,
    pub classes: CharClasses,
    /// Facts a state keeps about the previous character: the class facts
    /// plus [`Facts::EDGE`] when some assertion tests the text start.
    prev_mask: Facts,
    pub(crate) stride_shift: u32,
    pub(crate) trans: Vec<u32>,
    states: Vec<StateKey>,
    state_ids: FxHashMap<StateKey, u32>,
    /// Start state index + 1 per previous-character facts (0: not built).
    pub(crate) start: [u32; 32],
    scratch: ClosureScratch,
    positions: Vec<u16>,
    kernel: Vec<KernelItem>,
    pub(crate) memory: usize,
    clears_this_search: u32,
    pub search: SearchSyntax,
    /// A property-free input stretch this search has looked up.
    pub plain: std::ops::Range<usize>,
    /// Whether the pattern tests point (`\=`).
    has_at_dot: bool,
    pub counters: DfaCounters,
    pub(crate) gave_up: Option<DfaGaveUp>,
}

impl ExistenceDfa {
    pub fn new(nfa: Nfa) -> Self {
        let mask = nfa.fact_mask();
        let reads_edge = nfa.assertions.intersects(
            Assertions::BEG_LINE | Assertions::BEG_BUF | Assertions::WORD | Assertions::SYMBOL,
        );
        let prev_mask = if reads_edge { mask | Facts::EDGE } else { mask };
        let mut dfa = Self {
            classes: CharClasses::new(mask),
            nfa,
            prev_mask,
            stride_shift: MIN_STRIDE_SHIFT,
            trans: Vec::new(),
            states: Vec::new(),
            state_ids: FxHashMap::default(),
            start: [0; 32],
            scratch: ClosureScratch::default(),
            positions: Vec::new(),
            kernel: Vec::new(),
            memory: 0,
            clears_this_search: 0,
            search: SearchSyntax::PLAIN,
            plain: 0..0,
            has_at_dot: false,
            counters: DfaCounters::default(),
            gave_up: None,
        };
        dfa.has_at_dot = dfa.nfa.assertions.contains(Assertions::AT_DOT);
        dfa.trans.resize(dfa.stride(), UNKNOWN);
        dfa
    }

    pub fn nfa(&self) -> &Nfa {
        &self.nfa
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn classes_mut(&mut self) -> &mut CharClasses {
        &mut self.classes
    }

    /// Why this DFA stopped deciding for good, if it did.
    pub fn gave_up(&self) -> Option<DfaGaveUp> {
        self.gave_up
    }

    /// Start a search of `pattern` under `syntax`: per-search limits reset,
    /// and what the lookup means for the DFA is read once.  Every
    /// [`Self::anchored_exists`] until the next call must pass the same
    /// lookup.
    pub fn begin_search(&mut self, pattern: &CompiledPattern, syntax: &dyn SyntaxLookup) {
        self.clears_this_search = 0;
        self.plain = 0..0;
        self.search = SearchSyntax {
            positional: self.nfa.reads_syntax && syntax.position_dependent(),
            // The matcher records syntax reads past the frontier only for a
            // pattern that reads syntax (`uses_syntax`, the builtin's test).
            read_limit: if pattern.uses_syntax {
                syntax.syntax_read_limit()
            } else {
                usize::MAX
            },
        };
    }

    #[inline]
    fn stride(&self) -> usize {
        1 << self.stride_shift
    }

    #[inline]
    fn row_of(&self, index: u32) -> u32 {
        (index + 1) << self.stride_shift
    }

    #[inline]
    fn index_of(&self, row: u32) -> u32 {
        (row >> self.stride_shift) - 1
    }

    /// Drop every state and transition, keeping the NFA and the classes.
    pub(crate) fn clear_states(&mut self) {
        self.states.clear();
        self.state_ids.clear();
        self.trans.clear();
        self.trans.resize(self.stride(), UNKNOWN);
        self.start = [0; 32];
        self.memory = 0;
        self.counters.clears += 1;
        self.clears_this_search += 1;
        if self.clears_this_search >= 4 {
            self.gave_up = Some(DfaGaveUp::StateExplosion);
        }
    }

    /// Widen the rows when the classes outgrow them (all transitions are
    /// dropped: they are relearned lazily).
    fn fit_classes(&mut self) {
        let classes = self.classes.len();
        if classes <= self.stride() {
            return;
        }
        while (1usize << self.stride_shift) < classes {
            self.stride_shift += 1;
        }
        self.trans.clear();
        self.trans
            .resize((self.states.len() + 1) << self.stride_shift, UNKNOWN);
        self.memory = self.trans.len() * 4
            + self
                .states
                .iter()
                .map(|state| state.kernel.len() * 4 + 48)
                .sum::<usize>();
    }

    /// The row of the state `(kernel, prev)`, added if new.
    fn intern_state(&mut self, kernel: &[KernelItem], prev: Facts) -> u32 {
        let key = StateKey {
            kernel: kernel.into(),
            prev,
        };
        if let Some(&index) = self.state_ids.get(&key) {
            return self.row_of(index);
        }
        let index = self.states.len() as u32;
        self.memory += key.kernel.len() * 4 + 48 + self.stride() * 4;
        self.states.push(key.clone());
        self.state_ids.insert(key, index);
        self.trans.resize(self.trans.len() + self.stride(), UNKNOWN);
        self.counters.states += 1;
        self.row_of(index)
    }

    fn start_row(&mut self, prev: Facts) -> u32 {
        let slot = prev.bits() as usize;
        match self.start[slot] {
            0 => {
                let row = self.intern_state(&[kernel_item(0, 0)], prev);
                self.start[slot] = self.index_of(row) + 1;
                row
            }
            index => self.row_of(index - 1),
        }
    }

    /// Whether a transition out of a state with previous-character `prev`
    /// on a character of class `class` depends on the character pair.
    #[inline]
    fn pair_dependent(&self, prev: Facts, class: u8) -> bool {
        if !self.nfa.assertions.intersects(Assertions::WORD) {
            return false;
        }
        let current = self.classes.facts(class);
        prev.contains(Facts::WORD)
            && current.contains(Facts::WORD)
            && (prev.contains(Facts::WIDE_WORD) || current.contains(Facts::WIDE_WORD))
    }

    /// Compute the transition out of the state at `row` on the character of
    /// class `class` at `at.d`: the closure at `at`, then every consuming
    /// position whose predicate accepts the class.  Cached unless `cache` is
    /// false or the pair decides it.
    fn transition(&mut self, row: u32, class: u8, at: &Place<'_>, cache: bool) -> u32 {
        let index = self.index_of(row) as usize;
        let prev = self.states[index].prev;
        let kernel = std::mem::take(&mut self.kernel);
        let mut positions = std::mem::take(&mut self.positions);
        positions.clear();
        let accept = {
            let state = &self.states[index];
            self.nfa
                .closure(&state.kernel, at, &mut self.scratch, &mut positions)
        };
        let result = if accept {
            self.kernel = kernel;
            MATCH
        } else {
            let mut next = kernel;
            next.clear();
            let key = self.classes.key(class);
            for &position in &positions {
                let position = self.nfa.positions[position as usize];
                if key.accepts(position.predicate) {
                    next.push(position.next);
                }
            }
            next.sort_unstable();
            next.dedup();
            let result = if next.is_empty() {
                DEAD
            } else {
                let facts = self.classes.facts(class);
                let facts = Facts(facts.bits() & self.prev_mask.bits());
                self.intern_state(&next, facts)
            };
            self.kernel = next;
            result
        };
        self.positions = positions;
        let slow = self.pair_dependent(prev, class);
        if slow {
            self.counters.slow_transitions += 1;
        }
        if cache {
            let entry = &mut self.trans[row as usize + class as usize];
            *entry = if slow { SLOW } else { result };
        }
        result
    }

    /// The closure of the state at `row` at `at`, and the consuming step on
    /// the character there, never cached: a special position (point for a
    /// `\=` pattern).  Returns the next row, `MATCH` or `DEAD`.
    fn special_transition(&mut self, row: u32, class: u8, at: &Place<'_>) -> u32 {
        self.transition(row, class, at, false)
    }

    /// Whether the state at `row`, at `at` (the match stop), accepts.
    fn accepts_at(&mut self, row: u32, at: &Place<'_>) -> bool {
        let index = self.index_of(row) as usize;
        let mut positions = std::mem::take(&mut self.positions);
        positions.clear();
        let accept = self.nfa.closure(
            &self.states[index].kernel,
            at,
            &mut self.scratch,
            &mut positions,
        );
        self.positions = positions;
        accept
    }

    /// Can a match of `pattern` start at `p` and end by `stop`?  The same
    /// question the backtracker answers at the candidate, without registers.
    /// Inlined into the out-of-line [`DfaLease::candidate`](crate::engine::dfa::filter::DfaLease::candidate): a rejection the
    /// cached loop settles costs one call from `re_search`.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn anchored_exists(
        &mut self,
        pattern: &CompiledPattern,
        text: &[u8],
        p: usize,
        stop: usize,
        point: usize,
        syntax: &dyn SyntaxLookup,
    ) -> Exists {
        self.anchored_exists_inner::<true>(pattern, text, p, stop, point, syntax)
    }

    /// The candidate path after a caller's inline probe omits that probe
    /// here (`FIRST_STEP == false`); direct and verify calls retain it.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn anchored_exists_inner<const FIRST_STEP: bool>(
        &mut self,
        pattern: &CompiledPattern,
        text: &[u8],
        p: usize,
        stop: usize,
        point: usize,
        syntax: &dyn SyntaxLookup,
    ) -> Exists {
        if self.gave_up.is_some() {
            return Exists::Unknown;
        }
        if self.memory > MEMORY_CAP {
            self.clear_states();
            if self.gave_up.is_some() {
                return Exists::Unknown;
            }
        }
        let cached = if FIRST_STEP && first_step_enabled() {
            self.cached_prefix_rejection(pattern, text, p, stop, point, usize::MAX)
        } else {
            None
        };
        let verdict = if let Some(consumed) = cached {
            self.counters.bytes += consumed as u64;
            Exists::No { consumed }
        } else {
            self.run(pattern, text, p, stop, point, syntax)
        };
        match verdict {
            Exists::Yes => self.counters.yes += 1,
            Exists::No { .. } => self.counters.no += 1,
            Exists::Unknown => self.counters.unknown += 1,
        }
        verdict
    }

    /// A bounded cached rejection, without classifying, interning, or reading
    /// syntax. Called only after the give-up and memory-cap guards. Returns
    /// the same consumed span as `run`, strictly below `overflow_free_span`.
    ///
    /// Threading: this cache and its compiled pattern are mutator-owned, not
    /// shared concurrently. `DfaLease::acquire` synchronizes character maps
    /// with the search context before calling this helper. A context change
    /// clears those maps; the context-independent transitions remain valid.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn cached_prefix_rejection(
        &self,
        pattern: &CompiledPattern,
        text: &[u8],
        p: usize,
        stop: usize,
        point: usize,
        overflow_free_span: usize,
    ) -> Option<usize> {
        // Establish memory safety first. The remaining guards apply only
        // once an existing DEAD entry could actually reject this candidate.
        if p >= text.len() {
            return None;
        }
        let class = self.classes.byte_class[text[p] as usize];
        if class == UNKNOWN_CLASS {
            return None;
        }
        let prev_mask = self.prev_mask.0;
        let reads_prev_syntax = prev_mask & !(Facts::EDGE.0 | Facts::NEWLINE.0) != 0;
        let facts = if prev_mask == 0 {
            // No assertion reads the previous character: use start[0]
            // without inspecting point zero or the preceding byte.
            0
        } else if p == 0 {
            Facts::EDGE.0
        } else if !reads_prev_syntax {
            // EDGE and newline facts do not depend on syntax or decoding.
            if text[p - 1] == b'\n' {
                Facts::NEWLINE.0
            } else {
                0
            }
        } else {
            let byte = text[p - 1];
            if pattern.target_multibyte && byte >= 0x80 {
                return None;
            }
            let prev_class = self.classes.byte_class[byte as usize];
            if prev_class != UNKNOWN_CLASS {
                self.classes.facts(prev_class).0
            } else {
                let facts = self.classes.byte_facts[byte as usize];
                if facts == NO_FACTS {
                    return None;
                }
                facts
            }
        };
        let start = self.start[(facts & prev_mask) as usize];
        if start == 0 {
            return None;
        }
        let mut row = start << self.stride_shift;
        // Every consumed byte must be before the stop, frontier and point's
        // special closure, and within already established plain syntax.
        // Eight probes cover short literal failures without making a true
        // candidate walk an unbounded prefix twice.
        let mut limit = stop
            .min(text.len())
            .min(self.search.read_limit)
            .min(p.saturating_add(CACHED_PREFIX_BYTES))
            .min(p.saturating_add(overflow_free_span));
        if self.has_at_dot && point >= p {
            limit = limit.min(point);
        }
        if self.search.positional {
            // The current class is a base-table class. Previous word/symbol
            // facts require base syntax at p - 1 too, in the same known run.
            let from = if p > 0 && reads_prev_syntax { p - 1 } else { p };
            if from < self.plain.start || p >= self.plain.end {
                return None;
            }
            limit = limit.min(self.plain.end);
        }
        let stride = 1u32 << self.stride_shift;
        let mut d = p;
        while d < limit {
            let class = if d == p {
                class
            } else {
                self.classes.byte_class[text[d] as usize]
            };
            if class == UNKNOWN_CLASS {
                return None;
            }
            let at = row as usize + class as usize;
            debug_assert!(at < self.trans.len());
            // SAFETY: as in `run`, cached rows have a complete stride and
            // known byte classes are below that stride. This path mutates
            // neither rows nor classes.
            let next = unsafe { *self.trans.get_unchecked(at) };
            if next == DEAD {
                return Some(d - p);
            }
            if next < stride {
                return None;
            }
            row = next;
            d += 1;
        }
        None
    }

    /// The one-step subset is kept explicit in the cache guard tests.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn cached_first_step_dead(
        &self,
        pattern: &CompiledPattern,
        text: &[u8],
        p: usize,
        stop: usize,
        point: usize,
    ) -> bool {
        self.cached_prefix_rejection(pattern, text, p, stop, point, 1) == Some(0)
    }

    /// The end of the property-free stretch from `at` (see
    /// [`SyntaxLookup::plain_syntax_until`]), remembered for the search: the
    /// candidates of a scan mostly fall into the stretch the last one did.
    #[inline]
    fn plain_until(&mut self, at: usize, syntax: &dyn SyntaxLookup) -> usize {
        if self.plain.start <= at && at < self.plain.end {
            return self.plain.end;
        }
        self.counters.plain_runs += 1;
        let end = syntax.plain_syntax_until(at);
        if end > at {
            self.plain = at..end;
        }
        end
    }

    /// The start state at candidate `p` and the end of the property-free
    /// stretch the cached loop may run in from `p`: the previous character's
    /// facts come from its memoized class when the base table applies there.
    #[inline(always)]
    fn start_at(
        &mut self,
        text: &[u8],
        p: usize,
        target_multibyte: bool,
        syntax: &dyn SyntaxLookup,
    ) -> (u32, usize) {
        let positional = self.search.positional;
        if p == 0 {
            let plain_end = if positional {
                self.plain_until(0, syntax)
            } else {
                usize::MAX
            };
            return (
                self.start_row(Facts(Facts::EDGE.0 & self.prev_mask.0)),
                plain_end,
            );
        }
        let byte = text[p - 1];
        if byte < 0x80 || !target_multibyte {
            // A single-byte previous character at `p - 1`: one lookup says
            // both whether its memoized class applies and how far the
            // stretch runs (one that ends at `p` is renewed there).
            let (plain_end, prev_plain) = if positional {
                let end = self.plain_until(p - 1, syntax);
                (end, end > p - 1)
            } else {
                (usize::MAX, true)
            };
            if prev_plain {
                let base = BaseTableView(syntax);
                let facts = self
                    .classes
                    .byte_facts_at(text, p - 1, target_multibyte, &base);
                return (self.start_row(Facts(facts.0 & self.prev_mask.0)), plain_end);
            }
            return (
                self.start_row_slow(text, p, target_multibyte, syntax),
                plain_end,
            );
        }
        let plain_end = if positional {
            self.plain_until(p, syntax)
        } else {
            usize::MAX
        };
        (
            self.start_row_slow(text, p, target_multibyte, syntax),
            plain_end,
        )
    }

    /// The start state at `p` from the previous character read at its
    /// position (the lookup the closures use).
    #[cold]
    #[inline(never)]
    fn start_row_slow(
        &mut self,
        text: &[u8],
        p: usize,
        target_multibyte: bool,
        syntax: &dyn SyntaxLookup,
    ) -> u32 {
        let base = BaseTableView(syntax);
        let lookup: &dyn SyntaxLookup = if self.search.positional {
            syntax
        } else {
            &base
        };
        let facts =
            match crate::engine::matcher::step::re_prev_char_start(text, p, target_multibyte) {
                None => Facts::EDGE,
                Some(start) => self.classes.facts_at(text, start, target_multibyte, lookup),
            };
        self.start_row(Facts(facts.0 & self.prev_mask.0))
    }

    /// The decision at one candidate: the start state, then the cached loop,
    /// which settles most rejections by itself (a cached dead transition);
    /// everything else resumes out of line.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        pattern: &CompiledPattern,
        text: &[u8],
        p: usize,
        stop: usize,
        point: usize,
        syntax: &dyn SyntaxLookup,
    ) -> Exists {
        let stop = stop.min(text.len());
        if p > stop {
            return Exists::No { consumed: 0 };
        }
        // Every syntax read of a candidate is at or below the furthest
        // position its threads reach, and the matcher's threads are among
        // the DFA's (the rewind view only adds paths).  So a candidate the
        // DFA decides without reaching the lazy-propertize frontier is one
        // the matcher would have failed without recording a read there; one
        // that reaches it is left to the matcher, which records its reads.
        let read_limit = self.search.read_limit;
        if p >= read_limit {
            self.counters.frontier_unknown += 1;
            return Exists::Unknown;
        }
        let (mut row, plain_end) = self.start_at(text, p, pattern.target_multibyte, syntax);
        // `\=` holds at point only: that position's closure is never cached.
        let special = if self.has_at_dot && point >= p {
            point
        } else {
            usize::MAX
        };
        let limit = stop.min(special).min(plain_end).min(read_limit);
        let chunk = limit.min(p.saturating_add(QUIT_POLL_BYTES));
        let mut d = p;
        let stride = 1u32 << self.stride_shift;
        let mut next = stride;
        {
            let byte_class = &self.classes.byte_class;
            let trans = &self.trans[..];
            let window = &text[..chunk];
            while d < window.len() {
                let class = byte_class[window[d] as usize];
                if class == UNKNOWN_CLASS {
                    next = UNKNOWN;
                    break;
                }
                let at = row as usize + class as usize;
                debug_assert!(at < trans.len());
                // SAFETY: `row` is the row of a live state (every row is
                // `stride` entries of `trans`) and a class in the byte table
                // is below `classes.len() <= stride` (`fit_classes` runs after
                // every new class).
                next = unsafe { *trans.get_unchecked(at) };
                if next < stride {
                    break;
                }
                row = next;
                d += 1;
            }
        }
        // A cached dead or accepting transition at `d` (a plain position
        // short of every special one) is the answer the slow path would
        // compute there.
        match next {
            DEAD => {
                self.counters.bytes += (d - p) as u64;
                Exists::No { consumed: d - p }
            }
            MATCH => {
                self.counters.bytes += (d - p) as u64;
                Exists::Yes
            }
            _ => self.resume(
                pattern, text, p, d, row, stop, point, special, plain_end, syntax,
            ),
        }
    }

    /// The rest of a candidate the cached loop could not settle: new
    /// classes and transitions, property runs, point, the stop, quit polls.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn resume(
        &mut self,
        pattern: &CompiledPattern,
        text: &[u8],
        p: usize,
        mut d: usize,
        mut row: u32,
        stop: usize,
        point: usize,
        mut special: usize,
        mut plain_end: usize,
        syntax: &dyn SyntaxLookup,
    ) -> Exists {
        let base = BaseTableView(syntax);
        let target_multibyte = pattern.target_multibyte;
        let search = self.search;
        // Where `syntax-table` properties may apply, every syntax the DFA
        // reads -- assertions in the closure, the previous character, a
        // character inside a property run -- is read through the matcher's
        // own lookup at the real position.  The cached loop steps only
        // through property-free stretches, whose memoized base-table classes
        // are what that lookup answers there.  A cached transition stays a
        // function of (state, class): the assertions it evaluated read only
        // the facts both carry, and a class read at a position is interned
        // like any other.
        let lookup: &dyn SyntaxLookup = if search.positional { syntax } else { &base };
        let place = |d: usize| Place {
            text,
            d,
            stop,
            point,
            target_multibyte,
            syntax: lookup,
        };
        let mut polled = d - p;
        loop {
            // The cached loop: single-byte characters with a known class and
            // a cached live transition, in chunks between quit polls.
            let limit = stop.min(special).min(plain_end).min(search.read_limit);
            let chunk = limit.min(d.saturating_add(QUIT_POLL_BYTES));
            let start = d;
            {
                let byte_class = &self.classes.byte_class;
                let trans = &self.trans[..];
                let stride = 1u32 << self.stride_shift;
                let window = &text[..chunk];
                while d < window.len() {
                    let class = byte_class[window[d] as usize];
                    if class == UNKNOWN_CLASS {
                        break;
                    }
                    let at = row as usize + class as usize;
                    debug_assert!(at < trans.len());
                    // SAFETY: as in `run`.
                    let next = unsafe { *trans.get_unchecked(at) };
                    if next < stride {
                        break;
                    }
                    row = next;
                    d += 1;
                }
            }
            polled += d - start;
            if polled >= QUIT_POLL_BYTES {
                polled = 0;
                if crate::host::quit_pending() {
                    return Exists::Unknown;
                }
            }
            if d >= search.read_limit {
                self.counters.bytes += (d - p) as u64;
                self.counters.frontier_unknown += 1;
                return Exists::Unknown;
            }
            if d >= stop {
                self.counters.bytes += (d - p) as u64;
                return if self.accepts_at(row, &place(d)) {
                    Exists::Yes
                } else {
                    Exists::No { consumed: d - p }
                };
            }
            if d == chunk && d < limit {
                continue;
            }
            // Past the property-free stretch: the next one, or a character
            // inside a property run.
            let mut at_position = false;
            if d >= plain_end {
                plain_end = self.plain_until(d, syntax);
                if plain_end > d {
                    continue;
                }
                at_position = true;
            }
            // One character the cached loop could not take.
            let row_index = self.index_of(row);
            let classified = if at_position {
                self.counters.positional_chars += 1;
                self.classes
                    .class_at_position(&self.nfa, pattern, text, d, &base, syntax)
            } else {
                self.classes.class_at(&self.nfa, pattern, text, d, &base)
            };
            let (class, len) = match classified {
                Ok(found) => found,
                Err(TooManyClasses) => {
                    self.gave_up = Some(DfaGaveUp::TooManyClasses);
                    return Exists::Unknown;
                }
            };
            self.fit_classes();
            row = self.row_of(row_index);
            let next = if d == special {
                special = usize::MAX;
                self.special_transition(row, class, &place(d))
            } else {
                match self.trans[row as usize + class as usize] {
                    UNKNOWN => self.transition(row, class, &place(d), true),
                    SLOW => self.transition(row, class, &place(d), false),
                    cached => cached,
                }
            };
            match next {
                MATCH => {
                    self.counters.bytes += (d - p) as u64;
                    return Exists::Yes;
                }
                DEAD => {
                    self.counters.bytes += (d - p) as u64;
                    return Exists::No { consumed: d - p };
                }
                live => {
                    if d + len > stop {
                        // The character straddles the stop: no path that
                        // consumed it can end or go on.
                        self.counters.bytes += (d + len - p) as u64;
                        return Exists::No {
                            consumed: d + len - p,
                        };
                    }
                    row = live;
                    d += len;
                }
            }
            if self.memory > MEMORY_HARD_CAP {
                return Exists::Unknown;
            }
        }
    }
}
