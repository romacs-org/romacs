use crate::engine::compile::{RegexCompileError, regex_compile};
use crate::engine::fastmap::recompute_fastmap;
use crate::engine::matcher::re_match;
use crate::engine::matcher::route::{
    RegexEngineOverride, force_backtrack, force_pike, pike_fallback_count,
    with_regex_engine_override,
};
use crate::engine::opcode::RegexOp;
use crate::engine::optimize::pattern_max_match_chars;
use crate::engine::pattern::{CompiledPattern, MatchRegisters, RewindView};
use crate::engine::search::{fastmap_force_disabled, re_search, with_fastmap_disabled};
use crate::engine::syntax::{DefaultSyntaxLookup, SyntaxLookup};
// Compile + search, and compile + match, in one call.

/// Compile a pattern and search for it in text.
fn search_pattern(
    pattern_str: &str,
    text: &str,
    start: usize,
    case_fold: bool,
    syntax: &dyn SyntaxLookup,
    point: usize,
) -> Result<Option<(usize, MatchRegisters)>, RegexCompileError> {
    let compiled = regex_compile(pattern_str, false, case_fold)?;
    Ok(re_search(
        &compiled,
        text.as_bytes(),
        start,
        (text.len() - start) as isize,
        syntax,
        point,
    ))
}

/// Compile a pattern and match at a specific position.
fn match_pattern(
    pattern_str: &str,
    text: &str,
    pos: usize,
    case_fold: bool,
    syntax: &dyn SyntaxLookup,
    point: usize,
) -> Result<Option<(usize, MatchRegisters)>, RegexCompileError> {
    let compiled = regex_compile(pattern_str, false, case_fold)?;
    Ok(re_match(
        &compiled,
        text.as_bytes(),
        pos,
        text.len(),
        syntax,
        point,
    ))
}

