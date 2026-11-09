//! GNU Emacs regex engine translated to Rust.
//!
//! This is a direct translation of GNU Emacs's `regex-emacs.c` — the same
//! algorithm, same bytecode format, same semantics.  The engine compiles
//! Emacs regex patterns to bytecode and executes them with syntax-table
//! awareness, backreference support, and POSIX backtracking.
//!
//! ## Architecture
//!
//! ```text
//! Pattern string
//!     ↓
//! regex_compile()     →  CompiledPattern (bytecode + fastmap)
//!     ↓
//! re_search()         →  Find match position (uses fastmap for skipping)
//!     ↓
//! re_match_internal() →  Execute bytecode against text (backtracking)
//!     ↓
//! MatchRegisters      →  Group start/end positions
//! ```
//!
//! ## Reference
//!
//! - GNU source: `src/regex-emacs.c` (5355 lines)
//! - GNU header: `src/regex-emacs.h`
//! - GNU search: `src/search.c` (3514 lines)

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
// Shared with `dfa` and `linear`, which import them through `super`.
pub use self::matcher::fail::{MatchScratch, fail_stack_overflow_free_span};
pub use self::matcher::step::{
    match_anychar_at, match_categoryspec_at, match_charset_at, match_exactn_char_at,
    match_syntaxspec_at, match_syntaxspecset_at, re_text_char,
};
pub use self::opcode::RegexOp;
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
    matcher::fail::{fail_stack_may_overflow, fail_stack_may_overflow_with},
    matcher::{pike_match, re_match_internal},
    pattern::{LiteralPrefilter, RewindView},
    scan::{FoldedScan, SparseAsciiFastmap, TextRepr},
    search::{StartAnchor, anchor_alt_enabled, match_pattern, search_pattern, start_anchor},
    translate::CHAR_TABLE_WALK_MIN_SPAN,
};
#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
pub use crate::case_table::{CaseTable, EmacsText, emacs_downcase_compat};
#[cfg(any(test, feature = "test-hooks", feature = "fuzzing"))]
pub use crate::syntax_class::SyntaxClass;

use self::class::posix_class_bits_read_syntax;
#[cfg(test)]
use self::compile::seal::validate_sealed_buffer;
use self::matcher::fail::{fail_stack_push_sites, matcher_overflow_pending, note_search_work};
use self::matcher::re_match_candidate_in;
#[cfg(any(test, feature = "test-hooks"))]
use self::matcher::route::MATCHER_ENTRY_COUNT;
#[cfg(test)]
use self::matcher::route::{force_backtrack, force_pike, take_pike_fallback};
use self::matcher::step::{SyntaxAssertion, evaluate_syntax_assertion, re_prev_char_start};
#[cfg(test)]
use self::opcode::store_number;
use self::opcode::{extract_number, extract_number_u16, opcode_len};
#[cfg(test)]
use self::scan::fold_prefix_literals;
#[cfg(test)]
use self::search::fastmap_force_disabled;
use self::search::{regex_knob_off, regex_knob_on};
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
