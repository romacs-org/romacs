//! The linear-time matcher (R92 step 2, `linear.rs`): its answers against
//! the backtracker's, GNU's empty-loop rule, and the work bounds that are
//! the point of it.
//!
//! GNU-pinned values below are GNU 32.0.50 (b28750b822dc)
//! `(and (string-match RE S) (match-data t))` under the standard syntax
//! table; `tools/regex_oracle.py` records them live.

use super::*;

fn compile(pattern: &str) -> CompiledPattern {
    regex_compile(pattern, false, false).expect("pattern compiles")
}

/// Registers as GNU's `(match-data t)` lists them for a match in TEXT, in
/// character positions, trailing unset groups dropped.
fn match_data(text: &[u8], registers: &MatchRegisters) -> Vec<i64> {
    let chars = |byte: i64| {
        if byte < 0 {
            byte
        } else {
            text[..byte as usize]
                .iter()
                .filter(|&&b| (b & 0xC0) != 0x80)
                .count() as i64
        }
    };
    let mut data: Vec<i64> = registers
        .start
        .iter()
        .zip(&registers.end)
        .flat_map(|(&start, &end)| [chars(start), chars(end)])
        .collect();
    while data.len() > 2 && data[data.len() - 1] < 0 && data[data.len() - 2] < 0 {
        data.truncate(data.len() - 2);
    }
    data
}

fn search_from(
    compiled: &CompiledPattern,
    text: &[u8],
    start: usize,
    point: usize,
) -> (Option<(usize, Vec<i64>)>, bool) {
    let _ = take_matcher_overflow();
    let found = re_search(
        compiled,
        text,
        start,
        (text.len() - start) as isize,
        &DefaultSyntaxLookup,
        point,
    );
    let overflow = take_matcher_overflow();
    (
        found.map(|(at, regs)| (at, match_data(text, &regs))),
        overflow,
    )
}

// ---- GNU's empty-loop rule ------------------------------------------------

/// A capture inside a loop whose body can match empty: GNU runs ONE empty
/// iteration and keeps its registers (`CHECK_INFINITE_LOOP` jumps to the
/// exit only on the second empty pass).  Every engine must give GNU's data.
const EMPTY_LOOP_CASES: &[(&str, &str, Option<&[i64]>)] = &[
    ("\\(a*\\)*b", "aab", Some(&[0, 3, 2, 2])),
    ("\\(a*\\)*", "aa", Some(&[0, 2, 2, 2])),
    ("\\(a*\\)*", "", Some(&[0, 0, 0, 0])),
    ("\\(a*\\)*", "b", Some(&[0, 0, 0, 0])),
    ("\\(a*\\)+b", "aab", Some(&[0, 3, 2, 2])),
    ("\\(a\\|\\)*b", "aab", Some(&[0, 3, 2, 2])),
    ("\\(a*\\)*\\(b\\)", "aab", Some(&[0, 3, 2, 2, 2, 3])),
    ("\\(?:\\(a\\)\\|b*\\)*c", "abc", Some(&[0, 3, 0, 1])),
    ("\\(a*?\\)*b", "aab", Some(&[0, 3, 2, 2])),
    ("\\(a*\\)*b", "aaaa", None),
];

#[test]
fn empty_loop_captures_follow_gnu_in_every_engine() {
    for &(pattern, text, expected) in EMPTY_LOOP_CASES {
        let compiled = compile(pattern);
        assert!(
            compiled.linear.eligible(),
            "{pattern:?}: the linear matcher must run a capture in a nullable loop"
        );
        let expected = expected.map(<[i64]>::to_vec);
        for engine in [
            RegexEngineOverride::Production,
            RegexEngineOverride::Backtracker,
            RegexEngineOverride::PikeVm,
            RegexEngineOverride::LinearSearch,
        ] {
            let (found, overflow) = with_regex_engine_override(engine, || {
                search_from(&compiled, text.as_bytes(), 0, 0)
            });
            assert!(!overflow);
            assert_eq!(
                found.map(|(_, data)| data),
                expected,
                "{engine:?} {pattern:?} on {text:?}"
            );
        }
    }
}

/// The empty iteration changes which thread ranks first, not only the
/// registers: after `\(?:\|a\)*` reads nothing, GNU's backtracker tries the
/// loop exit (here `\(?:ab\)?`, which reads `ab`) BEFORE the loop's `a` arm.
/// A Pike VM that prunes the empty pass by pc ranks the `a` arm first and
/// stops at the empty match after it.  GNU: (0 2).
#[test]
fn empty_iteration_ranks_the_loop_exit_like_gnu() {
    for (pattern, text, expected) in [
        ("\\(?:\\|a\\)*\\(?:ab\\)?", "ab", vec![0, 2]),
        ("\\(?:\\|a\\)*\\(ab\\)?", "ab", vec![0, 2, 0, 2]),
    ] {
        let compiled = compile(pattern);
        for engine in [
            RegexEngineOverride::Backtracker,
            RegexEngineOverride::PikeVm,
            RegexEngineOverride::LinearSearch,
        ] {
            let (found, _) = with_regex_engine_override(engine, || {
                search_from(&compiled, text.as_bytes(), 0, 0)
            });
            assert_eq!(
                found.map(|(_, data)| data),
                Some(expected.clone()),
                "{engine:?} {pattern:?} on {text:?}"
            );
        }
    }
}