#[test]
fn sparse_fastmap_search_agrees_with_exhaustive_candidates() {
    crate::test_utils::init_test_tracing();
    let syntax = DefaultSyntaxLookup;
    for pattern in [
        "a",
        "[ab]",
        "[abc]",
        "[abcd]",
        "[^ab]",
        "é",
        "[aé]",
        ".",
        "\\(a\\|b\\|c\\)",
        "\\(a\\|b\\|c\\|d\\)",
        "[abc]$",
        "[ab]?",
        "[ab]*c",
        "\\(a\\)\\1",
        "[ab]\\{1,2\\}",
        "\\b[abc]",
        "^a",
        "",
        "\0",
        "\u{7f}",
    ] {
        for case_fold in [false, true] {
            for posix in [false, true] {
                let compiled = regex_compile(pattern, posix, case_fold).expect("compile");
                for text in ["", "a", "zzz", "abca\nd", " éaB中c", "\0a\u{7f}b"] {
                    let positions: Vec<_> = text
                        .char_indices()
                        .map(|(pos, _)| pos)
                        .chain(std::iter::once(text.len()))
                        .collect();
                    for &start in &positions {
                        for &limit in &positions {
                            let search = || {
                                re_search(
                                    &compiled,
                                    text.as_bytes(),
                                    start,
                                    limit as isize - start as isize,
                                    &syntax,
                                    start,
                                )
                                .map(|(pos, regs)| (pos, regs.start, regs.end))
                            };
                            let expected = with_fastmap_disabled(search);
                            // Reuse the same compiled pattern over changing text,
                            // directions and bounds, exercising cached decisions.
                            assert_eq!(
                                search(),
                                expected,
                                "{pattern:?}, {text:?}, {start} -> {limit}, fold={case_fold}, posix={posix}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn sparse_fastmap_rebuild_does_not_reuse_an_old_skip_byte() {
    crate::test_utils::init_test_tracing();
    let syntax = DefaultSyntaxLookup;
    let mut compiled = regex_compile("a", false, false).expect("compile");
    let search = |pattern: &CompiledPattern, text: &[u8]| {
        re_search(pattern, text, 0, text.len() as isize, &syntax, 0)
            .map(|(pos, regs)| (pos, regs.end[0]))
    };
    assert_eq!(search(&compiled, b"xa"), Some((1, 2)));
    let original = compiled.clone();
    // Recompute rebuilds the fastmap for the current syntax table.
    // Change the literal operand without changing bytecode boundaries or
    // match registers, so any stale skip decision would miss the new byte.
    assert_eq!(&compiled.buffer[..3], &[RegexOp::Exactn as u8, 1, b'a']);
    compiled.buffer[2] = b'b';
    recompute_fastmap(&mut compiled, &syntax);
    assert_eq!(search(&compiled, b"xb"), Some((1, 2)));
    assert_eq!(search(&compiled, b"xa"), None);
    assert_eq!(search(&original, b"xa"), Some((1, 2)));
    assert_eq!(search(&original, b"xb"), None);
}

#[test]
fn bounded_search_view_one_character_context_agrees_with_full_text() {
    crate::test_utils::init_test_tracing();
    let syntax = DefaultSyntaxLookup;
    for text in ["", "aaTAIL", "aa TAIL", "a_a\nTAIL", "a\n\nz\n", "é中日\nZ"] {
        let positions: Vec<_> = text
            .char_indices()
            .map(|(pos, _)| pos)
            .chain(std::iter::once(text.len()))
            .collect();
        for pattern in [
            "",
            "a",
            "a$",
            "a\\'",
            "\\`",
            "\\'",
            "^",
            "$",
            "\\b",
            "\\B",
            "\\<",
            "\\>",
            "\\_<",
            "\\_>",
            "a.*",
            "a.*?",
            "a?",
            ".*a$",
            "\\(a\\)\\1",
            "\\(a\\|aa\\)",
            "\\(.*\\)$",
            "中\\b",
            "[[:word:]]+",
        ] {
            for case_fold in [false, true] {
                for posix in [false, true] {
                    let compiled = regex_compile(pattern, posix, case_fold).expect("compile");
                    for &start in &positions {
                        for &end in positions.iter().filter(|&&end| end >= start) {
                            // The full text is the independent oracle. The view
                            // keeps the next complete character so assertions at
                            // END see its syntax and do not see a false EOF.
                            let context_end = positions
                                .iter()
                                .copied()
                                .find(|&pos| pos > end)
                                .unwrap_or(text.len());
                            let search = |bytes| {
                                re_search(
                                    &compiled,
                                    bytes,
                                    start,
                                    (end - start) as isize,
                                    &syntax,
                                    start,
                                )
                                .map(|(pos, regs)| (pos, regs.start, regs.end))
                            };
                            assert_eq!(
                                search(&text.as_bytes()[..context_end]),
                                search(text.as_bytes()),
                                "{pattern:?} in {text:?}, {start}..={end}, fold={case_fold}, posix={posix}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn test_simple_literal() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    let result = search_pattern("hello", "say hello world", 0, false, &syn, 0);
    assert!(result.is_ok());
    let r = result.unwrap();
    assert!(r.is_some());
    let (pos, regs) = r.unwrap();
    assert_eq!(pos, 4); // "hello" starts at position 4
    assert_eq!(regs.end[0], 9); // ends at 9
}

#[test]
fn test_dot_matches_any() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    let result = search_pattern("h.llo", "say hello world", 0, false, &syn, 0);
    assert!(result.is_ok());
    let r = result.unwrap();
    assert!(r.is_some());
}

#[test]
fn test_anchors() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // ^ at beginning
    let r = match_pattern("^hello", "hello world", 0, false, &syn, 0).unwrap();
    assert!(r.is_some());
    // ^ not at beginning
    let r = match_pattern("^hello", "say hello", 4, false, &syn, 0).unwrap();
    assert!(r.is_none());
}

#[test]
fn test_groups() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    let result = search_pattern("\\(hel\\)lo", "hello", 0, false, &syn, 0);
    assert!(result.is_ok());
    let (pos, regs) = result.unwrap().unwrap();
    assert_eq!(pos, 0);
    assert_eq!(regs.start[1], 0); // group 1 start
    assert_eq!(regs.end[1], 3); // group 1 end ("hel")
}

#[test]
fn test_word_boundary() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    let r = search_pattern("\\bhello\\b", "say hello world", 0, false, &syn, 0);
    assert!(r.is_ok());
    assert!(r.unwrap().is_some());
}

#[test]
fn test_star_repetition() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    let r = search_pattern("hel*o", "heo", 0, false, &syn, 0);
    assert!(r.unwrap().is_some()); // zero l's
    let r = search_pattern("hel*o", "hello", 0, false, &syn, 0);
    assert!(r.unwrap().is_some()); // two l's
    let r = search_pattern("hel*o", "hellllo", 0, false, &syn, 0);
    assert!(r.unwrap().is_some()); // four l's
}

#[test]
fn test_nongreedy_optional_prefers_zero() {
    // Regression: non-greedy `??` used to fall through into the body first
    // (greedy order), so `a??` matched one char instead of zero.  GNU
    // `string-match` semantics — the lazy optional prefers the empty match:
    //   (string-match "a??" "aaa")   => 0, match-data (0 0)
    //   (string-match ".??" "xy")    => 0, match-data (0 0)
    //   (string-match "a??b" "ab")   => 0, match-data (0 2)   [backtracks in]
    //   (string-match "a??a" "aa")   => 0, match-data (0 1)
    //   (string-match "x*a??" "xxaa")=> 0, match-data (0 2)
    //   (string-match "\\(a??\\)b" "ab") => 0, groups (0 2)(0 1)
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;

    let matched = |pat: &str, text: &str| -> (usize, i64, i64) {
        let (pos, regs) = search_pattern(pat, text, 0, false, &syn, 0)
            .expect("compile")
            .expect("should match");
        (pos, regs.start[0], regs.end[0])
    };

    assert_eq!(matched("a??", "aaa"), (0, 0, 0), "a?? prefers empty");
    assert_eq!(matched(".??", "xy"), (0, 0, 0), ".?? prefers empty");
    assert_eq!(
        matched("a??b", "ab"),
        (0, 0, 2),
        "a??b backtracks into body"
    );
    assert_eq!(matched("a??a", "aa"), (0, 0, 1), "a??a lazy then literal");
    assert_eq!(matched("a??", ""), (0, 0, 0), "a?? on empty input");
    assert_eq!(
        matched("x*a??", "xxaa"),
        (0, 0, 2),
        "greedy x* then lazy a??"
    );

    // Captured group inside a non-greedy optional.
    let (pos, regs) = search_pattern("\\(a??\\)b", "ab", 0, false, &syn, 0)
        .expect("compile")
        .expect("should match");
    assert_eq!((pos, regs.start[0], regs.end[0]), (0, 0, 2));
    assert_eq!((regs.start[1], regs.end[1]), (0, 1), "group 1 = the 'a'");
}

#[test]
fn test_word_boundary_at_string_edges() {
    // Regression: GNU treats the beginning and end of the searched region as
    // *unconditional* word boundaries (regex-emacs.c `case wordbound`, Case 1),
    // so `\b` succeeds and `\B` fails there regardless of the adjacent char.
    // romacs previously computed the boundary from neighbours only, treating
    // the missing edge char as non-word, so `\b` wrongly failed (and `\B`
    // matched) at an edge next to a non-word char or in the empty string.
    // GNU ground truth (`string-match`):
    //   (string-match "\\b" "")     => 0     (string-match "\\B" "")     => nil
    //   (string-match "\\b" ".")    => 0     (string-match "\\B" ".")    => nil
    //   (string-match "\\b" "a")    => 0     (string-match "\\B" "a")    => nil
    //   (string-match "\\B" "ab")   => 1     (string-match "\\B" ".)_aA")=> 1
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;

    let pos_of = |pat: &str, text: &str| -> Option<usize> {
        search_pattern(pat, text, 0, false, &syn, 0)
            .expect("compile")
            .map(|(pos, _)| pos)
    };

    // `\b` matches at the leading edge for every string, including empty and
    // ones whose first char is not a word constituent.
    assert_eq!(pos_of("\\b", ""), Some(0), "\\b on empty string");
    assert_eq!(pos_of("\\b", "."), Some(0), "\\b before non-word char");
    assert_eq!(pos_of("\\b", "a"), Some(0), "\\b before word char");

    // `\B` never matches at an edge.
    assert_eq!(pos_of("\\B", ""), None, "\\B on empty string");
    assert_eq!(pos_of("\\B", "."), None, "\\B on single non-word char");
    assert_eq!(pos_of("\\B", "a"), None, "\\B on single word char");

    // Interior non-boundaries still match `\B` at the right place.
    assert_eq!(pos_of("\\B", "ab"), Some(1), "\\B between two word chars");
    assert_eq!(
        pos_of("\\B", ".)_aA"),
        Some(1),
        "\\B between two non-word chars"
    );

    // `\W\b`: a non-word char followed by the trailing edge (a boundary).
    assert_eq!(pos_of("\\W\\b", ","), Some(0), "\\W then \\b at EOF edge");
}

#[test]
fn test_explicit_group_number_collision_with_open_group() {
    // Regression: GNU rejects an explicit `\(?N:...\)` whose number collides
    // with a still-OPEN enclosing group (regex-emacs.c:2250-2259,
    // group_in_compile_stack) as `invalid-regexp`; romacs used to accept it.
    // But *reusing* a number for a sequential/closed group stays legal.  Found
    // by the differential proptest.
    crate::test_utils::init_test_tracing();

    // Collision with an enclosing open group -> compile error.
    assert!(
        regex_compile("\\(\\(?1:\\)\\)", false, false).is_err(),
        "explicit group 1 nested inside the auto-numbered group 1 must error"
    );
    assert!(
        regex_compile("\\(\\(\\(?2:\\)\\)\\)", false, false).is_err(),
        "explicit group 2 nested inside enclosing group 2 must error"
    );

    // Reusing a number for a closed/sequential group is still accepted.
    assert!(
        regex_compile("\\(?1:\\)\\(?1:\\)", false, false).is_ok(),
        "two sequential explicit group 1s are legal"
    );
    assert!(
        regex_compile("\\(\\)\\(?1:\\)", false, false).is_ok(),
        "auto group 1 then a sequential explicit group 1 is legal"
    );
    assert!(
        regex_compile("\\(\\(?2:\\)\\)", false, false).is_ok(),
        "explicit group 2 inside open group 1 (no collision) is legal"
    );
}

#[test]
fn test_auto_group_number_after_explicit_uses_re_nsub() {
    // Regression: an auto-numbered `\(...\)` following an explicit
    // `\(?N:...\)` that lowered the running regnum below re_nsub used to reuse
    // an already-assigned number (`regnum + 1`) instead of GNU's `++re_nsub`.
    // For `\(?2:\(?1:x\)\)\(y\)` on "xy" GNU numbers the trailing `\(y\)`
    // as group 3 -> match-data (0 2 0 1 0 1 1 2); romacs used to mislabel it 2.
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    let (pos, regs) = search_pattern("\\(?2:\\(?1:x\\)\\)\\(y\\)", "xy", 0, false, &syn, 0)
        .expect("compile")
        .expect("should match");
    assert_eq!(pos, 0);
    assert_eq!((regs.start[0], regs.end[0]), (0, 2));
    assert_eq!((regs.start[1], regs.end[1]), (0, 1), "explicit group 1 = x");
    assert_eq!((regs.start[2], regs.end[2]), (0, 1), "explicit group 2 = x");
    assert_eq!(
        (regs.start[3], regs.end[3]),
        (1, 2),
        "auto group must be numbered 3 (= ++re_nsub), not colliding with 2"
    );
}

#[test]
fn test_charset() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    let r = search_pattern("[abc]", "xbz", 0, false, &syn, 0);
    assert!(r.unwrap().is_some());
    let r = search_pattern("[abc]", "xyz", 0, false, &syn, 0);
    assert!(r.unwrap().is_none());
}

#[test]
fn test_syntax_word() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // \sw matches word characters
    let r = search_pattern("\\sw+", "hello world", 0, false, &syn, 0);
    assert!(r.unwrap().is_some());
}

#[test]
fn default_syntax_lookup_uses_gnu_standard_classes() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    assert_eq!(syn.char_syntax('a'), crate::SyntaxClass::Word);
    assert_eq!(syn.char_syntax('$'), crate::SyntaxClass::Word);
    assert_eq!(syn.char_syntax('_'), crate::SyntaxClass::Symbol);
    assert_eq!(syn.char_syntax('-'), crate::SyntaxClass::Symbol);
    assert_eq!(syn.char_syntax(' '), crate::SyntaxClass::Whitespace);
    assert_eq!(syn.char_syntax('\u{4e2d}'), crate::SyntaxClass::Word);
}

#[test]
fn test_backreference() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    let r = search_pattern("\\(a\\)\\1", "aa", 0, false, &syn, 0);
    assert!(r.unwrap().is_some());
    let r = search_pattern("\\(a\\)\\1", "ab", 0, false, &syn, 0);
    assert!(r.unwrap().is_none());
}

#[test]
fn backreference_to_open_group_is_invalid_like_gnu() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    let err = search_pattern("\\([^ \t\n]+ \\1\\)", "hello hello", 0, false, &syn, 0)
        .expect_err("GNU signals invalid-regexp for a backreference before group end");
    assert_eq!(err.message, "Invalid back reference");
}

#[test]
fn test_alternation() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    let r = search_pattern("\\(foo\\|bar\\)", "test bar baz", 0, false, &syn, 0);
    assert!(r.is_ok(), "compile failed: {:?}", r.err());
    assert!(r.as_ref().unwrap().is_some(), "match failed");
    let (pos, regs) = r.unwrap().unwrap();
    assert_eq!(pos, 5, "match position");
    assert_eq!(regs.start[0], 5);
    assert_eq!(regs.end[0], 8);
}

#[test]
fn test_char_range() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    let r = search_pattern("[0-9]+", "foo 123 bar", 0, false, &syn, 0);
    assert!(r.is_ok(), "compile failed: {:?}", r.err());
    assert!(r.as_ref().unwrap().is_some(), "match failed");
    let (pos, _regs) = r.unwrap().unwrap();
    assert_eq!(pos, 4, "match position");
}

#[test]
fn test_fastmap_skips_positions() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // Pattern starts with 'z' — should skip to position where 'z' appears
    let r = search_pattern("zing", "aaaaaaaaaazing", 0, false, &syn, 0);
    assert!(r.unwrap().is_some());
    let r = search_pattern("zing", "aaaaaaaaaazing", 0, false, &syn, 0);
    let (pos, _) = r.unwrap().unwrap();
    assert_eq!(pos, 10);
}

#[test]
fn test_fastmap_literal_accurate() {
    crate::test_utils::init_test_tracing();
    // Verify fastmap is populated and accurate for a simple literal
    let compiled = regex_compile("hello", false, false).unwrap();
    assert!(compiled.fastmap_accurate);
    assert!(compiled.fastmap[b'h' as usize]);
    assert!(!compiled.fastmap[b'a' as usize]);
    assert!(!compiled.fastmap[b'z' as usize]);
}

#[test]
fn test_fastmap_charset() {
    crate::test_utils::init_test_tracing();
    // Verify fastmap for character class patterns
    let compiled = regex_compile("[abc]", false, false).unwrap();
    assert!(compiled.fastmap_accurate);
    assert!(compiled.fastmap[b'a' as usize]);
    assert!(compiled.fastmap[b'b' as usize]);
    assert!(compiled.fastmap[b'c' as usize]);
    assert!(!compiled.fastmap[b'd' as usize]);
}

#[test]
fn test_fastmap_case_fold() {
    crate::test_utils::init_test_tracing();
    // Case-folded pattern should match both cases
    let compiled = regex_compile("Hello", false, true).unwrap();
    assert!(compiled.fastmap_accurate);
    assert!(compiled.fastmap[b'h' as usize]);
    assert!(compiled.fastmap[b'H' as usize]);
}

#[test]
fn test_fastmap_alternation() {
    crate::test_utils::init_test_tracing();
    // Alternation: both branches should appear in fastmap
    let compiled = regex_compile("\\(foo\\|bar\\)", false, false).unwrap();
    assert!(compiled.fastmap_accurate);
    assert!(compiled.fastmap[b'f' as usize]);
    assert!(compiled.fastmap[b'b' as usize]);
    assert!(!compiled.fastmap[b'z' as usize]);
}

#[test]
fn test_fastmap_dot() {
    crate::test_utils::init_test_tracing();
    // AnyChar: everything except newline
    let compiled = regex_compile(".", false, false).unwrap();
    assert!(compiled.fastmap_accurate);
    assert!(compiled.fastmap[b'a' as usize]);
    assert!(compiled.fastmap[b'Z' as usize]);
    assert!(!compiled.fastmap[b'\n' as usize]);
}

#[test]
fn test_fastmap_anchor_then_literal() {
    crate::test_utils::init_test_tracing();
    // ^hello — anchor is zero-width, fastmap should see 'h'
    let compiled = regex_compile("^hello", false, false).unwrap();
    assert!(compiled.fastmap_accurate);
    assert!(compiled.fastmap[b'h' as usize]);
    assert!(!compiled.fastmap[b'x' as usize]);
}

#[test]
fn test_fastmap_charset_not() {
    crate::test_utils::init_test_tracing();
    // [^abc] should allow everything except a, b, c
    let compiled = regex_compile("[^abc]", false, false).unwrap();
    assert!(compiled.fastmap_accurate);
    assert!(!compiled.fastmap[b'a' as usize]);
    assert!(!compiled.fastmap[b'b' as usize]);
    assert!(!compiled.fastmap[b'c' as usize]);
    assert!(compiled.fastmap[b'd' as usize]);
    assert!(compiled.fastmap[b'z' as usize]);
}

#[test]
fn test_unterminated_charset_reports_gnu_ebrack() {
    crate::test_utils::init_test_tracing();
    match regex_compile("[invalid", false, false) {
        Ok(_) => panic!("unterminated charset should fail"),
        Err(err) => assert_eq!(err.message, "Unmatched [ or [^"),
    }
}

#[test]
fn test_trailing_backslash_reports_gnu_eescape() {
    crate::test_utils::init_test_tracing();
    match regex_compile("a\\", false, false) {
        Ok(_) => panic!("trailing backslash should fail"),
        Err(err) => assert_eq!(err.message, "Trailing backslash"),
    }
}

#[test]
fn test_unmatched_interval_reports_gnu_ebrace() {
    crate::test_utils::init_test_tracing();
    match regex_compile("a\\{2", false, false) {
        Ok(_) => panic!("unmatched interval should fail"),
        Err(err) => assert_eq!(err.message, "Unmatched \\{"),
    }
}

#[test]
fn test_multibyte_charset() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    let r = search_pattern("[àáâ]", "hello à world", 0, false, &syn, 0);
    assert!(r.is_ok(), "compile failed: {:?}", r.err());
    assert!(r.unwrap().is_some(), "should match à in text");
}

