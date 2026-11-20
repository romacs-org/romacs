//! `CompiledPattern::reads_lookup`: which patterns may consult the
//! `SyntaxLookup` a search is given, so a host may pass the constant
//! `DefaultSyntaxLookup` in place of the one it would prepare.

use crate::engine::compile::regex_compile;
use crate::engine::search::re_search;
use crate::engine::syntax::{DefaultSyntaxLookup, SyntaxCacheKey, SyntaxLookup};
use crate::syntax_class::SyntaxClass;

/// The standard table inverted: a word constituent is punctuation, anything
/// else a word constituent, and each category flipped. Whatever reads it
/// answers otherwise than the standard, on any character.
struct Hostile;
impl SyntaxLookup for Hostile {
    fn char_syntax(&self, c: char) -> SyntaxClass {
        if DefaultSyntaxLookup.char_syntax(c) == SyntaxClass::Word { SyntaxClass::Punctuation } else { SyntaxClass::Word }
    }
    fn char_has_category(&self, c: char, cat: u8) -> bool {
        !DefaultSyntaxLookup.char_has_category(c, cat)
    }
    fn cache_key(&self) -> SyntaxCacheKey {
        SyntaxCacheKey::External { id: 1, epoch: 0 }
    }
    fn table_write_tick(&self) -> u64 {
        0
    }
    fn position_dependent(&self) -> bool {
        false
    }
}

/// A table under which every character is a word constituent with every
/// category: it moves word boundaries, which the inverted table keeps.
struct AllWord;
impl SyntaxLookup for AllWord {
    fn char_syntax(&self, _c: char) -> SyntaxClass {
        SyntaxClass::Word
    }
    fn char_has_category(&self, _c: char, _cat: u8) -> bool {
        true
    }
    fn cache_key(&self) -> SyntaxCacheKey {
        SyntaxCacheKey::External { id: 2, epoch: 0 }
    }
    fn table_write_tick(&self) -> u64 {
        0
    }
    fn position_dependent(&self) -> bool {
        false
    }
}

const READING: &[&str] = &[
    "\\bx", "\\Bx", "\\<x", "x\\>", "\\_<x", "x\\_>", "\\w", "\\W", "\\s-", "\\S-", "[[:word:]]", "[[:space:]]",
    "[[:punct:]]", "\\cg", "\\Cg",
];
const PLAIN: &[&str] = &[
    "\\`/:", "\\.gpg\\(~\\|\\.~[0-9]+~\\)?\\'", "\\.\\(?:gz\\|bz2\\|xz\\)\\'", "\\`/[^/|:][^/|]*:", "abc", "a.*c",
    "[a-z]+", "^foo$", "\\(x\\)\\1", "x\\{2,3\\}", "[[:alnum:]]+", "[[:digit:]]", "[[:upper:]]",
    "\\.~[-[:alnum:]:#@^._]+\\(?:~[[:digit:]]+\\)?~\\'",
];
const SUBJECTS: &[&str] = &["/tmp/ffc-none", "/: x", "a-b c", "file.gpg", "foo", "xx yy", "abc abc", "", "/ssh:host:"];

fn first(pattern: &str, subject: &str, lookup: &dyn SyntaxLookup) -> Option<usize> {
    let compiled = regex_compile(pattern, false, false).expect("pattern compiles");
    re_search(&compiled, subject.as_bytes(), 0, subject.len() as isize, lookup, usize::MAX).map(|(at, _)| at)
}

#[test]
fn each_construct_that_reads_the_table_says_so() {
    for pattern in READING {
        assert!(regex_compile(pattern, false, false).unwrap().reads_lookup(), "{pattern}");
    }
    for pattern in PLAIN {
        assert!(!regex_compile(pattern, false, false).unwrap().reads_lookup(), "{pattern}");
    }
}

#[test]
fn a_pattern_that_reads_no_table_matches_alike_under_any() {
    for pattern in PLAIN {
        for subject in SUBJECTS.iter().chain(&["\u{e9}", "\u{3042}", "\u{2014}", "9z", "X"]) {
            assert_eq!(first(pattern, subject, &Hostile), first(pattern, subject, &DefaultSyntaxLookup), "{pattern} on {subject:?}");
            assert_eq!(first(pattern, subject, &AllWord), first(pattern, subject, &DefaultSyntaxLookup), "{pattern} on {subject:?}");
        }
    }
}

#[test]
fn the_reading_patterns_do_see_the_table() {
    // Each reading construct answers differently under one of the two
    // tables on some subject, so the flag is what keeps the table for them.
    for pattern in READING {
        assert!(
            SUBJECTS.iter().chain(&["-", " ", "a", "\u{e9}", "\u{3042}", "\u{2014}"]).any(|subject| {
                let standard = first(pattern, subject, &DefaultSyntaxLookup);
                first(pattern, subject, &Hostile) != standard || first(pattern, subject, &AllWord) != standard
            }),
            "{pattern}"
        );
    }
}
