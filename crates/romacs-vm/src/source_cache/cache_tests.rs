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

/// Whether V, a Value the snapshot fixture holds, is live: verify mode
/// poisons a freed cons and frees an object's entry, and reuses neither.
/// `nil`, `t`, fixnums and symbols are never freed.
fn live(vm: &Vm, v: Value) -> bool {
    match v.tag() {
        _ if v == Value::NIL || v == Value::TRUE => true,
        1 | 7 => true,
        3 => vm.cons_pair(v).is_ok_and(|pair| pair != (crate::gc::POISON, crate::gc::POISON)),
        0 => vm.object(v).is_ok(),
        tag => panic!("the fixture holds no Value of tag {tag}: {v:?}"),
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

/// However many weak rounds a mark takes, each live entry is traced once:
/// a chain through a key-weak table, whose next key each entry's value
/// holds, makes a round per link, and a list the chain's last value holds
/// is found live only in the last of them.
#[test]
fn a_live_entry_is_traced_once_per_collection() {
    let mut vm = Vm::new();
    vm.eval_source(
        "(progn (defvar cache-lists (list (list 'lambda '() 1) (list 'lambda '() 2) (list 'lambda '() 3)))
                (funcall (car cache-lists)) (funcall (car (cdr cache-lists))) (funcall (car (cdr (cdr cache-lists))))
                (defvar cache-chain (make-hash-table :weakness 'key :test 'eq))
                (defvar cache-chain-head (list 'link))
                (let ((key cache-chain-head))
                  (dotimes (_ 4) (let ((next (list 'link))) (puthash key (list next) cache-chain) (setq key next)))
                  (let ((late (list 'lambda '() 4))) (funcall late) (puthash key (list late) cache-chain))))",
    )
    .unwrap();
    vm.collect_garbage();
    let (rounds, traced) = vm.gc_stress.last_mark;
    assert!(rounds > 5, "the chain must force a round per link: {rounds}");
    assert_eq!(traced, vm.lambda_lists.len(), "each live entry traced once, in {rounds} rounds");
    let late = vm
        .eval_source_raw(
            "(let ((key cache-chain-head)) (dotimes (_ 4) (setq key (car (gethash key cache-chain)))) (car (gethash key cache-chain)))",
        )
        .unwrap();
    assert!(vm.lambda_lists.contains_key(&late.0), "the last round's list was traced, so kept");
}