#[test]
fn test_multibyte_charset_no_match() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    let r = search_pattern("[àáâ]", "hello world", 0, false, &syn, 0);
    assert!(r.is_ok());
    assert!(
        r.unwrap().is_none(),
        "should not match when no accented chars"
    );
}

#[test]
fn test_multibyte_charset_range() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // Range of accented Latin characters: é (U+00E9) through ü (U+00FC)
    let r = search_pattern("[é-ü]", "hello ö world", 0, false, &syn, 0);
    assert!(r.is_ok(), "compile failed: {:?}", r.err());
    assert!(r.unwrap().is_some(), "ö should be in range é-ü");
}

#[test]
fn test_multibyte_charset_range_no_match() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // 'a' (U+0061) is outside the range é (U+00E9) through ü (U+00FC)
    let r = search_pattern("[é-ü]", "hello a world", 0, false, &syn, 0);
    assert!(r.is_ok());
    assert!(r.unwrap().is_none(), "ASCII 'a' should not be in range é-ü");
}

#[test]
fn test_multibyte_charset_not() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // [^à] should match any character that is not à
    let r = search_pattern("[^à]", "à", 0, false, &syn, 0);
    assert!(r.is_ok());
    assert!(r.unwrap().is_none(), "[^à] should not match 'à'");

    let r = search_pattern("[^à]", "b", 0, false, &syn, 0);
    assert!(r.is_ok());
    assert!(r.unwrap().is_some(), "[^à] should match 'b'");
}

