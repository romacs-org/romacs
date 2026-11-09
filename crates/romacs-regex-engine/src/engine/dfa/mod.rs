//! An anchored EXISTENCE lazy DFA in front of the backtracker (P3.3).
//!
//! The DFA answers one question per search candidate `p`: can any match start
//! at `p` and end by `stop`?  Existence is a property of the pattern's regular
//! language alone -- GNU's leftmost-first priority, greedy vs non-greedy,
//! POSIX-longest and the submatch registers never enter it -- so a DFA over
//! the same bytecode, stepping the same per-character tests, answers it
//! exactly.  A "no" lets `re_search` skip the candidate without entering the
//! backtracker; a "yes" or "unknown" runs the unchanged backtracker, which
//! produces GNU's registers, overflow, quit and Pike behaviour.
//!
//! Layers, one module each (`nfa`, `class`, `lazy`, `filter`):
//! * the NFA ([`Nfa`](nfa::Nfa)): the consuming positions of the rewind view
//!   ([`RewindView`](crate::engine::pattern::RewindView)), the per-character test each runs, and the epsilon
//!   closure, whose zero-width assertions are evaluated at the real text
//!   position by the matcher's own tests;
//! * character classes: the partition of characters by the vector of those
//!   tests' results;
//! * the lazy DFA: states are (NFA kernel, facts about the previous
//!   character), transitions are cached by (state, class);
//! * the candidate filter ([`DfaLease`]): `re_search` asks the DFA at each
//!   candidate; a rejection skips the matcher unless the fail-stack bound
//!   says the backtracker could have overflowed there;
//! * `syntax-table` property runs and the lazy `syntax-propertize` frontier
//!   ([`ExistenceDfa::begin_search`](lazy::ExistenceDfa::begin_search)): where properties may apply, the DFA
//!   reads every syntax through the matcher's own lookup at the real
//!   position, steps its cached loop only through property-free stretches
//!   ([`SyntaxLookup::plain_syntax_until`](crate::engine::syntax::SyntaxLookup::plain_syntax_until)), classifies a character inside a
//!   run at its position ([`CharClasses::class_at_position`](class::CharClasses::class_at_position)), and leaves a
//!   candidate that reaches the frontier to the matcher, which records the
//!   read.
//!
//! No Lisp `Value` is stored here: only ids, byte tables and identity bits.
//!
//! # Knobs (read once per process)
//!
//! | Knob | Values (default) | Effect |
//! |------|------------------|--------|
//! | `ROMACS_REGEX_DFA` | `on` (default), `off`, `verify` | Candidate existence filter ([`DfaMode`]). |
//! | `ROMACS_REGEX_DFA_COLD` | `on` (default), `off` | Defer the lease; admit two nonempty failures during a scan, or one when the whole search fails ([`cold_path_enabled`]). |
//! | `ROMACS_REGEX_DFA_STATS` | unset (default), `1` | Print this thread's [`DfaStats`](filter::DfaStats) on stderr at exit with the filter on. |
//! | `ROMACS_REGEX_DFA_FIRST_STEP` | `on` (default), `off` | Reject cached prefixes of at most eight bytes inline; verify also checks the predicate against the matcher ([`first_step_enabled`](filter::first_step_enabled)). |

mod class;
mod filter;
mod lazy;
mod nfa;

// What `re_search` and the compiled pattern use.
pub use self::filter::{ColdSearchHeat, DfaCell, DfaLease, DfaMode, cold_path_enabled, dfa_mode};

#[cfg(any(test, feature = "test-hooks"))]
pub use self::filter::reset_dfa_stats;
// The layers, for the test and fuzzing seams.
#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
pub use self::{
    class::{
        BaseTableView, CharClasses, ClassContext, ClassKey, Facts, MAX_CLASSES, NO_FACTS,
        TooManyClasses, UNKNOWN_CLASS,
    },
    filter::{
        COLD_THRESHOLD, DfaSlot, DfaStats, LiveDfa, dfa_stats, first_step_enabled, prime,
        with_cold_path, with_dfa_mode, with_first_step,
    },
    lazy::{DfaCounters, DfaGaveUp, ExistenceDfa, Exists, SearchSyntax},
    nfa::{
        Assertions, ClosureScratch, DfaIneligible, KernelItem, Nfa, Place, Position, Predicate,
        dfa_eligibility, kernel_item,
    },
};

#[cfg(test)]
use self::lazy::{MATCH, MEMORY_CAP, SLOW, UNKNOWN};
#[cfg(test)]
use crate::SyntaxClass;
#[cfg(test)]
use crate::emacs_char;
#[cfg(test)]
use crate::engine::matcher::fail::fail_stack_overflow_free_span;
#[cfg(test)]
use crate::engine::matcher::step::re_text_char;
#[cfg(test)]
use crate::engine::opcode::RegexOp;
#[cfg(test)]
use crate::engine::pattern::CompiledPattern;
#[cfg(test)]
use crate::engine::syntax::{SyntaxCacheKey, SyntaxLookup};

#[cfg(test)]
// Moved unchanged from romacs-core, which does not run Clippy on its tests.
#[allow(clippy::unusual_byte_groupings, clippy::needless_range_loop)]
mod tests;
