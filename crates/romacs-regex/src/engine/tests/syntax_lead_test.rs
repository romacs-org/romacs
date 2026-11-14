//! The syntax-aware first-character set: a pattern that can start with
//! `\sC` enters the matcher only at characters of that class, where the
//! table alone gives the classes.

use crate::engine::compile::regex_compile;
use crate::engine::dfa;
use crate::engine::matcher::route::matcher_entry_count;
use crate::engine::pattern::MatchRegisters;
use crate::engine::search::{build_search_optimizations, re_search, with_fastmap_disabled};
use crate::engine::syntax::{DefaultSyntaxLookup, SyntaxCacheKey, SyntaxLookup, default_char_has_category};
use crate::engine::syntax_lead::syntax_lead;
use crate::syntax_class::SyntaxClass;

fn lead_of(pattern: &str) -> Option<(Vec<u8>, u16)> {
    let compiled = regex_compile(pattern, false, false).expect("pattern compiles");
    syntax_lead(&compiled).map(|lead| lead.parts())
}

#[test]
fn syntax_leads() {
    let class = |c: SyntaxClass| 1u16 << (c as u16);
    for (pattern, expected) in [
        ("\\s|", Some((vec![], class(SyntaxClass::StringFence)))),
        ("[;:,]\\|\\(=\\|\\s(\\)", Some((b",:;=".to_vec(), class(SyntaxClass::Open)))),
        ("\\_<\\sw+x", Some((vec![], class(SyntaxClass::Word)))),
        ("\\=\\s-*#", Some((b"#".to_vec(), class(SyntaxClass::Whitespace)))),
        ("\\S-a", Some((vec![], !class(SyntaxClass::Whitespace)))),
        ("\\(?:\\sw\\|\\s_\\)+:", Some((vec![], class(SyntaxClass::Word) | class(SyntaxClass::Symbol)))),
        // No class leads, a nullable start, or anything can lead.
        ("abc", None),
        ("\\sw*", None),
        ("\\sw\\|.", None),
        ("\\sw\\|[^a]", None),
        ("\\sw\\|\\S-\\|\\s-", None),
        ("\\sw\\|[[:alpha:]]", None),
        ("\\sw\\|\\cg", None),
    ] {
        assert_eq!(lead_of(pattern), expected, "{pattern:?}");
    }
    let folded = regex_compile("\\s|", false, true).expect("pattern compiles");
    assert_eq!(syntax_lead(&folded), None);
}

/// The standard table with `syntax-table' properties at some positions, and
/// a frontier past which a lazy `syntax-propertize' has yet to look.
struct Propertized {
    properties: Vec<Option<SyntaxClass>>,
    read_limit: usize,
}
impl SyntaxLookup for Propertized {
    fn char_syntax(&self, c: char) -> SyntaxClass {
        crate::standard_syntax_class_for_char(c)
    }
    fn char_syntax_at(&self, c: char, pos: usize) -> SyntaxClass {
        self.properties.get(pos).copied().flatten().unwrap_or_else(|| self.char_syntax(c))
    }
    fn char_has_category(&self, c: char, cat: u8) -> bool {
        default_char_has_category(c, cat)
    }
    fn cache_key(&self) -> SyntaxCacheKey {
        SyntaxCacheKey::Standard
    }
    fn table_write_tick(&self) -> u64 {
        0
    }
    fn plain_syntax_until(&self, pos: usize) -> usize {
        (pos..self.properties.len()).find(|&at| self.properties[at].is_some()).unwrap_or(usize::MAX)
    }
    fn syntax_read_limit(&self) -> usize {
        self.read_limit
    }
}

type Found = Option<(usize, MatchRegisters)>;
fn norm(found: &Found) -> Option<(usize, Vec<i64>, Vec<i64>)> {
    found.as_ref().map(|(at, regs)| (*at, regs.start.to_vec(), regs.end.to_vec()))
}