// ---- Differential against the backtracker ----------------------------------

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const ATOMS: &[&str] = &[
    "a",
    "b",
    "ab",
    ".",
    "[ab]",
    "[^a]",
    "\\w",
    "\\W",
    "\\s-",
    "\\S-",
    "\\sw",
    "é",
    "\\b",
    "\\B",
    "\\<",
    "\\>",
    "\\_<",
    "\\_>",
    "^",
    "$",
    "\\`",
    "\\'",
    "\\=",
    "\\cl",
    "[[:alpha:]]",
    "_",
];
const QUANTIFIERS: &[&str] = &["", "", "", "*", "+", "?", "*?", "+?"];

fn gen_pattern(rng: &mut Rng, depth: usize) -> String {
    let mut out = String::new();
    let terms = 1 + rng.below(3);
    for _ in 0..terms {
        if depth > 0 && rng.below(3) == 0 {
            out.push_str(rng.pick(&["\\(", "\\(?:"]));
            let arms = 1 + rng.below(3);
            for arm in 0..arms {
                if arm > 0 {
                    out.push_str("\\|");
                }
                // An empty arm makes the group nullable.
                if rng.below(5) != 0 {
                    out.push_str(&gen_pattern(rng, depth - 1));
                }
            }
            out.push_str("\\)");
        } else {
            out.push_str(rng.pick(ATOMS));
        }
        out.push_str(rng.pick(QUANTIFIERS));
    }
    out
}

fn gen_text(rng: &mut Rng, max_len: usize) -> Vec<u8> {
    let mut text = String::new();
    for _ in 0..rng.below(max_len + 1) {
        text.push_str(rng.pick(&["a", "a", "b", " ", "\n", "é", "_", "-"]));
    }
    text.into_bytes()
}

fn char_boundaries(text: &[u8]) -> Vec<usize> {
    (0..=text.len())
        .filter(|&i| i == text.len() || (text[i] & 0xC0) != 0x80)
        .collect()
}

/// Every forward search the linear matcher runs in one pass gives the
/// registers the per-candidate backtracker gives, over random patterns with
/// nested, nullable and non-greedy loops, alternations with empty arms,
/// every zero-width assertion, syntax and category classes, multibyte text
/// and case folding.
#[test]
fn linear_search_agrees_with_the_backtracker() {
    let mut rng = Rng(0x5105_2026_1018);
    let (mut eligible, mut path_sensitive, mut compared) = (0usize, 0usize, 0usize);
    for case in 0..6_000 {
        let pattern = gen_pattern(&mut rng, 2);
        let fold = rng.below(4) == 0;
        let Ok(compiled) = regex_compile(&pattern, false, fold) else {
            continue;
        };
        if !compiled.linear.eligible() {
            continue;
        }
        eligible += 1;
        if matches!(compiled.linear, linear::Linear::PathSensitive(_)) {
            path_sensitive += 1;
        }
        let text = gen_text(&mut rng, 10);
        let bounds = char_boundaries(&text);
        let start = bounds[rng.below(bounds.len())];
        let point = bounds[rng.below(bounds.len())];
        let expected = with_backtracker_forced(|| search_from(&compiled, &text, start, point));
        let actual = with_regex_engine_override(RegexEngineOverride::LinearSearch, || {
            search_from(&compiled, &text, start, point)
        });
        assert_eq!(
            actual,
            expected,
            "case {case}: {pattern:?} (fold {fold}) on {:?} from {start}, point {point}",
            String::from_utf8_lossy(&text)
        );
        compared += 1;
    }
    assert!(compared > 2_000, "too few comparisons: {compared}");
    assert!(
        path_sensitive * 10 > eligible,
        "the generator must exercise nullable loops: {path_sensitive}/{eligible}"
    );
}

/// The anchored form (the budgeted backtracker's fallback, `re_match`)
/// against the backtracker at every character boundary.
#[test]
fn linear_anchored_match_agrees_with_the_backtracker() {
    let mut rng = Rng(0xA2C4_0D00_5105);
    let mut compared = 0usize;
    for case in 0..4_000 {
        let pattern = gen_pattern(&mut rng, 2);
        let Ok(compiled) = regex_compile(&pattern, false, rng.below(4) == 0) else {
            continue;
        };
        if !compiled.linear.eligible() {
            continue;
        }
        let text = gen_text(&mut rng, 8);
        for pos in char_boundaries(&text) {
            let run = || {
                re_match(&compiled, &text, pos, text.len(), &DefaultSyntaxLookup, pos)
                    .map(|(end, regs)| (end, match_data(&text, &regs)))
            };
            let expected = with_backtracker_forced(run);
            let actual = with_pike_forced(run);
            assert_eq!(
                actual,
                expected,
                "case {case}: {pattern:?} on {:?} at {pos}",
                String::from_utf8_lossy(&text)
            );
            compared += 1;
        }
    }
    assert!(compared > 5_000, "too few comparisons: {compared}");
}

