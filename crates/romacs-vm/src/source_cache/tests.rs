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

/// Every Value ENTRY's snapshot names, read from the nodes themselves.
fn snapshot_values(entry: &LambdaList) -> Vec<Value> {
    entry.snapshot.iter().flat_map(|node| match node {
        LambdaNode::Cons(node, car, cdr) => vec![*node, *car, *cdr],
        LambdaNode::Text(node, ..) => vec![*node],
        LambdaNode::Vector(node, items) => std::iter::once(*node).chain(items.iter().copied()).collect(),
    }).collect()
}

/// Whether V is a live object: verify mode poisons a freed cons and frees
/// an object's entry, and reuses neither.
fn live(vm: &Vm, v: Value) -> bool {
    match v.tag() {
        3 => vm.cons_pair(v).is_ok_and(|pair| pair != (crate::gc::POISON, crate::gc::POISON)),
        0 if v != Value::NIL => vm.object(v).is_ok(),
        _ => true,
    }
}

/// An old child only the snapshot still reaches lives through a
/// collection, in place or moving: no new object takes its index and
/// passes for it, and the next call sees the change and compiles again.
#[test]
fn a_snapshot_keeps_the_children_only_it_names() {
    for moving in [false, true] {
        let mut vm = Vm::new();
        vm.set_gc_verify(true);
        vm.set_gc_moving(moving);
        vm.eval_source(
            "(progn (defvar cache-list (list 'lambda '() (list 'car (list 'quote (list 'old 'child)))))
                    (funcall cache-list)
                    (setcar (cdr (cdr cache-list)) (list 'quote 'new)))",
        )
        .unwrap();
        let compiles = vm.source_cache_stats().lambda_compiles;
        vm.collect_garbage();
        let list = variable(&mut vm, "cache-list");
        let values = snapshot_values(&vm.lambda_lists[&list.0]);
        let dead: Vec<Value> = values.into_iter().filter(|&v| !live(&vm, v)).collect();
        assert!(dead.is_empty(), "the snapshot names freed objects {dead:?} (moving: {moving})");
        assert_eq!(vm.eval_source("(funcall cache-list)").unwrap().display().unwrap(), "new", "moving: {moving}");
        assert_eq!(vm.source_cache_stats().lambda_compiles, compiles + 1, "changed, so compiled again (moving: {moving})");
    }
}
