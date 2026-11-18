//! Compiled queries, shared: one `ts_query_new` per grammar and query text
//! for the VM, where a major mode compiles the same queries for every
//! buffer it sets up (c-ts-mode's font-lock settings probe each optional
//! keyword with `treesit-query-valid-p`). A failure is kept as well, so a
//! probe that fails fails once.
//!
//! Lisp sees nothing of it: each `treesit-query-compile` still makes a
//! query object of its own, with its own source, which holds the shared
//! `Compiled`, and a failure's signal is made anew from the caller's
//! source. The key is the text `ts_query_new` reads, predicates and all,
//! so two sources that expand alike share. The memo is the VM's
//! (`Vm::treesit_queries`), as the compiled queries are its alone: a
//! worker VM compiles its own. A query keeps its text (`Compiled::text`),
//! so `romacs--treesit-query-combine` joins its parts' texts without
//! expanding them again, and its joined text is found here in turn.
use super::{Compiled, native};
use std::collections::HashMap;
use std::rc::Rc;

/// What compiling gives: the shared query, or `ts_query_new`'s error offset
/// and GNU's description of its type.
pub(crate) type Outcome = Result<Rc<Compiled>, (u32, &'static str)>;

/// A loaded grammar, as the memo tells them apart: its `TSLanguage`'s
/// address, which no other grammar can take for the session since no
/// grammar library is closed, and the ABI it was built for. A library
/// loaded again from another file is another mapping, and so another
/// address: its queries are its own.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Grammar {
    address: usize,
    abi: u32,
}

impl Grammar {
    fn of(language: &native::Language) -> Self {
        Self { address: language.address(), abi: language.abi_version() }
    }
}

/// The most outcomes kept; past it, the least recently used goes. A mode
/// compiles a few dozen; queries made afresh by a program are what the
/// bound is for.
const CAPACITY: usize = 512;

struct Entry {
    outcome: Outcome,
    /// The memo's clock at the entry's last use.
    used: u64,
}

/// The VM's compiled queries by grammar and text.
#[derive(Default)]
pub(crate) struct Memo {
    entries: HashMap<Grammar, HashMap<Box<[u8]>, Entry>>,
    len: usize,
    clock: u64,
}

impl Memo {
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// SOURCE compiled for LANGUAGE, once per grammar and text.
    pub(crate) fn compile(&mut self, language: &native::Language, source: &[u8]) -> Outcome {
        let grammar = Grammar::of(language);
        if let Some(outcome) = self.lookup(grammar, source) {
            return outcome;
        }
        let outcome = native::Query::new(language, source).map(|query| Rc::new(Compiled::new(query, source)));
        self.insert(grammar, source, outcome.clone());
        outcome
    }
    fn lookup(&mut self, grammar: Grammar, source: &[u8]) -> Option<Outcome> {
        self.clock += 1;
        let entry = self.entries.get_mut(&grammar)?.get_mut(source)?;
        entry.used = self.clock;
        Some(entry.outcome.clone())
    }
    fn insert(&mut self, grammar: Grammar, source: &[u8], outcome: Outcome) {
        if self.len >= CAPACITY {
            self.evict();
        }
        self.clock += 1;
        let entry = Entry { outcome, used: self.clock };
        if self.entries.entry(grammar).or_default().insert(source.into(), entry).is_none() {
            self.len += 1;
        }
    }
    /// Drop the least recently used outcome.
    fn evict(&mut self) {
        let oldest = self.entries.iter()
            .flat_map(|(&grammar, sources)| sources.iter().map(move |(source, e)| (e.used, grammar, source)))
            .min_by_key(|&(used, ..)| used)
            .map(|(_, grammar, source)| (grammar, source.clone()));
        let Some((grammar, source)) = oldest else { return };
        let sources = self.entries.get_mut(&grammar).expect("the oldest entry's grammar");
        sources.remove(&source);
        if sources.is_empty() {
            self.entries.remove(&grammar);
        }
        self.len -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::{CAPACITY, Grammar, Memo, native};
    use std::rc::Rc;

    /// The system C grammar (the one `tests/treesit.rs` needs), loaded from
    /// a copy of its own at PATH, or from the system's when None.
    fn c_grammar(copy: Option<&std::path::Path>) -> native::Language {
        let system = std::path::Path::new("/usr/lib/libtree-sitter-c.so");
        let path = copy.map_or(system.to_path_buf(), |to| {
            std::fs::copy(system, to).expect("a copy of the system C grammar");
            to.to_path_buf()
        });
        let library = native::open_library(path.as_os_str().as_encoded_bytes()).expect("the C grammar opens");
        native::language_symbol(library, b"tree_sitter_c").expect("tree_sitter_c").0
    }

    #[test]
    fn one_grammar_shares_and_a_second_load_does_not() {
        let dir = std::env::temp_dir().join(format!("query-memo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = c_grammar(None);
        let again = c_grammar(None);
        let other = c_grammar(Some(&dir.join("libtree-sitter-c-copy.so")));
        assert_eq!(Grammar::of(&first), Grammar::of(&again), "one library opened twice is one grammar");
        assert_ne!(Grammar::of(&first), Grammar::of(&other), "a copy under another path is loaded apart");
        let text = b"(number_literal) @n";
        let mut memo = Memo::default();
        let (a, b, c) = (memo.compile(&first, text).unwrap(), memo.compile(&again, text).unwrap(), memo.compile(&other, text).unwrap());
        assert!(Rc::ptr_eq(&a, &b), "one grammar and text compile once");
        assert!(!Rc::ptr_eq(&a, &c), "another load's queries are its own");
        assert_eq!(c.query.pattern_count(), 1);
        // A failure is kept, and given again as it was.
        assert_eq!(memo.compile(&first, b"(bogus) @n").err(), Some((1, "Node type error at")));
        assert_eq!(memo.compile(&first, b"(bogus) @n").err(), Some((1, "Node type error at")));
        assert_eq!(memo.len, 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_memo_keeps_its_bound_dropping_the_least_recently_used() {
        let grammar = Grammar { address: 1, abi: 15 };
        let mut memo = Memo::default();
        for i in 0..CAPACITY {
            memo.insert(grammar, format!("q{i}").as_bytes(), Err((0, "x")));
        }
        assert!(memo.lookup(grammar, b"q0").is_some());
        memo.insert(grammar, b"new", Err((0, "x")));
        assert_eq!(memo.len, CAPACITY);
        assert!(memo.lookup(grammar, b"q0").is_some(), "a recent use stays");
        assert!(memo.lookup(grammar, b"q1").is_none(), "the least recently used goes");
        assert!(memo.lookup(grammar, b"new").is_some());
    }
}
