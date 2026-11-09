//! GNU Emacs regular expressions: GNU's `regex-emacs.c`, translated, plus
//! the linear-time matchers that keep its answers without its blow-ups.
//!
//! A pattern compiles to GNU's bytecode ([`regex_compile`] to a
//! [`CompiledPattern`]); [`re_search`] tries candidate starts and
//! [`re_match`] runs the backtracker from one, filling [`MatchRegisters`].
//!
//! ## Where GNU's pieces live
//!
//! | Module | Contents | GNU regex-emacs.c |
//! |---|---|---|
//! | `opcode` | opcodes, operand encoding | `re_opcode_t`, `STORE_NUMBER`, `EXTRACT_NUMBER` |
//! | `pattern` | the compiled pattern, match registers | regex-emacs.h `re_pattern_buffer`, `re_registers` |
//! | `translate` | case folding | `RE_TRANSLATE`, `TRANSLATE` |
//! | `class` | `[[:alpha:]]` classes | `re_wctype_parse`, `re_iswctype`, `re_wctype_to_bit` |
//! | `syntax` | the host's syntax and category tables | `SYNTAX`, `CHAR_HAS_CATEGORY` |
//! | `compile` | the compiler | `regex_compile`, `at_begline_loc_p`, `at_endline_loc_p` |
//! | `compile::charset` | `[...]` | `regex_compile`'s `case '['`, range tables |
//! | `compile::repeat` | `*`, `+`, `?`, `\{n,m\}` | `handle_plus`, `handle_interval` |
//! | `compile::emit` | buffer editing, jumps | `STORE_JUMP`, `INSERT_JUMP`, `insert_op1` |
//! | `compile::seal` | the matcher's proof of a buffer | none |
//! | `optimize` | loop analyses, smart jumps | `skip_one_char`, `mutually_exclusive_p` |
//! | `fusion` | syntax-class peephole | none |
//! | `fastmap` | first-byte sets | `analyze_first`, `re_compile_fastmap` |
//! | `scan` | candidate scans | `re_search_2`'s fastmap loop |
//! | `search` | the search loop | `re_search_2` |
//! | `matcher` | match entry points | `re_match_2` |
//! | `matcher::exec` | the backtracker's loop | `re_match_2_internal` |
//! | `matcher::fail` | the failure stack | `PUSH_FAILURE_POINT`, `POP_FAILURE_POINT`, `CHECK_INFINITE_LOOP` |
//! | `matcher::step` | per-character tests | `PREFETCH`, `AT_WORD_BOUNDARY`, `execute_charset` |
//! | `matcher::route` | backtracker, Pike VM or linear | none |
//! | `linear` | the linear-time Pike VM | none |
//! | `dfa` | the existence DFA | none |
//!
//! GNU's search.c (the pattern cache, `looking-at`, `string-match`) is the
//! host's: romacs-vm's `regexp.rs`.

mod class;
mod compile;
mod fastmap;
mod fusion;
mod matcher;
mod opcode;
mod optimize;
mod pattern;
mod scan;
mod search;
mod syntax;
mod translate;

// The API `lib.rs` re-exports.
pub use self::class::{char_class_bit, char_in_classes};
pub use self::compile::{
    RegexCompileError, regex_compile, regex_compile_lisp, regex_compile_lisp_with_translation,
};
pub use self::matcher::fail::{MATCHER_OVERFLOW_MESSAGE, take_matcher_overflow};
pub use self::matcher::re_match;
pub use self::optimize::pattern_max_match_chars;
pub use self::pattern::{CompiledPattern, MatchRegisters};
pub use self::search::re_search;
pub use self::syntax::{
    DefaultSyntaxLookup, LookupClassKey, SyntaxCacheKey, SyntaxLookup, default_char_has_category,
};
pub use self::translate::{AsciiPreimage, CaseTranslation};

// The internals `lib.rs` opens to the test and fuzzing seams.
#[cfg(any(test, feature = "test-hooks"))]
pub use self::fusion::with_syntax_fusion_disabled;
#[cfg(any(test, feature = "test-hooks"))]
pub use self::matcher::route::{
    FailStackProbe, PIKE_FALLBACK_COUNT, linear_search_count, matcher_entry_count,
    pike_fallback_count, take_fail_stack_probe, with_backtracker_forced, with_pike_forced,
};
#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
pub use self::matcher::route::{RegexEngineOverride, with_regex_engine_override};
#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
pub use self::search::{build_search_optimizations, with_anchor_alt, with_fastmap_disabled};
#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
pub use self::{
    fastmap::recompute_fastmap,
    matcher::fail::{
        MatchScratch, fail_stack_may_overflow, fail_stack_may_overflow_with,
        fail_stack_overflow_free_span,
    },
    matcher::step::{
        match_anychar_at, match_categoryspec_at, match_charset_at, match_exactn_char_at,
        match_syntaxspec_at, match_syntaxspecset_at, re_text_char,
    },
    matcher::{pike_match, re_match_internal},
    opcode::RegexOp,
    pattern::{LiteralPrefilter, RewindView},
    scan::{FoldedScan, SparseAsciiFastmap, TextRepr},
    search::{StartAnchor, anchor_alt_enabled, start_anchor},
    translate::CHAR_TABLE_WALK_MIN_SPAN,
};
#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
pub use crate::case_table::{CaseTable, EmacsText, emacs_downcase_compat};
#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
pub use crate::syntax_class::SyntaxClass;

// What the engine's tests (`tests/`, which glob this module) reach beyond the
// seams above.
#[cfg(test)]
use self::{
    compile::seal::validate_sealed_buffer,
    matcher::fail::fail_stack_push_sites,
    matcher::route::{force_backtrack, force_pike, take_pike_fallback},
    opcode::{extract_number, opcode_len, store_number},
    scan::fold_prefix_literals,
    search::fastmap_force_disabled,
};
#[cfg(test)]
use crate::emacs_char;

pub mod dfa;
pub mod linear;

#[cfg(test)]
#[path = "tests/short_literal_test.rs"]
mod short_literal_tests;

#[cfg(test)]
#[path = "tests/emacs_test.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/opcode_decode_test.rs"]
mod opcode_decode_tests;

#[cfg(test)]
#[path = "tests/casefold_scan_test.rs"]
mod casefold_scan_tests;

#[cfg(test)]
#[path = "tests/fail_stack_parity_test.rs"]
mod fail_stack_parity_tests;

#[cfg(test)]
#[path = "tests/start_anchor_test.rs"]
mod start_anchor_tests;

#[cfg(test)]
#[path = "tests/host_boundary_test.rs"]
mod host_boundary_tests;
#[cfg(test)]
#[path = "tests/linear_test.rs"]
mod linear_tests;
