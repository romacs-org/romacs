//! GNU Emacs regular expressions: GNU's `regex-emacs.c`, translated, plus
//! the linear-time matchers that keep its answers without its blow-ups.
//!
//! A pattern compiles to GNU's bytecode ([`regex_compile`](crate::regex_compile) to a
//! [`CompiledPattern`](crate::CompiledPattern)); [`re_search`](crate::re_search) tries candidate starts and
//! [`re_match`](crate::re_match) runs the backtracker from one, filling [`MatchRegisters`](crate::MatchRegisters).
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

pub(crate) mod class;
pub(crate) mod compile;
pub(crate) mod dfa;
pub(crate) mod fastmap;
pub(crate) mod fusion;
pub(crate) mod linear;
pub(crate) mod matcher;
pub(crate) mod opcode;
pub(crate) mod optimize;
pub(crate) mod pattern;
pub(crate) mod scan;
pub(crate) mod search;
pub(crate) mod syntax;
pub(crate) mod translate;

#[cfg(test)]
#[path = "tests/short_literal_test.rs"]
mod short_literal_tests;

#[cfg(test)]
#[path = "tests/emacs_test.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/interval_test.rs"]
mod interval_tests;

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
