//! The compiled-pattern cache across `modify-syntax-entry`: a pattern whose
//! compilation read no table keeps its compilation; one whose compilation
//! read a table's content, a syntax table's (`used_syntax`) or a case
//! table's through a completion translation, is compiled again, against the
//! tables as they are now.
use super::{PatternId, Translation};
use crate::Vm;

/// The pattern the cache holds for SOURCE: its id, which a compile of it
/// afresh does not share.
fn compiled(vm: &Vm, source: &str) -> Option<PatternId> {
    let cache = &vm.mutator_rest.regexp_cache;
    let slot = *cache.index.get(source.as_bytes())?.first()?;
    cache.entries[slot].as_ref().map(|e| PatternId { slot, serial: e.serial })
}

fn same(a: Option<&PatternId>, b: &PatternId) -> bool {
    a == Some(b)
}

/// A fresh syntax table inheriting the standard one, as `make-syntax-table` (Lisp) makes it.
const NEW_TABLE: &str = "(let ((table (make-char-table 'syntax-table))) (set-char-table-parent table (standard-syntax-table)) table)";

fn eval(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source_raw(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    vm.display(value).unwrap()
}

#[test]
fn an_unrelated_modify_syntax_entry_keeps_patterns_that_read_no_syntax_table() {
    let mut vm = Vm::new();
    eval(&mut vm, r#"(progn (string-match "foo[0-9]+" "x foo12") (string-match "\\sw+" "ab") (string-match "[[:word:]]+x" "abx"))"#);
    let plain = compiled(&vm, "foo[0-9]+").expect("compiled and cached");
    let syntax_at_match = compiled(&vm, "\\sw+").expect("compiled and cached");
    assert!(compiled(&vm, "[[:word:]]+x").is_some(), "the word class is cached under its syntax table");

    eval(&mut vm, &format!(r#"(modify-syntax-entry ?_ "w" {NEW_TABLE})"#));

    assert!(same(compiled(&vm, "foo[0-9]+").as_ref(), &plain), "a pattern that read no syntax table keeps its compilation");
    assert!(same(compiled(&vm, "\\sw+").as_ref(), &syntax_at_match), "`\\sw' reads the table only while it matches");
    assert!(compiled(&vm, "[[:word:]]+x").is_none(), "a pattern whose fastmap read the table is dropped");
    assert_eq!(eval(&mut vm, r#"(string-match "foo[0-9]+" "x foo12")"#), "2");
}

#[test]
fn kept_and_recompiled_patterns_match_by_the_table_as_modified() {
    let mut vm = Vm::new();
    let ends = r#"(list (progn (string-match "\\sw+" "a_b") (match-end 0)) (progn (string-match "[[:word:]]+" "a_b") (match-end 0)))"#;
    eval(&mut vm, &format!("(set-syntax-table {NEW_TABLE})"));
    assert_eq!(eval(&mut vm, ends), "(1 1)", "`_' is symbol syntax");
    let kept = compiled(&vm, "\\sw+").expect("compiled and cached");

    eval(&mut vm, r#"(modify-syntax-entry ?_ "w")"#);

    assert_eq!(eval(&mut vm, ends), "(3 3)", "`_' is word syntax now, for both");
    assert!(same(compiled(&vm, "\\sw+").as_ref(), &kept), "the match-time pattern was not compiled again");
}

/// A completion translation folds by the canon table's effective ranges, its
/// parents' included, and a syntax table may be one of those parents: an edit
/// to that syntax table changes how completion folds case.
#[test]
fn a_syntax_edit_under_the_case_canon_recompiles_completion_patterns() {
    let mut vm = Vm::new();
    eval(&mut vm, r"(progn
  (setq probe-base (make-char-table 'case-table))
  (aset probe-base ?A ?a)
  (setq probe-syntax (make-char-table 'syntax-table))
  (set-char-table-parent probe-syntax probe-base)
  (aset probe-syntax ?A '(2))
  (setq probe-canon (make-char-table 'case-table))
  (set-char-table-parent probe-canon probe-syntax)
  (setq probe-case (copy-sequence (standard-case-table)))
  (set-char-table-extra-slot probe-case 1 probe-canon)
  (set-case-table probe-case))");
    let complete = r#"(let ((completion-ignore-case t) (completion-regexp-list '("A"))) (all-completions "" '("a")))"#;
    let before = eval(&mut vm, complete);
    eval(&mut vm, r#"(modify-syntax-entry ?A "@" probe-syntax)"#);
    let after = eval(&mut vm, complete);
    // What a pattern compiled afresh against the tables as they are now answers.
    vm.mutator_rest.regexp_cache.retain(|_| false);
    let fresh = eval(&mut vm, complete);
    assert_ne!(before, fresh, "the edit changes how completion folds ?A");
    assert_eq!(after, fresh, "the kept completion pattern answers as a fresh one");
}

/// A matcher keeps its patterns' ids across its tests; a pattern that left
/// the cache meanwhile, its slot now another pattern's, is compiled again,
/// never read from the slot.
#[test]
fn a_matcher_compiles_again_a_pattern_that_left_the_cache() {
    let mut vm = Vm::new();
    let pattern = vm.eval_source_raw(r#""foo[0-9]+""#).unwrap();
    let mut matcher = vm.string_matcher(&[pattern], Translation::None, None).ok().flatten().expect("a matcher, with no syntax properties");
    let hit = vm.eval_source_raw(r#""x foo12""#).unwrap();
    assert_eq!(vm.string_matcher_matches(&mut matcher, hit).ok(), Some(true));
    vm.mutator_rest.regexp_cache.retain(|_| false);
    eval(&mut vm, r#"(string-match "bar" "bar")"#);
    let hit = vm.eval_source_raw(r#""x foo12""#).unwrap();
    assert_eq!(vm.string_matcher_matches(&mut matcher, hit).ok(), Some(true), "the pattern compiled again");
    let miss = vm.eval_source_raw(r#""bar""#).unwrap();
    assert_eq!(vm.string_matcher_matches(&mut matcher, miss).ok(), Some(false), "not the slot's new pattern");
}