#[test]
fn test_multibyte_charset_mixed() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // Mix of ASCII and non-ASCII in one charset
    let r = search_pattern("[aéz]", "hello é world", 0, false, &syn, 0);
    assert!(r.is_ok());
    assert!(r.unwrap().is_some(), "should match é");

    let r = search_pattern("[aéz]", "hello z world", 0, false, &syn, 0);
    assert!(r.is_ok());
    assert!(r.unwrap().is_some(), "should also match z");
}

#[test]
fn test_multibyte_charset_cjk() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // CJK characters
    let r = search_pattern("[你好世]", "say 好 to the world", 0, false, &syn, 0);
    assert!(r.is_ok());
    assert!(r.unwrap().is_some(), "should match 好");
}

#[test]
fn test_multibyte_charset_match_position() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    let r = search_pattern("[àáâ]", "hello á world", 0, false, &syn, 0);
    let (pos, regs) = r.unwrap().unwrap();
    assert_eq!(pos, 6, "á starts at byte 6");
    assert_eq!(regs.end[0], 8, "á is 2 bytes in UTF-8, ends at byte 8");
}

// Regression tests for the byte-shift bug: when an alternation/quantifier
// splices an `on_failure_jump` (or similar) AHEAD of an already-emitted
// `Charset`/`CharsetNot` opcode, the opcode's byte position shifts.  The
// multibyte range table is kept in a side map keyed by that byte position;
// before the fix the keys were never updated, so the range table was orphaned
// and non-ASCII chars silently failed to match.  GNU returns 0 for all of the
// patterns below; romacs returned `None` (matching char position 0 here).

