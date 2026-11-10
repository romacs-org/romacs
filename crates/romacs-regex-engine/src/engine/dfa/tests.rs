//! Tests of the existence DFA (P3.3): the NFA over the rewind view (C2).

use crate::engine::compile::regex_compile;
use crate::engine::dfa::class::ClassContext;
use crate::engine::dfa::filter::{
    COLD_THRESHOLD, DfaLease, DfaMode, DfaSlot, dfa_stats, prime, reset_dfa_stats, with_cold_path,
    with_dfa_mode, with_first_step,
};
use crate::engine::dfa::lazy::{DfaGaveUp, ExistenceDfa, Exists, MATCH, MEMORY_CAP, SLOW, UNKNOWN};
use crate::engine::dfa::nfa::{ClosureScratch, DfaIneligible, Nfa, Place, Predicate, kernel_item};
use crate::engine::matcher::fail::{fail_stack_overflow_free_span, fail_stack_push_sites};
use crate::engine::matcher::route::matcher_entry_count;
use crate::engine::matcher::step::re_text_char;
use crate::engine::matcher::{re_match, re_match_internal};
use crate::engine::opcode::RegexOp;
use crate::engine::pattern::{CompiledPattern, MatchRegisters};
use crate::engine::search::re_search;
use crate::engine::syntax::{DefaultSyntaxLookup, SyntaxCacheKey, SyntaxLookup};

use crate::SyntaxClass;
use crate::emacs_char;

fn nfa_of(pattern: &str) -> Nfa {
    let compiled = regex_compile(pattern, false, false).expect("pattern compiles");
    Nfa::build(&compiled).unwrap_or_else(|why| panic!("{pattern:?} is ineligible: {why:?}"))
}

fn place<'a>(text: &'a [u8], d: usize) -> Place<'a> {
    Place {
        text,
        d,
        stop: text.len(),
        point: 0,
        target_multibyte: true,
        syntax: &DefaultSyntaxLookup,
    }
}

/// The predicates of the positions the start closure reaches at `d`.
fn start_closure(nfa: &Nfa, text: &[u8], d: usize) -> (Vec<Predicate>, bool) {
    let mut scratch = ClosureScratch::default();
    let mut out = Vec::new();
    let accept = nfa.closure(
        &[kernel_item(0, 0)],
        &place(text, d),
        &mut scratch,
        &mut out,
    );
    out.sort_unstable();
    let predicates = out
        .iter()
        .map(|&p| nfa.predicates[nfa.positions[p as usize].predicate as usize])
        .collect();
    (predicates, accept)
}

#[test]
fn literal_positions_step_one_pattern_character_at_a_time() {
    let nfa = nfa_of("abca");
    assert_eq!(nfa.positions.len(), 4);
    // `a` twice: one predicate.
    assert_eq!(nfa.predicates.len(), 3);
    assert_eq!(nfa.positions[0].predicate, nfa.positions[3].predicate);
    let pc = nfa.positions[0].pc as usize;
    for (i, position) in nfa.positions.iter().enumerate() {
        assert_eq!(position.pc as usize, pc);
        assert_eq!(position.lit_off as usize, i);
    }
    assert_eq!(nfa.positions[0].next, kernel_item(pc, 1));
    assert_eq!(nfa.positions[2].next, kernel_item(pc, 3));
    // The last character resumes at the opcode after the literal.
    assert_eq!(nfa.positions[3].next, kernel_item(pc + 2 + 4, 0));
    assert_eq!(nfa.position_at(kernel_item(pc, 2)), Some(2));
    assert_eq!(nfa.position_at(kernel_item(pc, 5)), None);
}

#[test]
fn multibyte_literal_positions_follow_character_lengths() {
    let nfa = nfa_of("é中x");
    let offsets: Vec<u8> = nfa.positions.iter().map(|p| p.lit_off).collect();
    assert_eq!(offsets, vec![0, 2, 5]);
    let pc = nfa.positions[0].pc as usize;
    assert_eq!(nfa.positions[0].next, kernel_item(pc, 2));
    assert_eq!(nfa.positions[1].next, kernel_item(pc, 5));
    assert_eq!(nfa.positions[2].next, kernel_item(pc + 2 + 6, 0));
}

#[test]
fn every_consuming_opcode_is_one_position_with_its_test() {
    let nfa = nfa_of(".[a-z][^0-9]\\w\\W\\s-\\cg\\Cg");
    let kinds: Vec<Predicate> = nfa
        .positions
        .iter()
        .map(|p| nfa.predicates[p.predicate as usize])
        .collect();
    assert!(matches!(kinds[0], Predicate::AnyChar));
    assert!(matches!(kinds[1], Predicate::Charset { .. }));
    assert!(matches!(kinds[2], Predicate::Charset { .. }));
    assert!(matches!(kinds[3], Predicate::Syntax { negate: false, .. }));
    assert!(matches!(kinds[4], Predicate::Syntax { negate: true, .. }));
    assert!(matches!(kinds[5], Predicate::Syntax { negate: false, .. }));
    assert!(matches!(
        kinds[6],
        Predicate::Category {
            category: b'g',
            negate: false
        }
    ));
    assert!(matches!(
        kinds[7],
        Predicate::Category {
            category: b'g',
            negate: true
        }
    ));
    assert!(nfa.uses_categories);
    // The fused `\w\|\s_` is one position with a mask.
    let fused = nfa_of("\\(?:\\w\\|\\s_\\)+");
    assert!(
        fused
            .predicates
            .iter()
            .any(|p| matches!(p, Predicate::SyntaxSet { .. }))
    );
}

#[test]
fn closure_follows_both_edges_of_every_split() {
    let nfa = nfa_of("\\(?:ab\\|c\\)\\|d*e");
    let (predicates, accept) = start_closure(&nfa, b"", 0);
    assert!(!accept);
    // `a`, `c`, `d` and `e` (the empty `d*`).
    assert_eq!(predicates.len(), 4);
}

#[test]
fn closure_reads_the_rewind_view_of_keep_string_loops() {
    let compiled = regex_compile("[a-z]*:", false, false).unwrap();
    assert!(
        compiled
            .buffer
            .contains(&(RegexOp::OnFailureKeepStringJump as u8))
    );
    let nfa = Nfa::build(&compiled).unwrap();
    assert!(
        !nfa.bytecode
            .contains(&(RegexOp::OnFailureKeepStringJump as u8))
    );
    let (predicates, _) = start_closure(&nfa, b"", 0);
    assert_eq!(predicates.len(), 2, "the loop body and the continuation");
}

