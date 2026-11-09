//! The host boundary itself: what the engine reads through `host`,
//! `SyntaxLookup::table_write_tick`, and `CaseTable`.  New with the
//! extraction; everything else in `tests/` moved from romacs-core unchanged.

use crate::CaseTable;
use crate::SyntaxClass;
use crate::engine::compile::regex_compile;
use crate::engine::dfa;
use crate::engine::matcher::route::with_backtracker_forced;
use crate::engine::search::re_search;
use crate::engine::syntax::{
    DefaultSyntaxLookup, LookupClassKey, SyntaxCacheKey, SyntaxLookup, default_char_has_category,
};
use crate::engine::translate::CaseTranslation;
use crate::host;
use std::cell::Cell;

/// `a*ab` over `aaab` matches only after backtracking out of the greedy
/// `a*`, so the matcher reaches its quit poll at `goto_fail`.
#[test]
fn installed_quit_probe_aborts_a_backtracking_match() {
    let compiled = regex_compile("a*ab", false, false).unwrap();
    let text = b"aaab";
    let run = || {
        with_backtracker_forced(|| {
            dfa::filter::with_dfa_mode(dfa::filter::DfaMode::Off, || {
                re_search(
                    &compiled,
                    text,
                    0,
                    text.len() as isize,
                    &DefaultSyntaxLookup,
                    0,
                )
                .map(|(at, _)| at)
            })
        })
    };
    let previous = host::set_quit_probe(None);
    assert_eq!(run(), Some(0), "no probe: the match completes");
    host::set_quit_probe(Some(|| true));
    assert_eq!(run(), None, "a pending quit aborts the match");
    host::set_quit_probe(Some(|| false));
    assert_eq!(
        run(),
        Some(0),
        "a probe with no quit pending lets it complete"
    );
    host::set_quit_probe(previous);
}

/// A lookup over a mutable host table: `x` is a word constituent or not,
/// and the tick moves on every change, as `table_write_tick` requires.
struct TickedLookup {
    x_is_word: Cell<bool>,
    tick: Cell<u64>,
}

impl TickedLookup {
    fn set_x_is_word(&self, word: bool) {
        self.x_is_word.set(word);
        self.tick.set(self.tick.get() + 1);
    }
}

impl SyntaxLookup for TickedLookup {
    fn char_syntax(&self, c: char) -> SyntaxClass {
        if c == 'x' {
            if self.x_is_word.get() {
                SyntaxClass::Word
            } else {
                SyntaxClass::Punctuation
            }
        } else {
            crate::standard_syntax_class_for_char(c)
        }
    }

    fn char_has_category(&self, c: char, cat: u8) -> bool {
        default_char_has_category(c, cat)
    }

    fn cache_key(&self) -> SyntaxCacheKey {
        SyntaxCacheKey::Standard
    }

    fn class_cache_key(&self) -> Option<LookupClassKey> {
        // One table identity throughout: only the tick says it changed.
        Some(LookupClassKey::Tables {
            syntax: 1,
            category: 0,
        })
    }

    fn table_write_tick(&self) -> u64 {
        self.tick.get()
    }

    fn position_dependent(&self) -> bool {
        false
    }
}

/// The existence DFA memoizes each character's class per pattern.  Keyed by
/// the table identity alone, a memo taken while `x` was punctuation would
/// reject `x` after the host made it a word constituent; the tick is what
/// invalidates it.
#[test]
fn dfa_class_memo_follows_the_lookup_table_write_tick() {
    let lookup = TickedLookup {
        x_is_word: Cell::new(false),
        tick: Cell::new(0),
    };
    let compiled = regex_compile("\\sw+;", false, false).unwrap();
    let text = b"x;";
    let run = || {
        dfa::filter::with_dfa_mode(dfa::filter::DfaMode::On, || {
            dfa::filter::with_cold_path(false, || {
                re_search(&compiled, text, 0, text.len() as isize, &lookup, 0).map(|(at, _)| at)
            })
        })
    };
    dfa::filter::prime(&compiled, &lookup).expect("the pattern is DFA-eligible");
    assert_eq!(run(), None, "x is punctuation: no word run");
    lookup.set_x_is_word(true);
    assert_eq!(run(), Some(0), "the memo must not outlive the tick");
}

/// A case table whose translation of one non-ASCII code can change.
#[derive(Debug)]
struct MutableCaseTable {
    zhe_canon: Cell<u32>,
    tick: Cell<u64>,
}

impl CaseTable for MutableCaseTable {
    fn translate(&self, c: u32) -> u32 {
        if c == 0x416 { self.zhe_canon.get() } else { c }
    }

    fn write_tick(&self) -> u64 {
        self.tick.get()
    }

    fn folds_into_ascii(&self, _tick: u64, _allow_walk: bool) -> Option<bool> {
        Some(false)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Codes from 256 up are memoized per translation; an in-place table edit
/// is visible exactly when the table's tick moves.
#[test]
fn case_translation_wide_memo_follows_the_case_table_write_tick() {
    let table = std::rc::Rc::new(MutableCaseTable {
        zhe_canon: Cell::new(0x416),
        tick: Cell::new(7),
    });
    let translation = CaseTranslation::from_case_table(table.clone());
    assert_eq!(translation.translate(0x416), 0x416);
    table.zhe_canon.set(0x436);
    table.tick.set(8);
    assert_eq!(translation.translate(0x416), 0x436);
    assert!(translation.case_table().is_some());
}