#[test]
fn test_charset_before_alternation_shift() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // (string-match "[é]\\|x" (string ?é)) => 0 in GNU.
    // The `\\|x` second alternative splices an OnFailureJump before `[é]`.
    let r = search_pattern("[é]\\|x", "é", 0, false, &syn, 0);
    assert!(r.is_ok(), "compile failed: {:?}", r.err());
    let (pos, _) = r
        .unwrap()
        .expect("[é]\\|x should match the lone é, not be orphaned by the splice");
    assert_eq!(pos, 0);
}

#[test]
fn test_charset_range_before_alternation_shift() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // (string-match "[ç-ï]\\|x" (string ?é)) => 0 in GNU (é is in ç..ï).
    let r = search_pattern("[ç-ï]\\|x", "é", 0, false, &syn, 0);
    assert!(r.is_ok(), "compile failed: {:?}", r.err());
    let (pos, _) = r.unwrap().expect("[ç-ï]\\|x should match é (in range ç-ï)");
    assert_eq!(pos, 0);
}

#[test]
fn test_charset_range_before_quantifier_shift() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // (string-match "[ç-ï]*x" (string ?é ?é ?x)) => 0 in GNU.
    // The `*` splices OnFailureJumpLoop before `[ç-ï]`.
    let r = search_pattern("[ç-ï]*x", "ééx", 0, false, &syn, 0);
    assert!(r.is_ok(), "compile failed: {:?}", r.err());
    let (pos, _) = r
        .unwrap()
        .expect("[ç-ï]*x should match \"ééx\" from the start");
    assert_eq!(pos, 0);
}

#[test]
fn test_charset_in_group_before_alternation_shift() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // (string-match "\\([ç-ï]\\)\\|z" (string ?é)) => 0 in GNU.
    // Group + alternation both splice bytes ahead of `[ç-ï]`.
    let r = search_pattern("\\([ç-ï]\\)\\|z", "é", 0, false, &syn, 0);
    assert!(r.is_ok(), "compile failed: {:?}", r.err());
    let (pos, regs) = r
        .unwrap()
        .expect("\\([ç-ï]\\)\\|z should match é and capture it");
    assert_eq!(pos, 0);
    assert_eq!(regs.start[1], 0, "group 1 should capture the é");
    assert_eq!(regs.end[1], 2, "é is 2 bytes in UTF-8");
}

#[test]
fn test_charset_optional_before_quantifier_shift() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // Greedy `?` also splices an OnFailureJump before the charset.
    // (string-match "[ç-ï]?x" (string ?é ?x)) => 0 in GNU.
    let r = search_pattern("[ç-ï]?x", "éx", 0, false, &syn, 0);
    assert!(r.is_ok(), "compile failed: {:?}", r.err());
    let (pos, _) = r.unwrap().expect("[ç-ï]?x should match \"éx\"");
    assert_eq!(pos, 0);

    // Non-greedy `*?` truncates+re-extends the charset body to a new offset.
    // (string-match "[ç-ï]*?x" (string ?é ?é ?x)) => 0 in GNU.
    let r = search_pattern("[ç-ï]*?x", "ééx", 0, false, &syn, 0);
    assert!(r.is_ok(), "compile failed: {:?}", r.err());
    let (pos, _) = r.unwrap().expect("[ç-ï]*?x should match \"ééx\"");
    assert_eq!(pos, 0);
}

#[test]
fn test_ascii_class_before_alternation_no_regression() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // ASCII-only classes use the bitmap (no side-map entry), so the splice
    // never affected them; assert they still behave correctly.
    let r = search_pattern("[a-c]\\|x", "b", 0, false, &syn, 0);
    assert!(r.is_ok());
    assert_eq!(r.unwrap().expect("[a-c]\\|x should match b").0, 0);

    let r = search_pattern("[a-c]\\|x", "x", 0, false, &syn, 0);
    assert!(r.is_ok());
    assert_eq!(
        r.unwrap().expect("[a-c]\\|x should match x via 2nd alt").0,
        0
    );

    let r = search_pattern("[a-c]\\|x", "z", 0, false, &syn, 0);
    assert!(r.is_ok());
    assert!(r.unwrap().is_none(), "[a-c]\\|x should not match z");

    // Quantifier over an ASCII class (`*`) plus a multibyte class to confirm
    // the two co-exist after a shift.
    let r = search_pattern("[a-c]*[ç-ï]", "abcé", 0, false, &syn, 0);
    assert!(r.is_ok(), "compile failed: {:?}", r.err());
    assert_eq!(r.unwrap().expect("[a-c]*[ç-ï] should match \"abcé\"").0, 0);
}