#[test]
fn closure_evaluates_assertions_at_the_real_position() {
    let nfa = nfa_of("^a\\|\\bb\\|c$");
    let text = b"xa b\nc";
    // At 0: `^` and `\b` hold (buffer start); `$` is tested after `c` only.
    assert_eq!(start_closure(&nfa, text, 0).0.len(), 3);
    // At 1 ("x|a"): no line start, no word boundary.
    assert_eq!(start_closure(&nfa, text, 1).0.len(), 1);
    // At 3 (" |b"): a word boundary.
    assert_eq!(start_closure(&nfa, text, 3).0.len(), 2);
    // At 5 ("\n|c"): a line start and a word boundary.
    assert_eq!(start_closure(&nfa, text, 5).0.len(), 3);
    let empty = nfa_of("x\\|^$");
    assert!(
        start_closure(&empty, b"a\n\nb", 2).1,
        "`^$` matches at an empty line"
    );
    assert!(!start_closure(&empty, b"a\n\nb", 1).1);
}

#[test]
fn eligibility_rejects_what_the_dfa_does_not_model() {
    for (pattern, why) in [
        ("\\(a\\)\\1", DfaIneligible::Backreference),
        ("a\\{2,3\\}", DfaIneligible::IntervalCounter),
        ("\\(?:a?\\)*?b", DfaIneligible::NullableNonGreedyLoop),
        ("a*", DfaIneligible::MatchesEmptyEverywhere),
        ("\\(?:a\\|\\)", DfaIneligible::MatchesEmptyEverywhere),
        ("", DfaIneligible::MatchesEmptyEverywhere),
    ] {
        let compiled = regex_compile(pattern, false, false).unwrap();
        assert_eq!(Nfa::build(&compiled).err(), Some(why), "{pattern:?}");
    }
    // An empty match behind a test is fine: the test can fail.
    for pattern in ["^a*", "\\(?:a\\|\\)\\b", "\\=", "x??y", "\\(a*\\)*b"] {
        let compiled = regex_compile(pattern, false, false).unwrap();
        assert!(Nfa::build(&compiled).is_ok(), "{pattern:?}");
    }
    // POSIX patterns are eligible: existence ignores which match is chosen.
    let posix = regex_compile("\\(a\\|ab\\)c", true, false).unwrap();
    assert!(Nfa::build(&posix).is_ok());
}

#[test]
fn nfa_counts_the_fail_stack_push_sites() {
    for pattern in ["a\\|b", "\\(a*\\)b", "[a-z]*:", "x\\(?:a\\|b\\)*c"] {
        let compiled = regex_compile(pattern, false, false).unwrap();
        let nfa = Nfa::build(&compiled).unwrap();
        assert_eq!(nfa.push_sites, fail_stack_push_sites(&compiled.buffer));
        assert!(nfa.push_sites > 0, "{pattern:?}");
    }
}

