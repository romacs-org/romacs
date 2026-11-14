//! Collector proofs for the closure -> interactive-spec table that stands in
//! for GNU's CLOSURE_INTERACTIVE slot (eval.c:604-612). The table is key-weak
//! and `eq`: a live closure keeps its spec and thunk (which may close over
//! large values), a dead closure frees both. Both dimensions, live objects AND
//! bytes, are measured across forced collections.
use super::*;

fn show(vm: &mut Vm, src: &str) -> String {
    match vm.eval_source_raw(src) {
        Ok(v) => vm.display(v).unwrap_or_else(|e| format!("DISPLAY-ERROR {e}")),
        Err(e) => format!("ERROR {e}"),
    }
}

fn table_entries(vm: &Vm) -> usize {
    let Some(&slot) = vm.names.get(crate::prim::INTERACTIVE_TABLE) else { return 0 };
    let table = vm.slots[slot].value;
    if table == Value::UNBOUND {
        return 0;
    }
    match vm.object_if(table) {
        Some(data::Object::HashTable(t)) => t.count(),
        _ => panic!("the interactive table is a hash table"),
    }
}

#[test]
fn dead_interactive_closures_release_their_specs() {
    let mut vm = Vm::new();
    show(&mut vm, "(setq cl-keep (lambda () (interactive (list 1)) 1))");
    let report = gc::assert_no_retention_growth(&mut vm, &[16, 64, 256], |vm, n| {
        let src = format!(
            "(let ((i 0)) (while (< i {n}) (let ((s (make-string 500 ?s))) \
               (setq cl-last (lambda () (interactive (list s)) i))) (setq i (1+ i))) \
               (setq cl-last nil) nil)"
        );
        assert_eq!(show(vm, &src), "nil");
    });
    eprintln!("interactive table retention: {report:?}, entries={}", table_entries(&vm));
    assert_eq!(table_entries(&vm), 1, "only the closure cl-keep still names is registered");
}

#[test]
fn a_live_interactive_closure_keeps_its_spec_across_compaction() {
    let mut vm = Vm::new();
    assert_eq!(
        show(&mut vm, "(progn (setq cl-junk (let ((i 0) (l nil)) (while (< i 2000) (setq l (cons (make-string 8 ?j) l) i (1+ i))) l)) \
                        (setq cl-cmd (let ((payload (make-string 4000 ?p))) (lambda (a) (interactive (list payload)) a))) nil)"),
        "nil"
    );
    vm.collect(&mut []);
    let with_junk = vm.heap_stats();
    assert_eq!(show(&mut vm, "(progn (setq cl-junk nil) nil)"), "nil");
    vm.collect(&mut []);
    let held = vm.heap_stats();
    assert!(held.live_objects < with_junk.live_objects, "the junk was reclaimed, so the closure and its spec moved");
    assert_eq!(
        show(&mut vm, "(let ((s (call-interactively cl-cmd))) (list (length s) (aref s 3999) (car (cdr (interactive-form cl-cmd)))))"),
        "(4000 112 (list payload))",
        "the only path to the payload is closure -> table entry -> thunk"
    );
    // Dropping the one Lisp reference releases the closure, its table entry
    // (key-weak), the spec, the thunk and the 4000-byte payload.
    assert_eq!(show(&mut vm, "(progn (setq cl-cmd nil) nil)"), "nil");
    vm.collect(&mut []);
    vm.collect(&mut []);
    let released = vm.heap_stats();
    let bytes = held.live_bytes - released.live_bytes;
    eprintln!(
        "interactive closure: held {} objects / {} bytes; released {} objects / {bytes} bytes",
        held.live_objects,
        held.live_bytes,
        held.live_objects - released.live_objects
    );
    assert!(released.live_objects < held.live_objects, "objects released with the closure");
    assert!(bytes >= 4000, "the payload was live only through the table: {bytes} bytes released");
    assert_eq!(table_entries(&vm), 0, "the dead closure's entry is gone");
}

/// A function alias chain is bounded by the symbols that can form it, not by
/// the cons arena, which a collection can leave empty.
#[test]
fn interactive_form_follows_aliases_with_an_empty_cons_arena() {
    let mut vm = Vm::new();
    // Every Lisp entry collects, so the reader's dead conses are gone when
    // the primitive walks the chain.
    vm.set_gc_stress(1);
    assert_eq!(show(&mut vm, "(interactive-form t)"), "nil");
    assert_eq!(show(&mut vm, "(interactive-form 'forward-char)"), "(interactive \"^p\")");
    assert!(vm.conses.len() <= 2, "the fixture empties the cons arena");
}

/// `all-completions` with `commandp` judges each candidate in its loop: a
/// walk over many symbols keeps one native frame, as GNU's does.
#[test]
fn commandp_walks_many_symbols_at_one_depth() {
    let mut vm = Vm::new();
    let count = show(&mut vm, "
      (let ((i 0))
        (while (< i 200000)
          (intern (concat \"walk-symbol-\" (number-to-string i)))
          (setq i (1+ i)))
        (fset 'walk-symbol-7 (lambda () (interactive) 7))
        (fset 'walk-symbol-99999 (lambda () (interactive) 9)))
      (length (all-completions \"walk-symbol-\" obarray 'commandp))");
    assert_eq!(count, "2");
}

/// The command set follows every write that changes a verdict: a definition,
/// an alias's target, `fmakunbound` and `unintern`.
#[test]
fn command_symbols_follow_definitions() {
    let mut vm = Vm::new();
    let members = "(let (found) (dolist (s '(set-a set-b set-c set-d)) (when (memq s (romacs-command-symbols)) (push s found))) (nreverse found))";
    show(&mut vm, "(progn (defalias 'set-a (lambda () (interactive) 1)) (defalias 'set-b 'set-c) (defalias 'set-c (lambda () 1)) (defalias 'set-d (lambda () (interactive) 4)))");
    assert_eq!(show(&mut vm, members), "(set-a set-d)");
    let epoch = show(&mut vm, "(romacs-command-epoch)");
    show(&mut vm, "(defalias 'set-c (lambda () (interactive) 3))");
    assert_eq!(show(&mut vm, members), "(set-a set-b set-c set-d)", "an alias follows its target");
    show(&mut vm, "(progn (fmakunbound 'set-a) (unintern \"set-d\" obarray))");
    assert_eq!(show(&mut vm, members), "(set-b set-c)");
    assert_ne!(show(&mut vm, "(romacs-command-epoch)"), epoch);
    let epoch = show(&mut vm, "(romacs-command-epoch)");
    show(&mut vm, "(put 'unrelated-symbol 'p 1)");
    assert_eq!(show(&mut vm, "(romacs-command-epoch)"), epoch, "a non-member's plist is no change");
    show(&mut vm, "(put 'set-b 'completion-predicate 'ignore)");
    assert_ne!(show(&mut vm, "(romacs-command-epoch)"), epoch, "a member's plist is");
}