// ---------------------------------------------------------------------------
// GNU parity: descending intervals \{n,m\} with n>m must be rejected.
// (string-match "a\\{2,1\\}" "aa") -> (invalid-regexp "Invalid content of \\{\\}")
// ---------------------------------------------------------------------------

#[test]
fn test_descending_interval_reports_gnu_badbr() {
    crate::test_utils::init_test_tracing();
    // \{2,1\}, \{5,2\}, \{3,0\}: lower > upper must signal "Invalid content of \{\}".
    for pat in ["a\\{2,1\\}", "a\\{5,2\\}", "a\\{3,0\\}"] {
        match regex_compile(pat, false, false) {
            Ok(_) => panic!("descending interval {pat:?} should fail to compile"),
            Err(err) => assert_eq!(
                err.message, "Invalid content of \\{\\}",
                "wrong error for {pat:?}"
            ),
        }
    }
}

#[test]
fn test_ascending_and_unbounded_intervals_still_compile() {
    crate::test_utils::init_test_tracing();
    // Equal bounds, ascending bounds, and an unbounded upper must remain valid.
    for pat in ["a\\{2,3\\}", "a\\{2,2\\}", "a\\{2,\\}", "a\\{0,2\\}"] {
        assert!(
            regex_compile(pat, false, false).is_ok(),
            "valid interval {pat:?} should compile"
        );
    }
}

// ---------------------------------------------------------------------------
// GNU parity: a redundant trailing quantifier folds onto the preceding one.
// (string-match "a**" "aaa") -> 0  (GNU; neo previously returned nil)
// Also a*?*, a*+, a++, a???.
// ---------------------------------------------------------------------------

#[test]
fn test_stacked_quantifiers_fold_like_gnu() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // Each of these must compile and match at position 0 of "aaa",
    // exactly like GNU's quantifier folding.
    for pat in ["a**", "a*?*", "a*+", "a++", "a???", "a+*", "a?*", "a?+"] {
        let r = search_pattern(pat, "aaa", 0, false, &syn, 0);
        let r = r.unwrap_or_else(|e| panic!("{pat:?} failed to compile: {e:?}"));
        let (pos, _regs) = r.unwrap_or_else(|| panic!("{pat:?} should match \"aaa\""));
        assert_eq!(pos, 0, "{pat:?} should match at position 0");
    }
}

#[test]
fn test_stacked_greedy_star_consumes_all() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // `a**` folds to a greedy `a*`, so on "aaa" it consumes all three a's.
    let (pos, regs) = search_pattern("a**", "aaa", 0, false, &syn, 0)
        .unwrap()
        .expect("a** should match \"aaa\"");
    assert_eq!(pos, 0);
    assert_eq!(regs.end[0], 3, "greedy a** should consume all three a's");
}

#[test]
fn test_stacked_plus_requires_one() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // `a++` folds to a greedy `a+`: must match at least one `a`.
    let r = search_pattern("a++", "aaa", 0, false, &syn, 0).unwrap();
    let (pos, regs) = r.expect("a++ should match \"aaa\"");
    assert_eq!(pos, 0);
    assert_eq!(regs.end[0], 3);
    // `a+` (folded from a++) must NOT match a string with no `a`.
    let r = search_pattern("a++", "bbb", 0, false, &syn, 0).unwrap();
    assert!(r.is_none(), "a++ must not match a string with no a's");
}

#[test]
fn cntrl_class_excludes_del_like_gnu() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    // GNU `ISCNTRL(c)` is `((c) < ' ')` (regex-emacs.c:108), so `[[:cntrl:]]`
    // matches only 0x00..=0x1F.  In particular it must NOT match DEL (0x7F),
    // unlike the C-locale `iscntrl`.  This is the primitive behind json.el's
    // `(rx (in cntrl))`, which controls whether `json-encode-string` escapes a
    // character: DEL must pass through literally, only chars < 0x20 are escaped.

    // 0x1F (unit separator) is a control char and must match.
    assert!(
        search_pattern("[[:cntrl:]]", "\u{1f}", 0, false, &syn, 0)
            .unwrap()
            .is_some(),
        "[[:cntrl:]] must match 0x1F"
    );
    // 0x7F (DEL) is NOT a control char for Emacs regexp.
    assert!(
        search_pattern("[[:cntrl:]]", "\u{7f}", 0, false, &syn, 0)
            .unwrap()
            .is_none(),
        "[[:cntrl:]] must NOT match DEL (0x7F)"
    );
    // The same holds when combined with other class members, mirroring the
    // exact charset json.el compiles: `(rx (in ?\" ?\\ cntrl))`.
    assert!(
        search_pattern("[\"\\\\[:cntrl:]]", "\u{7f}", 0, false, &syn, 0)
            .unwrap()
            .is_none(),
        "[\"\\\\[:cntrl:]] must NOT match DEL (0x7F)"
    );
    // A boundary check: 0x20 (space) is not a control char.
    assert!(
        search_pattern("[[:cntrl:]]", " ", 0, false, &syn, 0)
            .unwrap()
            .is_none(),
        "[[:cntrl:]] must NOT match space"
    );
}

/// Under the GNU *standard* syntax table (`init_syntax_once`, syntax.c:3686-3691,
/// the table used by `fundamental-mode` / `(standard-syntax-table)`), LF and CR
/// ARE whitespace, so `[[:space:]]` DOES match them.  This is the other half of
/// the syntax-table dependency and guards against over-correcting the fix.
#[test]
fn posix_space_class_matches_newline_cr_under_standard_syntax() {
    crate::test_utils::init_test_tracing();
    let syn = DefaultSyntaxLookup;
    for (text, label) in [
        ("\n", "LF"),
        ("\r", "CR"),
        (" ", "space"),
        ("\t", "tab"),
        ("\u{0c}", "formfeed"),
    ] {
        assert!(
            search_pattern("[[:space:]]", text, 0, false, &syn, 0)
                .unwrap()
                .is_some(),
            "[[:space:]] must match {label} under the standard syntax table"
        );
    }
}

// ===========================================================================
// Pike VM (non-backtracking fast path): it must be byte-exact with the
// backtracker for every eligible pattern.
// ===========================================================================