/// A closure that accepts at `p` is a zero-length match the backtracker must
/// find too (it may prefer a longer one).
#[test]
fn an_accepting_start_closure_is_a_match() {
    let syntax = DefaultSyntaxLookup;
    let text = b"ab \n cd\n\nx_y ";
    for pattern in [
        "^$",
        "\\b",
        "\\_>",
        "a?$",
        "\\(?:x\\|\\<\\)",
        "\\B\\|q",
        "\\'",
    ] {
        let compiled = regex_compile(pattern, false, false).unwrap();
        let nfa = Nfa::build(&compiled).unwrap();
        for p in 0..=text.len() {
            let (_, accept) = start_closure(&nfa, text, p);
            let matched = re_match(&compiled, text, p, text.len(), &syntax, 0);
            if accept {
                assert!(matched.is_some(), "{pattern:?} at {p}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Character classes (C3)
// ---------------------------------------------------------------------------

fn encode(code: u32) -> Vec<u8> {
    let mut buf = [0u8; 8];
    let len = emacs_char::char_string(code, &mut buf);
    buf[..len].to_vec()
}

// ---------------------------------------------------------------------------
// The lazy DFA (C4)
// ---------------------------------------------------------------------------

use crate::engine::matcher::fail::{MatchScratch, take_matcher_overflow};

use crate::engine::matcher::route::{take_fail_stack_probe, take_pike_fallback};

struct DfaRng(u64);

impl DfaRng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n.max(1) as u64) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const GEN_ATOMS: &[&str] = &[
    "a",
    "b",
    "c",
    "x",
    "A",
    " ",
    "-",
    "_",
    ":",
    "\n",
    "é",
    "中",
    "Ж",
    ".",
    "[a-c]",
    "[^a-c\n]",
    "[[:alpha:]]",
    "[[:space:]]",
    "[[:upper:]]",
    "[[:word:]]",
    "[[:punct:]]",
    "[é-ü]",
    "\\w",
    "\\W",
    "\\s-",
    "\\s_",
    "\\sw",
    "\\S-",
    "\\cg",
    "\\C|",
    "\\(?:\\w\\|\\s_\\)",
];
const GEN_ZERO_WIDTH: &[&str] = &[
    "^", "$", "\\`", "\\'", "\\b", "\\B", "\\<", "\\>", "\\_<", "\\_>", "\\=",
];
const GEN_QUANTIFIERS: &[&str] = &["", "", "", "*", "+", "?", "*?", "+?", "??"];

/// A random pattern over the whole eligible vocabulary, quantifiers nested.
fn gen_pattern(rng: &mut DfaRng, depth: usize) -> String {
    let mut out = String::new();
    let arms = 1 + rng.below(3);
    for arm in 0..arms {
        if arm > 0 {
            out.push_str("\\|");
        }
        for _ in 0..1 + rng.below(4) {
            let atom = match rng.below(if depth == 0 { 6 } else { 9 }) {
                0..=3 => rng.pick(GEN_ATOMS).to_string(),
                4 | 5 => rng.pick(GEN_ZERO_WIDTH).to_string(),
                6 | 7 => format!("\\(?:{}\\)", gen_pattern(rng, depth - 1)),
                _ => format!("\\({}\\)", gen_pattern(rng, depth - 1)),
            };
            out.push_str(&atom);
            if !GEN_ZERO_WIDTH.contains(&atom.as_str()) {
                out.push_str(rng.pick(GEN_QUANTIFIERS));
            }
        }
    }
    out
}

/// A random valid multibyte text (raw bytes encoded as Emacs does).
fn gen_text(rng: &mut DfaRng, max_len: usize) -> Vec<u8> {
    const PIECES: &[&str] = &[
        "a", "b", "c", "x", "A", "B", " ", " ", "\n", "-", "_", ":", "!", "é", "É", "中", "日",
        "Ж", "α", "K", "ab", "cab",
    ];
    let mut text = Vec::new();
    for _ in 0..rng.below(max_len) {
        if rng.below(12) == 0 {
            text.extend(encode(emacs_char::byte8_to_char(
                0x80 + rng.below(0x80) as u8,
            )));
        } else {
            text.extend_from_slice(rng.pick(PIECES).as_bytes());
        }
    }
    text
}

fn char_boundaries(text: &[u8], multibyte: bool) -> Vec<usize> {
    let mut out = Vec::new();
    let mut d = 0;
    while d < text.len() {
        out.push(d);
        d += re_text_char(text, d, multibyte).map_or(1, |(_, len)| len);
    }
    out.push(text.len());
    out
}

/// Every verdict of the DFA against the matcher at every candidate of
/// `text`, for a few stops and points.  Returns (yes, no, unknown).
fn check_dfa_against_matcher(
    compiled: &CompiledPattern,
    dfa: &mut ExistenceDfa,
    text: &[u8],
    syntax: &dyn SyntaxLookup,
    label: &str,
) -> (usize, usize, usize) {
    let context = ClassContext::of_search(compiled, dfa.nfa(), syntax)
        .expect("test lookups have a class identity");
    dfa.classes_mut().sync(context);
    dfa.begin_search(compiled, syntax);
    let boundaries = char_boundaries(text, compiled.target_multibyte);
    let mut tally = (0, 0, 0);
    let stops = [text.len(), boundaries[boundaries.len() / 2]];
    for &stop in &stops {
        for &point in &[0, boundaries[boundaries.len() / 3], text.len()] {
            // One pass over the candidates is one search.
            dfa.begin_search(compiled, syntax);
            for &p in boundaries.iter().filter(|&&p| p <= stop) {
                let verdict = dfa.anchored_exists(compiled, text, p, stop, point, syntax);
                let _ = take_matcher_overflow();
                let matched = re_match(compiled, text, p, stop, syntax, point);
                if take_matcher_overflow() {
                    continue;
                }
                let where_ = || {
                    format!(
                        "{label}: at {p} stop {stop} point {point} in {:?}: {verdict:?} vs {:?}",
                        String::from_utf8_lossy(text),
                        matched.as_ref().map(|m| m.0)
                    )
                };
                match verdict {
                    Exists::Yes => {
                        assert!(matched.is_some(), "{}", where_());
                        tally.0 += 1;
                    }
                    Exists::No { .. } => {
                        assert!(matched.is_none(), "{}", where_());
                        tally.1 += 1;
                    }
                    Exists::Unknown => tally.2 += 1,
                }
            }
        }
    }
    tally
}

/// The org font-lock patterns of the P3.3 census over an org-like text.
#[test]
fn dfa_verdicts_agree_on_org_font_lock_patterns() {
    let text = "* TODO [#A] Heading :tag:work:\n  - item :: description\n  + deep item\n\
                | a | b |\n|---+---|\n| =x= | *y* |\n:PROPERTIES:\n:ID: 42\n:END:\n\
                Some [[https://example.org][link]] and <mailto:x@y> text.\n\
                ** DONE Sub :ARCHIVE:\n#+BEGIN_SRC elisp\n(defun f () 1)\n#+END_SRC\n";
    for source in [
        "^[ \t]*|\\(?:.*?|\\)? *\\(:?=[^|\n]*\\)",
        "\\(?:^[ \t]*[-+]\\|^[ \t]+[*]\\)[ \t]+\\(.*?[ \t]+::\\)\\([ \t]+\\|$\\)",
        "^\\*+.*?\\(\\[#\\([A-Z]\\|[0-9]\\|[1-5][0-9]\\)\\] ?\\)",
        "| *\\(<[lrc]?[0-9]*>\\)",
        "^\\*+ \\(.*:ARCHIVE:.*\\)",
        "^\\*+ \\(?:.*[ \t]\\)?\\(:\\([[:alnum:]_@#%:]+\\):\\)[ \t]*$",
        "^[ \t]*|\\( *\\([$!_^/]\\) *\\|.*\\)|",
        "^[ \t]*| *\\([#*]\\) *|",
        "^[ \t]*\\(:\\(?: .*\\|$\\)\n?\\)",
        "^\\(\\*+\\)\\(?: +\\(?:DONE\\)\\)\\(?: +\\(.*?\\)\\)?[ \t]*$",
        "\\(\\[\\[\\([^]]+\\)\\]\\(?:\\[\\([^]]+\\)\\]\\)?\\]\\|<\\(mailto\\|https?\\):\\([^>]+\\)>\\|\\<\\(https?\\|mailto\\):\\([^ \t\n]+\\)\\)",
        "(\\(\\(?:\\w\\|\\s_\\|\\\\.\\)+\\)\\_>",
    ] {
        for case_fold in [false, true] {
            let compiled = regex_compile(source, false, case_fold).unwrap();
            let nfa = Nfa::build(&compiled).unwrap_or_else(|why| panic!("{source:?}: {why:?}"));
            let mut dfa = ExistenceDfa::new(nfa);
            let tally = check_dfa_against_matcher(
                &compiled,
                &mut dfa,
                text.as_bytes(),
                &DefaultSyntaxLookup,
                source,
            );
            assert_eq!(tally.2, 0);
            assert!(tally.1 > 0, "{source:?} rejects some candidates");
        }
    }
}

/// The overflow bound's premise for a rejected candidate: the backtracker's
/// deepest fail stack there stays below `(consumed + 1) * 2 * push sites`.
#[test]
fn a_rejected_candidate_stays_within_the_overflow_bound() {
    let mut rng = DfaRng(0x0B0_0D);
    let syntax = DefaultSyntaxLookup;
    let mut measured = 0usize;
    for _ in 0..1_500 {
        let source = gen_pattern(&mut rng, 2);
        let Ok(compiled) = regex_compile(&source, rng.below(4) == 0, false) else {
            continue;
        };
        let Ok(nfa) = Nfa::build(&compiled) else {
            continue;
        };
        let sites = nfa.push_sites;
        let mut dfa = ExistenceDfa::new(nfa);
        let context = ClassContext::of_search(&compiled, dfa.nfa(), &syntax).unwrap();
        dfa.classes_mut().sync(context);
        let text = gen_text(&mut rng, 24);
        for p in char_boundaries(&text, true) {
            let Exists::No { consumed } =
                dfa.anchored_exists(&compiled, &text, p, text.len(), 0, &syntax)
            else {
                continue;
            };
            let _ = take_fail_stack_probe();
            let _ = take_matcher_overflow();
            // The backtrack budget caps the exponential shapes; the depth
            // bound holds at every step of the run, however it ends.
            let mut scratch = MatchScratch::default();
            let mut registers = MatchRegisters::default();
            let found = re_match_internal(
                &mut scratch,
                &compiled,
                &text,
                p,
                text.len(),
                &syntax,
                0,
                true,
                &mut registers,
            );
            let gave_up = take_pike_fallback();
            assert!(gave_up || found.is_none(), "{source:?} at {p}");
            let probe = take_fail_stack_probe();
            assert!(
                probe.max_depth <= (consumed + 1) * 2 * sites,
                "{source:?} at {p}: depth {} consumed {consumed} sites {sites}",
                probe.max_depth
            );
            measured += 1;
        }
    }
    assert!(measured > 5_000, "{measured}");
}

/// The cache is cleared past its cap and relearned, with the same verdicts.
#[test]
fn a_full_cache_is_cleared_and_relearned() {
    // Many distinct literal prefixes: one state per prefix position.
    let words: Vec<String> = (0..700).map(|i| format!("w{i:03}q")).collect();
    let source = words.join("\\|");
    let compiled = regex_compile(&source, false, false).unwrap();
    let nfa = Nfa::build(&compiled).unwrap();
    let mut dfa = ExistenceDfa::new(nfa);
    let text: String = (0..700).map(|i| format!("w{i:03}x ")).collect();
    let tally = check_dfa_against_matcher(
        &compiled,
        &mut dfa,
        text.as_bytes(),
        &DefaultSyntaxLookup,
        "many prefixes",
    );
    assert_eq!(tally.0, 0);
    assert!(tally.1 > 0);
    assert!(dfa.counters.clears > 0, "{:?}", dfa.counters);
    assert_eq!(dfa.gave_up(), None);
}

// ---------------------------------------------------------------------------
// The candidate filter in `re_search` (C5)
// ---------------------------------------------------------------------------

type SearchResult = Option<(usize, Vec<i64>, Vec<i64>)>;

fn search(
    compiled: &CompiledPattern,
    text: &[u8],
    start: usize,
    range: isize,
    syntax: &dyn SyntaxLookup,
    point: usize,
) -> (SearchResult, bool) {
    let _ = take_matcher_overflow();
    let found = re_search(compiled, text, start, range, syntax, point)
        .map(|(at, regs)| (at, regs.start.to_vec(), regs.end.to_vec()));
    (found, take_matcher_overflow())
}

#[test]
fn the_knob_reads_off_on_and_verify() {
    assert_eq!(DfaMode::parse(None), DfaMode::On);
    assert_eq!(DfaMode::parse(Some("off")), DfaMode::Off);
    assert_eq!(DfaMode::parse(Some("0")), DfaMode::Off);
    assert_eq!(DfaMode::parse(Some(" OFF ")), DfaMode::Off);
    assert_eq!(DfaMode::parse(Some("no")), DfaMode::Off);
    assert_eq!(DfaMode::parse(Some("bogus")), DfaMode::On);
    assert_eq!(DfaMode::parse(Some("on")), DfaMode::On);
    assert_eq!(DfaMode::parse(Some(" 1 ")), DfaMode::On);
    assert_eq!(DfaMode::parse(Some("ON")), DfaMode::On);
    assert_eq!(DfaMode::parse(Some("verify")), DfaMode::Verify);
}

/// A search whose candidates always succeed never borrows a cold DFA slot.
#[test]
fn cold_successful_searches_take_no_dfa_lease() {
    let syntax = DefaultSyntaxLookup;
    let compiled = regex_compile("(defun \\([-a-z0-9]+\\)", false, true).unwrap();
    let text = b"(defun example-name)";
    let expected = with_dfa_mode(DfaMode::Off, || {
        search(&compiled, text, 0, text.len() as isize, &syntax, 0)
    });
    assert!(expected.0.is_some());
    reset_dfa_stats();
    with_cold_path(true, || {
        with_dfa_mode(DfaMode::On, || {
            for _ in 0..64 {
                assert_eq!(
                    search(&compiled, text, 0, text.len() as isize, &syntax, 0),
                    expected
                );
            }
        });
    });
    assert!(!compiled.dfa.initialized());
    assert!(matches!(
        *compiled.dfa.slot(),
        DfaSlot::Cold {
            failed: 0,
            keyless: false
        }
    ));
    let stats = dfa_stats();
    assert_eq!(stats.searches, 0, "{stats:?}");
    assert_eq!(stats.builds, 0, "{stats:?}");
}

/// The threshold failure publishes the slot and leases it for the remaining
/// candidates of the same search, without counting its classic attempt twice.
#[test]
fn cold_failure_threshold_filters_the_rest_of_the_same_search() {
    let syntax = DefaultSyntaxLookup;
    let compiled = regex_compile("z[0-9]", false, false).unwrap();
    let dense = vec![b'z'; 128];
    reset_dfa_stats();
    with_cold_path(true, || {
        with_dfa_mode(DfaMode::On, || {
            for _ in 0..COLD_THRESHOLD * 2 {
                // An isolated failure followed by a match must not heat the
                // slot. Whole failed searches are admitted on completion.
                let found = search(&compiled, b"zz0", 0, 3, &syntax, 0);
                assert!(!found.1);
                assert_eq!(found.0.as_ref().map(|(at, _, _)| *at), Some(1));
            }
            assert!(!compiled.dfa.initialized());
            assert!(matches!(
                *compiled.dfa.slot(),
                DfaSlot::Cold {
                    failed: 0,
                    keyless: false
                }
            ));
            assert_eq!(dfa_stats().searches, 0);
            let before = matcher_entry_count();
            assert_eq!(
                search(&compiled, &dense, 0, dense.len() as isize, &syntax, 0),
                (None, false)
            );
            assert_eq!(matcher_entry_count() - before, COLD_THRESHOLD as u64);
        });
    });
    assert!(compiled.dfa.initialized());
    assert!(matches!(*compiled.dfa.slot(), DfaSlot::Live(_)));
    let stats = dfa_stats();
    assert_eq!(stats.builds, 1, "{stats:?}");
    assert_eq!(stats.searches, 1, "{stats:?}");
    assert!(stats.skipped > 100, "{stats:?}");
}

/// The standard classes with no table identity, as a host lookup whose
/// tables cannot key a DFA's classes.
struct KeylessLookup;

impl SyntaxLookup for KeylessLookup {
    fn char_syntax(&self, c: char) -> SyntaxClass {
        DefaultSyntaxLookup.char_syntax(c)
    }

    fn char_has_category(&self, c: char, cat: u8) -> bool {
        DefaultSyntaxLookup.char_has_category(c, cat)
    }

    fn cache_key(&self) -> SyntaxCacheKey {
        SyntaxCacheKey::Standard
    }

    fn table_write_tick(&self) -> u64 {
        0
    }
}

/// A pattern reading syntax under a lookup with no class identity builds
/// its DFA once, finds it cannot key the classes, and stops counting
/// failures under such a lookup; a lookup with an identity builds it.
#[test]
fn a_keyless_lookup_does_not_rebuild_a_syntax_pattern() {
    let compiled = regex_compile("\\w[0-9]", false, false).unwrap();
    // Not a multiple of the threshold: a rebuild after every 16 failures
    // would leave a count behind.
    let dense = vec![b'z'; 129];
    let before = matcher_entry_count();
    let expected = with_dfa_mode(DfaMode::Off, || {
        search(
            &compiled,
            &dense,
            0,
            dense.len() as isize,
            &KeylessLookup,
            0,
        )
    });
    let entries = matcher_entry_count() - before;
    assert_eq!(expected, (None, false));
    with_cold_path(true, || {
        with_dfa_mode(DfaMode::On, || {
            for _ in 0..4 {
                let before = matcher_entry_count();
                assert_eq!(
                    search(
                        &compiled,
                        &dense,
                        0,
                        dense.len() as isize,
                        &KeylessLookup,
                        0
                    ),
                    expected
                );
                // The matcher alone decides every candidate.
                assert_eq!(matcher_entry_count() - before, entries);
                assert!(matches!(
                    *compiled.dfa.slot(),
                    DfaSlot::Cold {
                        failed: 0,
                        keyless: true
                    }
                ));
            }
            assert_eq!(
                search(
                    &compiled,
                    &dense,
                    0,
                    dense.len() as isize,
                    &DefaultSyntaxLookup,
                    0
                ),
                expected
            );
        });
    });
    assert!(matches!(*compiled.dfa.slot(), DfaSlot::Live(_)));
}

/// A bracket class that reads no syntax table (`[:alnum:]', `[:digit:]')
/// keys the DFA's classes without the lookup's identity, so a keyless
/// lookup builds it; `[:word:]' reads the table and stays cold there.
#[test]
fn table_free_classes_build_under_a_keyless_lookup() {
    for (pattern, live) in [
        ("\\.~[-[:alnum:]]+~[[:digit:]]", true),
        ("\\.~[[:word:]]+~", false),
    ] {
        let compiled = regex_compile(pattern, false, false).unwrap();
        let text = b"file-0001.txt.~a1~ x.y.z.~bb~ q".repeat(4);
        let expected = with_dfa_mode(DfaMode::Off, || {
            search(&compiled, &text, 0, text.len() as isize, &KeylessLookup, 0)
        });
        with_cold_path(true, || {
            with_dfa_mode(DfaMode::On, || {
                for start in 0..text.len() {
                    let reference = with_dfa_mode(DfaMode::Off, || {
                        search(
                            &compiled,
                            &text,
                            start,
                            (text.len() - start) as isize,
                            &KeylessLookup,
                            0,
                        )
                    });
                    assert_eq!(
                        search(
                            &compiled,
                            &text,
                            start,
                            (text.len() - start) as isize,
                            &KeylessLookup,
                            0
                        ),
                        reference,
                        "{pattern} from {start}"
                    );
                }
                assert_eq!(
                    search(&compiled, &text, 0, text.len() as isize, &KeylessLookup, 0),
                    expected
                );
            });
        });
        assert_eq!(
            matches!(*compiled.dfa.slot(), DfaSlot::Live(_)),
            live,
            "{pattern}"
        );
    }
}

/// The compression and encryption handlers' `file-name-handler-alist'
/// patterns over file names, every start: a keyless lookup's DFA (built,
/// for the bracket classes read no table) answers as the matcher alone.
#[test]
fn file_name_handler_patterns_match_as_the_matcher_under_a_keyless_lookup() {
    let patterns = [
        r"\(?:\.tzst\|\.zst\|\.dz\|\.txz\|\.xz\|\.lzma\|\.lz\|\.g?z\|\.\(?:tgz\|svgz\|sifz\)\|\.tbz2?\|\.bz2\|\.Z\)\(?:~\|\.~[-[:alnum:]:#@^._]+\(?:~[[:digit:]]+\)?~\)?\'",
        r"\.gpg\(~\|\.~[0-9]+~\)?\'",
    ];
    let names: Vec<&[u8]> = vec![
        b"file-0001.txt",
        b"/home/u/.cache/d5000/file-0001.txt",
        b"a.tar.gz",
        b"b.el.gz.~1~",
        b"c.xz~",
        b"d.Z.~a.b~",
        b"e.tgz.~x:y~12~",
        b"f.tbz",
        b"g.tbz2",
        b"h.lzma.~#@^~",
        b"i.gz.~~",
        b"j.txt.gpg",
        b"k.gpg~",
        b"l.gpg.~3~",
        b"m.gpg.~x~",
        b"/x/.gz/n",
        b".gz",
        b"o.zst.~1~2~",
    ];
    for pattern in patterns {
        let compiled = regex_compile(pattern, false, false).unwrap();
        with_cold_path(true, || {
            for _ in 0..4 {
                for name in &names {
                    for start in 0..=name.len() {
                        let range = (name.len() - start) as isize;
                        let reference = with_dfa_mode(DfaMode::Off, || {
                            search(&compiled, name, start, range, &KeylessLookup, 0)
                        });
                        let found = with_dfa_mode(DfaMode::On, || {
                            search(&compiled, name, start, range, &KeylessLookup, 0)
                        });
                        assert_eq!(
                            found,
                            reference,
                            "{pattern} on {:?} from {start}",
                            String::from_utf8_lossy(name)
                        );
                    }
                }
            }
        });
        assert!(
            matches!(*compiled.dfa.slot(), DfaSlot::Live(_)),
            "{pattern}"
        );
    }
}

/// Off: the slot is never touched.  On: after 16 failed entries the DFA is
/// built, then skips candidates; the results are those of the matcher alone.
#[test]
fn searches_skip_rejected_candidates_once_the_dfa_is_built() {
    let syntax = DefaultSyntaxLookup;
    let compiled = regex_compile("\\(?:foo\\|bar\\)[0-9]+;", false, false).unwrap();
    let text = b"foo bar fo1 foox barbar1 foo2 bar33x foo9; tail bar7 end";
    with_dfa_mode(DfaMode::Off, || {
        for start in 0..text.len() {
            let _ = search(
                &compiled,
                text,
                start,
                (text.len() - start) as isize,
                &syntax,
                0,
            );
        }
    });
    assert!(matches!(
        *compiled.dfa.slot(),
        DfaSlot::Cold {
            failed: 0,
            keyless: false
        }
    ));
    let reference: Vec<_> = (0..=text.len())
        .map(|start| {
            with_dfa_mode(DfaMode::Off, || {
                search(
                    &compiled,
                    text,
                    start,
                    (text.len() - start) as isize,
                    &syntax,
                    0,
                )
            })
        })
        .collect();
    reset_dfa_stats();
    let entries_before = matcher_entry_count();
    for _ in 0..3 {
        for start in 0..=text.len() {
            let got = with_dfa_mode(DfaMode::On, || {
                search(
                    &compiled,
                    text,
                    start,
                    (text.len() - start) as isize,
                    &syntax,
                    0,
                )
            });
            assert_eq!(got, reference[start], "from {start}");
        }
    }
    let entries = matcher_entry_count() - entries_before;
    assert!(matches!(*compiled.dfa.slot(), DfaSlot::Live(_)));
    let stats = dfa_stats();
    assert_eq!(stats.builds, 1);
    assert!(stats.skipped > 100, "{stats:?}");
    assert!(entries < 3 * reference.len() as u64 * 4, "{entries}");
}

// ---------------------------------------------------------------------------
// `syntax-table` property runs and the propertize frontier (C7)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Cached first-step rejection (same-binary knob)
// ---------------------------------------------------------------------------

fn first_step_dfa(pattern: &CompiledPattern, syntax: &dyn SyntaxLookup) -> ExistenceDfa {
    let mut dfa = ExistenceDfa::new(Nfa::build(pattern).unwrap());
    let context = ClassContext::of_search(pattern, dfa.nfa(), syntax).unwrap();
    dfa.classes.sync(context);
    dfa.begin_search(pattern, syntax);
    dfa
}

/// A real candidate rejected from the cached start transition avoids the
/// outlined matcher, while a successful fallback and verify keep all registers.
#[test]
fn inline_first_step_rejects_and_success_fallback_agree_with_verify() {
    let syntax = DefaultSyntaxLookup;
    let compiled = regex_compile("\\<\\(foo\\)\\([0-9]\\);", false, false).unwrap();
    let text = b"xfoo7; foo8;";
    let expected = with_dfa_mode(DfaMode::Off, || {
        search(&compiled, text, 0, text.len() as isize, &syntax, 0)
    });
    assert!(expected.0.is_some());
    prime(&compiled, &syntax).unwrap();
    // Warm an actual DEAD start transition and the successful continuation.
    with_first_step(false, || {
        with_dfa_mode(DfaMode::On, || {
            assert_eq!(
                search(&compiled, text, 0, text.len() as isize, &syntax, 0),
                expected
            );
        });
    });
    reset_dfa_stats();
    with_first_step(true, || {
        with_dfa_mode(DfaMode::On, || {
            let mut lease = DfaLease::acquire(&compiled, &syntax, text.len()).unwrap();
            assert!(lease.inline_first_step_enabled());
            let (before_no, before_decisions) = match &*lease.slot {
                DfaSlot::Live(live) => (live.dfa.counters.no, live.decisions),
                _ => panic!("primed slot should be live"),
            };
            let entries = matcher_entry_count();
            assert!(lease.try_inline_first_step_skip(&compiled, text, 1, text.len(), 0));
            assert_eq!(matcher_entry_count(), entries);
            let DfaSlot::Live(live) = &*lease.slot else {
                panic!("a cached rejection keeps the slot live");
            };
            assert_eq!(live.dfa.counters.no, before_no + 1);
            assert_eq!(live.decisions, before_decisions + 1);
            assert_eq!(lease.skipped, 1);
            // The real match falls through exactly once, with full registers.
            assert!(!lease.try_inline_first_step_skip(&compiled, text, 7, text.len(), 0));
            let mut scratch = MatchScratch::default();
            let mut registers = MatchRegisters::default();
            let end = lease.candidate::<false>(
                &mut scratch,
                &compiled,
                text,
                7,
                text.len(),
                &syntax,
                0,
                &mut registers,
            );
            assert_eq!(end, Some(text.len()));
            assert_eq!(matcher_entry_count() - entries, 1);
            assert_eq!(
                Some((7, registers.start.to_vec(), registers.end.to_vec())),
                expected.0
            );
        });
        assert_eq!(dfa_stats().skipped, 1);
        assert_eq!(dfa_stats().no, 1);
        assert_eq!(dfa_stats().yes, 1);
        // Exercise the actual re_search macro, then the verify-only path.
        reset_dfa_stats();
        let entries = matcher_entry_count();
        let on = with_dfa_mode(DfaMode::On, || {
            search(&compiled, text, 0, text.len() as isize, &syntax, 0)
        });
        assert_eq!(on, expected);
        assert_eq!(matcher_entry_count() - entries, 1);
        assert_eq!(dfa_stats().skipped, 1);
        let entries = matcher_entry_count();
        let verified = with_dfa_mode(DfaMode::Verify, || {
            let lease = DfaLease::acquire(&compiled, &syntax, text.len()).unwrap();
            assert!(!lease.inline_first_step_enabled());
            drop(lease);
            search(&compiled, text, 0, text.len() as isize, &syntax, 0)
        });
        assert_eq!(verified, expected);
        assert_eq!(matcher_entry_count() - entries, 2);
        let stats = dfa_stats();
        assert_eq!(stats.verify_bad_no + stats.verify_bad_yes, 0, "{stats:?}");
    });
}

#[test]
fn cached_first_step_rejections_equal_the_matcher() {
    let syntax = DefaultSyntaxLookup;
    for source in ["\\<foo", "^foo", "\\`foo", "\\(?:bar\\|baz\\)"] {
        let compiled = regex_compile(source, false, false).unwrap();
        let mut dfa = first_step_dfa(&compiled, &syntax);
        let text = b"xfoo";
        let p = 1;
        assert_eq!(
            with_first_step(false, || {
                dfa.anchored_exists(&compiled, text, p, text.len(), 0, &syntax)
            }),
            Exists::No { consumed: 0 },
            "{source}"
        );
        assert!(dfa.cached_first_step_dead(&compiled, text, p, text.len(), 0));
        let before = dfa.counters;
        assert_eq!(
            with_first_step(true, || {
                dfa.anchored_exists(&compiled, text, p, text.len(), 0, &syntax)
            }),
            Exists::No { consumed: 0 }
        );
        assert_eq!(dfa.counters.no, before.no + 1);
        assert_eq!(dfa.counters.bytes, before.bytes);
        assert_eq!(dfa.counters.states, before.states);
        assert!(re_match(&compiled, text, p, text.len(), &syntax, 0).is_none());
    }
}

#[test]
fn cached_first_step_respects_point_stop_and_unknown_entries() {
    let syntax = DefaultSyntaxLookup;
    let compiled = regex_compile("\\=foo", false, false).unwrap();
    let mut dfa = first_step_dfa(&compiled, &syntax);
    let text = b"xfoo";
    with_first_step(false, || {
        assert_eq!(
            dfa.anchored_exists(&compiled, text, 1, text.len(), 0, &syntax),
            Exists::No { consumed: 0 }
        );
    });
    assert!(dfa.cached_first_step_dead(&compiled, text, 1, text.len(), 0));
    // The same candidate now satisfies point: the cached failure is invalid.
    assert!(!dfa.cached_first_step_dead(&compiled, text, 1, text.len(), 1));
    assert_eq!(
        with_first_step(true, || {
            dfa.anchored_exists(&compiled, text, 1, text.len(), 1, &syntax)
        }),
        Exists::Yes
    );
    assert!(re_match(&compiled, text, 1, text.len(), &syntax, 1).is_some());
    assert!(!dfa.cached_first_step_dead(&compiled, text, 1, 1, 0));
    assert!(!dfa.cached_first_step_dead(&compiled, text, text.len(), text.len(), 0));
    // A byte that has never been classified must use the normal loop.
    assert!(!dfa.cached_first_step_dead(&compiled, b"xqoo", 1, text.len(), 0));
    let class = dfa.classes.byte_class[b'f' as usize];
    let start = dfa.start[0] << dfa.stride_shift;
    let at = start as usize + class as usize;
    let original = dfa.trans[at];
    for unresolved in [UNKNOWN, SLOW, MATCH, start] {
        dfa.trans[at] = unresolved;
        assert!(!dfa.cached_first_step_dead(&compiled, text, 1, text.len(), 0));
    }
    dfa.trans[at] = original;
    dfa.clear_states();
    assert!(!dfa.cached_first_step_dead(&compiled, text, 1, text.len(), 0));
}

#[test]
fn cached_first_step_keeps_memory_and_give_up_guards() {
    let syntax = DefaultSyntaxLookup;
    let compiled = regex_compile("z[0-9]", false, false).unwrap();
    let mut dfa = first_step_dfa(&compiled, &syntax);
    let text = b"y";
    with_first_step(false, || {
        assert_eq!(
            dfa.anchored_exists(&compiled, text, 0, text.len(), 0, &syntax),
            Exists::No { consumed: 0 }
        );
    });
    assert!(dfa.cached_first_step_dead(&compiled, text, 0, text.len(), 0));
    dfa.memory = MEMORY_CAP + 1;
    let clears = dfa.counters.clears;
    assert_eq!(
        with_first_step(true, || {
            dfa.anchored_exists(&compiled, text, 0, text.len(), 0, &syntax)
        }),
        Exists::No { consumed: 0 }
    );
    assert_eq!(dfa.counters.clears, clears + 1);
    dfa.gave_up = Some(DfaGaveUp::StateExplosion);
    assert_eq!(
        with_first_step(true, || {
            dfa.anchored_exists(&compiled, text, 0, text.len(), 0, &syntax)
        }),
        Exists::Unknown
    );
}

/// The filter's one-compare overflow bound answers exactly as the bound (for
/// every span a text can have: `consumed` is below `usize::MAX`).
#[test]
fn the_overflow_free_span_is_the_fail_stack_bound() {
    use crate::engine::matcher::fail::fail_stack_may_overflow_with;
    for push_sites in [
        0,
        1,
        2,
        3,
        7,
        64,
        100,
        4_096,
        133_332,
        133_333,
        266_666,
        usize::MAX / 3,
    ] {
        let span = fail_stack_overflow_free_span(push_sites);
        for consumed in [
            0,
            1,
            2,
            5,
            999,
            1_300,
            1_332,
            1_333,
            1_334,
            133_332,
            133_333,
            usize::MAX - 1,
        ]
        .into_iter()
        .chain([span.saturating_sub(1), span, span.saturating_add(1)])
        .map(|consumed| consumed.min(usize::MAX - 1))
        {
            assert_eq!(
                consumed < span,
                !fail_stack_may_overflow_with(push_sites, consumed),
                "push sites {push_sites}, consumed {consumed}, span {span}"
            );
        }
    }
}

/// A rejection whose consumed span could have filled GNU's fail stack runs
/// the matcher, which signals the overflow as GNU does.
#[test]
fn a_rejection_that_could_overflow_runs_the_matcher() {
    let syntax = DefaultSyntaxLookup;
    let compiled = regex_compile("x\\(?:a\\|b\\)*c", false, false).unwrap();
    let short = b"xab xba xabab xb xa xx xab xb xa xabb xa xbb xa xb xab xba xab xa";
    let long = [&b"x"[..], &b"ab".repeat(100_000)].concat();
    reset_dfa_stats();
    with_dfa_mode(DfaMode::On, || {
        // Warm the slot: 16 failed entries build the DFA.
        for _ in 0..3 {
            let _ = search(&compiled, short, 0, short.len() as isize, &syntax, 0);
        }
        assert!(matches!(*compiled.dfa.slot(), DfaSlot::Live(_)));
        let (found, overflow) = search(&compiled, &long, 0, long.len() as isize, &syntax, 0);
        assert_eq!(found, None);
        assert!(overflow, "GNU's fail-stack overflow");
    });
    assert!(dfa_stats().overflow_guarded >= 1);
}

/// A pattern whose candidates mostly match goes on holiday.
#[test]
fn mostly_matching_candidates_send_the_pattern_on_holiday() {
    let syntax = DefaultSyntaxLookup;
    let compiled = regex_compile("[a-z]+", false, false).unwrap();
    let failing = b"1234567890 1234567890 12345";
    let matching = b"ab cd ef gh ij kl";
    reset_dfa_stats();
    with_dfa_mode(DfaMode::On, || {
        // Build it on failures (the fastmap admits no digit, so fail with a
        // pattern-shaped text: every candidate fails at its second char).
        let failing_pattern = regex_compile("[a-z][0-9]", false, false).unwrap();
        for _ in 0..2 {
            let _ = search(
                &failing_pattern,
                failing,
                0,
                failing.len() as isize,
                &syntax,
                0,
            );
        }
        let _ = failing_pattern;
        // `[a-z]+` searched from each letter: every candidate matches.
        for _ in 0..10 {
            for start in 0..matching.len() {
                let _ = search(
                    &compiled,
                    matching,
                    start,
                    (matching.len() - start) as isize,
                    &syntax,
                    0,
                );
            }
        }
    });
    // `[a-z]+` never fails, so it never builds: no holiday needed.
    assert!(matches!(
        *compiled.dfa.slot(),
        DfaSlot::Cold {
            failed: 0,
            keyless: false
        }
    ));
    // A pattern that fails enough to build, then matches: holiday.
    let compiled = regex_compile("[a-z]+;", false, false).unwrap();
    let mixed_fail = b"ab cd ef gh ij kl mn op qr st uv wx yz";
    let mixed_match = b"a; b; c; d; e; f; g; h; i; j; k; l; m;";
    with_dfa_mode(DfaMode::On, || {
        let _ = search(
            &compiled,
            mixed_fail,
            0,
            mixed_fail.len() as isize,
            &syntax,
            0,
        );
        assert!(matches!(*compiled.dfa.slot(), DfaSlot::Live(_)));
        for _ in 0..8 {
            for start in (0..mixed_match.len()).step_by(3) {
                let _ = search(
                    &compiled,
                    mixed_match,
                    start,
                    (mixed_match.len() - start) as isize,
                    &syntax,
                    0,
                );
            }
        }
    });
    assert!(dfa_stats().holiday_off > 0, "{:?}", dfa_stats());
}

// ---------------------------------------------------------------------------
// Differential fuzz smoke (C6)
// ---------------------------------------------------------------------------

/// The sparse scan still runs its classic EOF attempt, but repeated
/// successful passes ending there cannot heat a never-useful cold DFA.
#[test]
fn cold_successful_passes_with_eof_failures_never_build() {
    let syntax = DefaultSyntaxLookup;
    let compiled = regex_compile("\\(z\\)[0-9]", false, false).unwrap();
    let text = b"z1 z2";
    let pass = || {
        [
            search(&compiled, text, 0, text.len() as isize, &syntax, 0),
            search(&compiled, text, 2, (text.len() - 2) as isize, &syntax, 0),
            search(&compiled, text, text.len(), 0, &syntax, 0),
        ]
    };
    let expected = with_dfa_mode(DfaMode::Off, pass);
    assert!(expected[0].0.is_some() && expected[1].0.is_some());
    assert_eq!(expected[2], (None, false));
    reset_dfa_stats();
    with_cold_path(true, || {
        with_dfa_mode(DfaMode::On, || {
            for _ in 0..COLD_THRESHOLD * 4 {
                let before = matcher_entry_count();
                assert_eq!(pass(), expected);
                assert_eq!(
                    matcher_entry_count() - before,
                    3,
                    "two matches and the unchanged classic EOF attempt"
                );
            }
        });
    });
    assert!(!compiled.dfa.initialized());
    assert!(matches!(
        *compiled.dfa.slot(),
        DfaSlot::Cold {
            failed: 0,
            keyless: false
        }
    ));
    let stats = dfa_stats();
    assert_eq!(stats.searches, 0, "{stats:?}");
    assert_eq!(stats.builds, 0, "{stats:?}");
}

/// The board's backquoted `(defun . ,f)` is a real isolated miss before
/// another true header. Repeating that scan cannot heat the mostly matching
/// pattern, even after the old cumulative threshold would have built it.
#[test]
fn cold_isolated_literal_failures_never_build() {
    let syntax = DefaultSyntaxLookup;
    let mut text = vec![b'x'; 4096];
    text.extend_from_slice(b"\n(defun first)\n`(defun . ,f)\n(defun second)\n");
    for fold in [false, true] {
        let compiled = regex_compile("(defun \\([-a-z0-9]+\\)", false, fold).unwrap();
        let pass = || {
            let mut start = 0;
            let mut results = Vec::new();
            loop {
                let result = search(
                    &compiled,
                    &text,
                    start,
                    (text.len() - start) as isize,
                    &syntax,
                    start,
                );
                let next = result.0.as_ref().map(|(_, _, ends)| ends[0] as usize);
                results.push(result);
                let Some(end) = next else {
                    break;
                };
                start = end;
            }
            results
        };
        let expected = with_dfa_mode(DfaMode::Off, pass);
        assert_eq!(
            expected.len(),
            3,
            "two captures and the terminal failed search"
        );
        assert!(expected[0].0.is_some() && expected[1].0.is_some());
        assert_eq!(expected[2], (None, false));
        reset_dfa_stats();
        with_cold_path(true, || {
            with_dfa_mode(DfaMode::On, || {
                for _ in 0..COLD_THRESHOLD * 4 {
                    let before = matcher_entry_count();
                    assert_eq!(pass(), expected);
                    assert_eq!(
                        matcher_entry_count() - before,
                        3,
                        "two matches, one real failure, no prefilter EOF attempt"
                    );
                }
            });
        });
        assert!(!compiled.dfa.initialized());
        assert!(matches!(
            *compiled.dfa.slot(),
            DfaSlot::Cold {
                failed: 0,
                keyless: false
            }
        ));
        let stats = dfa_stats();
        assert_eq!(stats.searches, 0, "{stats:?}");
        assert_eq!(stats.builds, 0, "{stats:?}");
        let verify = with_cold_path(true, || with_dfa_mode(DfaMode::Verify, pass));
        assert_eq!(verify, expected);
        assert_eq!(dfa_stats().verify_bad_no + dfa_stats().verify_bad_yes, 0);
    }
}

#[test]
fn inline_cached_prefix_keeps_consumed_accounting_and_captures() {
    let syntax = DefaultSyntaxLookup;
    let compiled = regex_compile("\\(zz\\)[0-9]", false, false).unwrap();
    let text = b"zzz zz7";
    let expected = with_dfa_mode(DfaMode::Off, || {
        search(&compiled, text, 0, text.len() as isize, &syntax, 0)
    });
    assert!(expected.0.is_some());
    prime(&compiled, &syntax).unwrap();
    with_first_step(false, || {
        with_dfa_mode(DfaMode::On, || {
            assert_eq!(
                search(&compiled, text, 0, text.len() as isize, &syntax, 0),
                expected
            );
        });
    });
    with_first_step(true, || {
        with_dfa_mode(DfaMode::On, || {
            let mut lease = DfaLease::acquire(&compiled, &syntax, text.len()).unwrap();
            let DfaSlot::Live(live) = &mut *lease.slot else {
                panic!("primed slot should be live");
            };
            let safe_span = live.overflow_free_span;
            live.overflow_free_span = 2;
            assert!(!lease.try_inline_first_step_skip(&compiled, text, 0, text.len(), 0));
            let DfaSlot::Live(live) = &mut *lease.slot else {
                panic!("guarded rejection should stay live");
            };
            live.overflow_free_span = safe_span;
            let before = live.dfa.counters;
            let entries = matcher_entry_count();
            assert!(lease.try_inline_first_step_skip(&compiled, text, 0, text.len(), 0));
            assert_eq!(matcher_entry_count(), entries);
            let DfaSlot::Live(live) = &*lease.slot else {
                panic!("cached rejection should stay live");
            };
            assert_eq!(live.dfa.counters.no, before.no + 1);
            assert_eq!(live.dfa.counters.bytes, before.bytes + 2);
            assert_eq!(lease.skipped, 1);
        });
        let entries = matcher_entry_count();
        let on = with_dfa_mode(DfaMode::On, || {
            search(&compiled, text, 0, text.len() as isize, &syntax, 0)
        });
        assert_eq!(on, expected);
        assert_eq!(matcher_entry_count() - entries, 1);
        let entries = matcher_entry_count();
        let verified = with_dfa_mode(DfaMode::Verify, || {
            search(&compiled, text, 0, text.len() as isize, &syntax, 0)
        });
        assert_eq!(verified, expected);
        assert_eq!(matcher_entry_count() - entries, 4);
        let stats = dfa_stats();
        assert_eq!(stats.verify_bad_no + stats.verify_bad_yes, 0, "{stats:?}");
    });
}
