//! GNU Emacs regular expressions, without an evaluator.
//!
//! The matcher (`engine`, a translation of GNU `regex-emacs.c`) compiles a
//! pattern given as internal-representation bytes and runs it over a byte
//! slice of internal-representation text.  Everything it needs to know about
//! its host goes through plain-Rust traits:
//!
//! - [`SyntaxLookup`]: syntax classes (per character and, for `syntax-table`
//!   text properties, per position), categories, word boundaries, and the
//!   identities/ticks host caches are keyed by;
//! - [`CaseTable`]: a case-canon table, wrapped in a [`CaseTranslation`];
//! - [`EmacsText`]: a pattern's bytes and multibyteness;
//! - [`host::set_quit_probe`]: the per-thread `C-g` poll.
//!
//! No Lisp value crosses these traits, so nothing the engine retains can be a
//! stale handle into a moving heap.
//!
//! ## Linear time
//!
//! GNU's backtracker is exponential on patterns that can match the same text
//! many ways (`\(a*\)*`), and trying candidates one by one makes a failing
//! scan quadratic even when each attempt is linear.  The engine therefore
//! carries a linear-time Pike VM (`engine/linear.rs`) that reproduces the
//! backtracker's leftmost-first answer and registers: it takes over a
//! candidate whose backtracking turns catastrophic, and the rest of a forward
//! search whose candidates cost more than the text justifies.  Patterns it
//! cannot run (backreferences, interval counters, `??`, nullable non-greedy
//! loops, POSIX) stay on the backtracker, decided at compile time.  The one
//! observable difference is recorded in `GNU-DIVERGENCES.md` (RX1).
//!
//! ## The API is the seam
//!
//! The public surface is deliberately engine-agnostic: compile
//! ([`regex_compile_lisp_with_translation`] and friends) to an opaque
//! [`CompiledPattern`], then [`re_search`] / [`re_match`] for
//! [`MatchRegisters`] (per-group byte spans).  Nothing here exposes the
//! backtracker's opcode buffer, failure stack, fastmap, or pattern layout, so
//! a different matcher can replace `engine` behind these signatures without
//! changing a caller.  The crate's own tests reach the internals from
//! inside it.

// Every unsafe block says why it holds (the unsafe census keeps the count).
#![deny(clippy::undocumented_unsafe_blocks)]
// The API seal, checked by doctests.
#![doc = r#"
## The seal

The compiled-pattern layout is not reachable:

```compile_fail,E0616
let pattern = romacs_regex::regex_compile("a", false, false).unwrap();
let _ = pattern.fastmap_accurate;
```
"#]

pub mod case_table;
pub mod emacs_char;
mod engine;
pub mod host;
pub mod syntax_class;
#[cfg(test)]
mod test_utils;
mod tls_scope;

pub use case_table::{CaseTable, EmacsStr, EmacsText, emacs_downcase_compat};
pub use syntax_class::{
    SyntaxClass, standard_syntax_class_for_char, standard_syntax_class_for_code,
};

// Compile.
pub use engine::compile::{
    RegexCompileError, regex_compile, regex_compile_lisp, regex_compile_lisp_with_translation,
};
pub use engine::pattern::{CompiledPattern, MatchRegisters};
// Match.
pub use engine::matcher::fail::{MATCHER_OVERFLOW_MESSAGE, take_matcher_overflow};
pub use engine::matcher::re_match;
pub use engine::optimize::pattern_max_match_chars;
pub use engine::search::re_search;
// Host boundary: syntax and case.
pub use engine::class::{char_class_bit, char_in_classes};
pub use engine::syntax::{
    DefaultSyntaxLookup, LookupClassKey, SyntaxCacheKey, SyntaxLookup, default_char_has_category,
};
pub use engine::translate::{AsciiPreimage, CaseTranslation};