/// A search for a string fence over text without one enters the matcher
/// only where a property could make one.  (The existence DFA's filter stays
/// off: entries count the scan.)
#[test]
fn syntax_led_search_enters_the_matcher_at_its_classes_only() {
    dfa::filter::with_dfa_mode(dfa::filter::DfaMode::Off, || {
        let text = "a comment of words, (some) punctuation; and no fence at all\n".repeat(100);
        let text = text.as_bytes();
        let compiled = regex_compile("\\s|", false, false).unwrap();
        let before = matcher_entry_count();
        let found = re_search(&compiled, text, 0, isize::try_from(text.len()).unwrap(), &DefaultSyntaxLookup, 0);
        assert!(found.is_none());
        let entries = matcher_entry_count() - before;
        assert!(entries <= 1, "{entries} matcher entries with no candidate");

        // Three propertied positions, one of them a fence: each is tried.
        let mut properties = vec![None; text.len()];
        properties[100] = Some(SyntaxClass::Word);
        properties[2000] = Some(SyntaxClass::StringFence);
        properties[3000] = Some(SyntaxClass::Punctuation);
        let syntax = Propertized { properties, read_limit: usize::MAX };
        let before = matcher_entry_count();
        let found = re_search(&compiled, text, 0, isize::try_from(text.len()).unwrap(), &syntax, 0);
        assert_eq!(found.map(|(at, _)| at), Some(2000));
        let entries = matcher_entry_count() - before;
        assert!(entries <= 2, "{entries} matcher entries for two propertied positions");
    });
}

/// Random syntax-led patterns over texts with multibyte characters,
/// properties and a propertize frontier, at every start and bound: the scan
/// equals the exhaustive oracle.
#[test]
fn syntax_lead_fuzz_agrees_with_the_exhaustive_oracle() {
    let mut seed = 0x51a7_1ead_u64;
    let mut next = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        usize::try_from(seed % n as u64).unwrap()
    };
    let leads = ["\\sw", "\\s-", "\\s(", "\\s|", "\\S-", "\\s.", "a", "[;(]", "\\=\\sw", "\\_<\\s_", "^\\s-", "\\(?:\\sw\\|\\s_\\)", "\u{e9}"];
    let rests = ["", "a", "+", "*;", "\\s-*#", "\\=", "\\(b\\|\u{3b1}\\)?"];
    let alphabet: Vec<&str> = vec!["a", "b", ";", "\n", "(", " ", "#", "_", "\u{e9}", "\u{3b1}"];
    let classes = [SyntaxClass::Word, SyntaxClass::Whitespace, SyntaxClass::StringFence, SyntaxClass::Open, SyntaxClass::Punctuation];
    let mut scanned = 0;
    for _ in 0..500 {
        let arms = 1 + next(3);
        let mut pattern = String::new();
        for arm in 0..arms {
            if arm > 0 {
                pattern.push_str("\\|");
            }
            pattern.push_str(leads[next(leads.len())]);
        }
        let pattern = format!("\\(?:{pattern}\\){}", rests[next(rests.len())]);
        let Ok(compiled) = regex_compile(&pattern, false, false) else { continue };
        build_search_optimizations(&compiled);
        if compiled.syntax_lead().is_some() {
            scanned += 1;
        }
        let text: String = (0..next(24)).map(|_| alphabet[next(alphabet.len())]).collect();
        let text = text.as_bytes();
        let properties = (0..text.len()).map(|_| (next(6) == 0).then(|| classes[next(classes.len())])).collect();
        let read_limit = if next(3) == 0 { next(text.len() + 1) } else { usize::MAX };
        let syntax = Propertized { properties, read_limit };
        let starts: Vec<usize> = (0..=text.len()).filter(|&at| at == text.len() || (text[at] & 0xC0) != 0x80).collect();
        for &start in &starts {
            for &stop in starts.iter().filter(|&&stop| stop >= start) {
                let point = starts[next(starts.len())];
                let range = isize::try_from(stop - start).unwrap();
                let got = re_search(&compiled, text, start, range, &syntax, point);
                let want = with_fastmap_disabled(|| re_search(&compiled, text, start, range, &syntax, point));
                assert_eq!(norm(&got), norm(&want), "{pattern:?} in {:?} from {start} to {stop}, point {point}",
                    String::from_utf8_lossy(text));
            }
        }
    }
    assert!(scanned > 150, "the generator must produce syntax-led patterns: {scanned}");
}