#[test]
fn differential_overrides_restore_nested_and_unwound_state() {
    assert!(!force_backtrack());
    assert!(!force_pike());
    assert!(!fastmap_force_disabled());

    with_regex_engine_override(RegexEngineOverride::Backtracker, || {
        assert!(force_backtrack());
        assert!(!force_pike());

        let panic = std::panic::catch_unwind(|| {
            with_regex_engine_override(RegexEngineOverride::PikeVm, || {
                assert!(!force_backtrack());
                assert!(force_pike());
                panic!("exercise override unwinding");
            });
        });
        assert!(panic.is_err());
        assert!(force_backtrack());
        assert!(!force_pike());
    });

    let panic = std::panic::catch_unwind(|| {
        with_fastmap_disabled(|| {
            assert!(fastmap_force_disabled());
            panic!("exercise optimization-override unwinding");
        });
    });
    assert!(panic.is_err());
    assert!(!force_backtrack());
    assert!(!force_pike());
    assert!(!fastmap_force_disabled());
}

#[test]
fn backward_syntax_assertions_share_gnu_stop_semantics_across_engines() {
    crate::test_utils::init_test_tracing();
    let syntax = DefaultSyntaxLookup;
    let text = b"one two three";

    for engine in [
        RegexEngineOverride::Backtracker,
        RegexEngineOverride::PikeVm,
    ] {
        for (pattern, expected_start) in [(" \\<", 3), (" \\_<", 3), (" \\b", 7)] {
            let compiled = regex_compile(pattern, false, false).expect("compile assertion pattern");
            assert!(
                compiled.pike_eligible,
                "{pattern:?} must exercise both engines"
            );
            let (start, registers) = with_regex_engine_override(engine, || {
                re_search(&compiled, text, 8, -8, &syntax, 8).expect("backward match")
            });
            assert_eq!(start, expected_start, "{engine:?} {pattern:?}");
            assert_eq!(registers.start[0], expected_start as i64);
        }
    }
}

// ---- targeted unit tests: Pike vs backtracker byte-exactness -------------

/// `a*a*b` — the classic catastrophic-backtracking pattern — must be handled
/// by the Pike VM in LINEAR time (was cubic in the backtracker).  This test
/// would blow up / time out if it ever fell back to the backtracker.
#[test]
fn pike_no_catastrophic_backtracking() {
    let cp = regex_compile("a*a*b", false, false).expect("compile");
    assert!(cp.pike_eligible, "a*a*b must be pike-eligible");

    // Linear check: a growing run of 'a' with no trailing 'b' fails FAST.
    // The backtracker is O(n^3) here; the Pike VM is O(n*m).  A generous
    // wall-clock ceiling that only the linear engine can meet.
    let syn = DefaultSyntaxLookup;
    let n = 20_000usize;
    let mut text = vec![b'a'; n];
    text.push(b'!');
    let t0 = std::time::Instant::now();
    assert!(re_match(&cp, &text, 0, text.len(), &syn, 0).is_none());
    let elapsed = t0.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "a*a*b over {n} a's took {elapsed:?} — not linear (Pike path not taken?)"
    );

    // And it still finds the real match when a 'b' is present.
    let mut text2 = vec![b'a'; 500];
    text2.push(b'b');
    let r = re_match(&cp, &text2, 0, text2.len(), &syn, 0);
    assert_eq!(r.map(|(e, _)| e), Some(501));
}

/// The dual: a capture INSIDE a nullable loop is Pike-INELIGIBLE (GNU's
/// empty-loop capture semantics), so it must fall back to the backtracker.
#[test]
fn pike_capture_in_nullable_loop_is_ineligible() {
    for p in [
        "\\(a*\\)*",
        "\\(a\\|\\)*",
        "\\(?:\\(x*\\)\\)*",
        "\\(.??\\)+",
    ] {
        let cp = regex_compile(p, false, false).expect("compile");
        assert!(
            !cp.pike_eligible,
            "{p:?} has a capture in a nullable loop; must be Pike-ineligible"
        );
    }
}

/// The rewind view exists for every pattern, not only the Pike-eligible ones:
/// the existence DFA reads it for POSIX patterns and `??`, and it costs
/// nothing to have it for the rest (a backreference here).
#[test]
fn rewind_view_is_built_for_every_pattern() {
    for (pattern, posix, keep_string) in [
        // `[a-z]*` before `:` is exclusive: a keep-string loop to rewrite.
        ("[a-z]*:", false, true),
        ("[a-z]*:", true, true),
        ("\\(x\\)\\1[a-z]*:", false, true),
        ("x??[a-z]*:", false, true),
        ("a*a", false, false),
    ] {
        let compiled = regex_compile(pattern, posix, false).expect("compile");
        let has_keep_string = compiled
            .buffer
            .contains(&(RegexOp::OnFailureKeepStringJump as u8));
        assert_eq!(has_keep_string, keep_string, "{pattern:?}");
        let view = compiled.rewind_bytecode().expect("a rewind view");
        assert_eq!(view.len(), compiled.buffer.len(), "{pattern:?}");
        assert!(
            !view.contains(&(RegexOp::OnFailureKeepStringJump as u8)),
            "{pattern:?}: the view has no keep-string jump left"
        );
        assert_eq!(
            matches!(compiled.rewind, RewindView::Rewritten(_)),
            keep_string,
            "{pattern:?}"
        );
    }
}

/// The production routing is backtracker-by-default with a Pike fallback that
/// triggers ONLY on catastrophic backtracking: a well-behaved pattern must
/// never fall back (so it keeps the backtracker's speed), while `a*a*b` on a
/// long non-matching run must fall back to the linear Pike VM.
#[test]
fn pike_fallback_only_on_catastrophe() {
    let syn = DefaultSyntaxLookup;

    // Well-behaved patterns over ordinary text: no fallback.
    for (pat, text) in [
        ("\\(foo\\|bar\\)+baz", &b"foobarbaz foo bar"[..]),
        ("[a-z]+\\_>", b"hello world twelve"),
        ("a.*b", b"axxxxxxxxxxb and more"),
    ] {
        let cp = regex_compile(pat, false, false).unwrap();
        let before = pike_fallback_count();
        let _ = re_search(&cp, text, 0, text.len() as isize, &syn, 0);
        assert_eq!(
            pike_fallback_count(),
            before,
            "{pat:?} should not trip the catastrophe fallback"
        );
    }

    // `a*a*b` over a long non-matching run: MUST fall back to Pike.
    let cp = regex_compile("a*a*b", false, false).unwrap();
    let mut text = vec![b'a'; 4000];
    text.push(b'!');
    let before = pike_fallback_count();
    assert!(re_match(&cp, &text, 0, text.len(), &syn, 0).is_none());
    assert!(
        pike_fallback_count() > before,
        "a*a*b over a long run should trip the catastrophe fallback"
    );
}

