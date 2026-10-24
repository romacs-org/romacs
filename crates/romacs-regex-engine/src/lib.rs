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
//! changing a caller.  The test and fuzzing seams that DO reach those
//! internals live in [`internals`], which exists only with the `test-hooks`
//! or `fuzzing` feature (never in a shipping build).

// The API seal, checked by doctests in builds without the seams (with them,
// these items are reachable on purpose).
#![cfg_attr(
    not(any(feature = "test-hooks", feature = "fuzzing")),
    doc = r#"
## The seal

The compiled-pattern layout is not reachable:

```compile_fail,E0616
let pattern = romacs_regex_engine::regex_compile("a", false, false).unwrap();
let _ = pattern.fastmap_accurate;
```

and neither are the engine's internals:

```compile_fail,E0433
let _ = romacs_regex_engine::internals::dfa::DfaMode::On;
```
"#
)]

/// Make an associated function crate-private, or public to the test/fuzz
/// seams (`test-hooks` / `fuzzing`, and this crate's own tests).  Used for
/// methods of public types that expose engine internals.
macro_rules! seam {
    ($(#[$meta:meta])* fn $($rest:tt)*) => {
        #[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
        $(#[$meta])*
        pub fn $($rest)*
        #[cfg(not(any(test, feature = "test-hooks", feature = "fuzzing")))]
        $(#[$meta])*
        pub(crate) fn $($rest)*
    };
}

pub mod case_table;
pub mod emacs_char;
mod engine;
pub mod host;
pub mod syntax_class;
#[cfg(test)]
mod test_paths;
mod tls_scope;

#[cfg(test)]
use test_paths::{emacs_core, test_utils};

pub use case_table::{CaseTable, EmacsStr, EmacsText, emacs_downcase_compat};
pub use syntax_class::{
    SyntaxClass, standard_syntax_class_for_char, standard_syntax_class_for_code,
};

// Compile.
pub use engine::{
    CompiledPattern, RegexCompileError, regex_compile, regex_compile_lisp,
    regex_compile_lisp_with_translation,
};
// Match.
pub use engine::{
    MATCHER_OVERFLOW_MESSAGE, MatchRegisters, pattern_max_match_chars, re_match, re_search,
    take_matcher_overflow,
};
// Host boundary: syntax and case.
pub use engine::{
    AsciiPreimage, CaseTranslation, DefaultSyntaxLookup, LookupClassKey, SyntaxCacheKey,
    SyntaxLookup, default_char_has_category,
};

/// The engine's internals, for a host's white-box tests and the differential
/// fuzzer only: engine routing overrides, matcher counters, the fail-stack
/// probe, the existence DFA, and the compiled-pattern layout.  These are the
/// backtracker's, and go away with it.
#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
#[doc(hidden)]
pub mod internals {
    pub use crate::engine::*;
}
