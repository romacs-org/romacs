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

pub(crate) mod class;
pub(crate) mod filter;
pub(crate) mod lazy;
pub(crate) mod nfa;

#[cfg(test)]
// Moved unchanged from romacs-core, which does not run Clippy on its tests.
#[allow(clippy::unusual_byte_groupings, clippy::needless_range_loop)]
mod tests;