// ===========================================================================
// Multi-literal SIMD prefilter — differential fuzzer
//
// The prefilter AUGMENTS the backtracker: it skips to candidate positions,
// which the backtracker still verifies.  So the ONLY correctness requirement
// is that the extracted literals are SOUND — every match provably contains one
// at a known offset.  An unsound skip drops a real match, which this fuzzer
// catches by comparing, over random elisp-subset patterns × buffers:
//   * ORACLE   = `re_search` with the fastmap/prefilter skip DISABLED
//                (every position scanned by the pure matcher), and
//   * CANDIDATE = `re_search` with the prefilter ENABLED.
// Same match engine on both sides (the production heuristic); only the skip
// mechanism differs, so any divergence pinpoints the prefilter.
// ===========================================================================

// ---- prefilter: targeted extraction-shape unit tests ---------------------

#[test]
fn prefilter_none_for_syntax_class_head() {
    // sexp-head-kw: after `(`, the group starts with `\w`/`\s_` — no required
    // literal beyond the common `(`, so the single-byte set is rejected (the
    // fastmap's memchr already covers it).
    let cp =
        regex_compile("(\\(\\(?:\\w\\|\\s_\\|\\\\.\\)+\\)\\_>", false, false).expect("compile");
    assert!(
        cp.literal_prefilter().is_none(),
        "single-byte `(` prefix must not build a prefilter (fastmap suffices)"
    );
}

#[test]
fn prefilter_none_for_leading_nonliteral() {
    for pat in ["\\w+foo", ".*bar", "[a-z]+baz", "\\(?:\\w\\|x\\)y"] {
        let cp = regex_compile(pat, false, false).expect("compile");
        assert!(
            cp.literal_prefilter().is_none(),
            "leading non-literal must have no prefilter: {pat:?}"
        );
    }
}

#[test]
fn test_pattern_max_match_chars() {
    crate::test_utils::init_test_tracing();
    let max = |pat: &str| pattern_max_match_chars(&regex_compile(pat, false, false).unwrap());
    // Loop-free patterns get a finite conservative bound at least as
    // large as any real match.
    assert_eq!(max("hello"), Some(5));
    assert_eq!(max("\\_<byte-compile\\_>"), Some(12));
    assert_eq!(max("^foo$"), Some(3));
    assert!(max("a\\|bb").unwrap() >= 2);
    assert!(max("\\(ab\\)c").unwrap() >= 3);
    assert!(max("[abc]x").unwrap() >= 2);
    assert!(max("\\sw\\s-").unwrap() >= 2);
    // Unbounded or counter-driven repetition, and backreferences,
    // report no finite bound.
    assert_eq!(max("a*"), None);
    assert_eq!(max("a+b"), None);
    assert_eq!(max("a\\{2,5\\}"), None);
    assert_eq!(max("\\(x\\)\\1"), None);
    assert_eq!(max(".*foo"), None);
}

/// The literal prefilter is built by the first forward search long enough to
/// use it, not at compile time: most patterns only ever match short strings.
/// A short search leaves it unbuilt and still finds the match.
#[test]
fn prefilter_is_built_by_the_first_long_forward_search() {
    let cp = regex_compile("unread-command-events", false, false).expect("compile");
    assert!(
        cp.prefilter.get().is_none(),
        "compiling builds no prefilter"
    );
    let short = b"xx unread-command-events";
    let found = re_search(&cp, short, 0, short.len() as isize, &DefaultSyntaxLookup, 0);
    assert_eq!(found.map(|(pos, _)| pos), Some(3));
    assert!(
        cp.prefilter.get().is_none(),
        "a short search builds no prefilter"
    );
    let mut long = vec![b'x'; 1000];
    long.extend_from_slice(b"unread-command-events");
    let found = re_search(&cp, &long, 0, long.len() as isize, &DefaultSyntaxLookup, 0);
    assert_eq!(found.map(|(pos, _)| pos), Some(1000));
    assert!(
        cp.prefilter.get().is_some_and(Option::is_some),
        "a long search builds and uses the prefilter"
    );
}

#[test]
fn bounded_bol_search_agrees_with_exhaustive_candidates() {
    crate::test_utils::init_test_tracing();
    let syntax = DefaultSyntaxLookup;
    for text in ["", "aa\nz", "a\n\nz\n", "é中\n日\nz\nZ"] {
        let positions: Vec<_> = text
            .char_indices()
            .map(|(pos, _)| pos)
            .chain(std::iter::once(text.len()))
            .collect();
        for pattern in ["^", "^z", "^\\(\\)", "^\\(z\\)", "^z?", "^$", "^日"] {
            for case_fold in [false, true] {
                let compiled = regex_compile(pattern, false, case_fold).expect("compile");
                for &start in &positions {
                    for &end in positions.iter().filter(|&&end| end >= start) {
                        // Use the matcher directly so the oracle does not share
                        // the searcher's BOL candidate skipping or scan bounds.
                        let expected = positions
                            .iter()
                            .copied()
                            .filter(|&pos| pos >= start && pos <= end)
                            .find_map(|pos| {
                                re_match(&compiled, text.as_bytes(), pos, end, &syntax, start)
                                    .map(|(_, regs)| (pos, regs.start, regs.end))
                            });
                        let actual = re_search(
                            &compiled,
                            text.as_bytes(),
                            start,
                            (end - start) as isize,
                            &syntax,
                            start,
                        )
                        .map(|(pos, regs)| (pos, regs.start, regs.end));
                        assert_eq!(
                            actual, expected,
                            "{pattern:?} in {text:?}, {start}..={end}, fold={case_fold}"
                        );
                    }
                }
            }
        }
    }
}