/// `re_search` hands the linear matcher the position after a failed
/// candidate, which inside multibyte text can be the middle of a character:
/// no candidate may start there (GNU tries character starts only).
#[test]
fn a_search_from_inside_a_character_starts_at_the_next_one() {
    let compiled = compile(".");
    let text = "éa".as_bytes();
    let found = linear::run_search(
        &compiled,
        text,
        1,
        text.len(),
        text.len(),
        &DefaultSyntaxLookup,
        0,
        None,
        usize::MAX,
    );
    let linear::SearchOutcome::Found(start, regs) = found else {
        panic!("`.` matches the `a`");
    };
    assert_eq!(
        (start, regs.end[0]),
        (2, 3),
        "the match starts at a character"
    );
}

// ---- The work bounds ---------------------------------------------------------

/// Monnier's `a*a*a*`: a scan that fails at every candidate.  Each anchored
/// attempt reads to the end of the run, so trying candidates one by one
/// costs `n^2 / 2` DFA bytes; the search must instead stop paying for
/// anchored attempts once they cost more than the text justifies, and still
/// find the match after the run.
#[test]
fn a_failing_scan_does_linear_anchored_work() {
    dfa::reset_dfa_stats();
    let n = 20_000usize;
    let mut text = "a".repeat(n);
    text.push_str("!aab");
    let compiled = compile("a*a*a*b");
    let entries = matcher_entry_count();
    let (found, overflow) = search_from(&compiled, text.as_bytes(), 0, 0);
    let entries = matcher_entry_count() - entries;
    let bytes = dfa::dfa_stats().bytes;
    assert!(!overflow);
    assert_eq!(found, Some((n + 1, vec![n as i64 + 1, n as i64 + 4])));
    assert!(
        bytes + entries < 16 * n as u64,
        "anchored work {bytes} DFA bytes + {entries} matcher entries over {n} bytes is not linear"
    );
}

/// Monnier's `(a*)*`: on a failing candidate the backtracker tries every way
/// to split the run between iterations (exponential).  The budgeted
/// backtracker must give such a candidate to the linear matcher.
#[test]
fn a_capture_in_a_nullable_loop_falls_back_to_the_linear_matcher() {
    let compiled = compile("\\(a*\\)*b");
    let text = "a".repeat(14);
    let fallbacks = pike_fallback_count();
    let found = re_match(
        &compiled,
        text.as_bytes(),
        0,
        text.len(),
        &DefaultSyntaxLookup,
        0,
    );
    assert!(found.is_none());
    assert!(
        pike_fallback_count() > fallbacks,
        "the catastrophic candidate must leave the backtracker"
    );
}

/// A linear search hands a candidate whose paths could overflow GNU's fail
/// stack back to the backtracker: GNU's "Stack overflow in regexp matcher"
/// survives the one-pass search (the D7 case of `fail_stack_parity_test`).
#[test]
fn a_linear_search_keeps_gnu_fail_stack_overflow() {
    let compiled = compile("\\(?:a\\|b\\)*c");
    let text = "ab".repeat(150_000);
    let (found, overflow) = with_regex_engine_override(RegexEngineOverride::LinearSearch, || {
        search_from(&compiled, text.as_bytes(), 0, 0)
    });
    assert_eq!(found, None);
    assert!(overflow, "GNU signals the fail-stack overflow");
    // Under the bound, the one-pass answer stands.
    let short = format!("{}c", "ab".repeat(3_000));
    let (found, overflow) = with_regex_engine_override(RegexEngineOverride::LinearSearch, || {
        search_from(&compiled, short.as_bytes(), 0, 0)
    });
    assert_eq!(found, Some((0, vec![0, short.len() as i64])));
    assert!(!overflow);
}

/// Past the fail-stack bound GNU's backtracker either overflows on its first
/// deep descent or runs for minutes (`a*a*a*b` over 100K `a`s pushes too
/// little to overflow and backtracks cubically).  The parity run is
/// abandoned after `PARITY_POP_BUDGET` pops and the linear answer stands: the
/// search ends, with no overflow.
#[test]
fn a_parity_run_that_would_freeze_gnu_is_abandoned() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let compiled = compile("a*a*a*b");
        let text = "a".repeat(100_000);
        let _ = tx.send(search_from(&compiled, text.as_bytes(), 0, 0));
    });
    let outcome = rx.recv_timeout(std::time::Duration::from_secs(60));
    assert_eq!(
        outcome.ok(),
        Some((None, false)),
        "the search must end, unmatched and without overflow"
    );
    // Where GNU's first descent overflows (400K `a`s), it still does.
    let compiled = compile("a*a*a*b");
    let (found, overflow) = search_from(&compiled, "a".repeat(400_000).as_bytes(), 0, 0);
    assert_eq!(found, None);
    assert!(overflow, "GNU signals the fail-stack overflow");
}
