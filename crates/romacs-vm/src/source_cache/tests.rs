//! A collection keeps a `(lambda ...)` list's compiled closure
//! (`lambda_lists`) while the list lives, moved with what it holds when the
//! heap moves. The mode-line segments' forms: `display_tests.rs`.
use super::*;

/// The value of the variable NAME.
fn variable(vm: &mut Vm, name: &str) -> Value {
    let slot = vm.slot(name);
    vm.slots[slot].value
}

/// A list's entry goes with the list, and only then.
#[test]
fn a_lambda_list_entry_dies_with_its_list() {
    let mut vm = Vm::new();
    vm.eval_source("(progn (defvar cache-list (list 'lambda '() 1)) (funcall cache-list))").unwrap();
    let list = variable(&mut vm, "cache-list");
    assert!(vm.lambda_lists.contains_key(&list.0), "called, so cached");
    vm.collect_garbage();
    // Read again: a collection may move it.
    let list = variable(&mut vm, "cache-list");
    assert!(vm.lambda_lists.contains_key(&list.0), "the list lives, so its entry does");
    let entries = vm.lambda_lists.len();
    vm.eval_source("(setq cache-list nil)").unwrap();
    vm.collect_garbage();
    assert_eq!(vm.lambda_lists.len(), entries - 1, "the list died, so its entry did");
}

/// A moving collection rewrites the key and what the entry holds: the
/// closure still runs, and nothing compiles or expands again.
#[test]
fn a_moved_lambda_list_keeps_its_closure() {
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
    vm.eval_source(
        "(progn (defvar cache-expansions 0)
                (defmacro cache-counted () (setq cache-expansions (1+ cache-expansions)) 10)
                (defvar cache-list nil)
                (let ((garbage (make-list 2000 (copy-sequence \"g\"))))
                  (setq cache-list (list 'lambda '(x) (list '+ 'x (list 'cache-counted) (length garbage))))))",
    )
    .unwrap();
    let call = |vm: &mut Vm| vm.eval_source("(list (funcall cache-list 1) cache-expansions)").unwrap().display().unwrap();
    assert_eq!(call(&mut vm), "(2011 1)");
    let before = (variable(&mut vm, "cache-list"), vm.source_cache_stats().lambda_compiles);
    vm.collect_garbage();
    let list = variable(&mut vm, "cache-list");
    assert_ne!(list, before.0, "the fixture must move the list");
    assert!(vm.lambda_lists.contains_key(&list.0), "kept under the moved key");
    assert_eq!(call(&mut vm), "(2011 1)");
    assert_eq!(vm.source_cache_stats().lambda_compiles, before.1, "compiled once");
}
