//! Who a buffer's local variables write: the buffer, or every buffer.
//!
//! GNU keeps a buffer's locals in the buffer (`local_var_alist`), so making,
//! reading, setting, binding and killing them, and killing the buffer, write
//! that buffer alone. The test runs the same Lisp twice on old (collected)
//! state, once in a buffer that makes 200 locals and once in one that makes
//! none, and compares what each run wrote among the objects, conses and
//! symbol slots that were there before it.
use super::*;
use std::collections::BTreeSet;

/// What a run wrote of what was old: the object and cons indices and the
/// slot cards.
#[derive(Debug, Default, PartialEq, Eq)]
struct Written {
    objects: BTreeSet<usize>,
    conses: BTreeSet<usize>,
    cards: BTreeSet<usize>,
}

/// One run: 200 symbols are first made local in another buffer, so their
/// declaration is old; then, after a collection, `own-a` makes them local,
/// reads, sets and binds them, kills one, and is killed; or `own-b` does
/// the same with no locals. What the run wrote, and the killed buffer's
/// object index.
fn written(locals: bool) -> (Written, usize) {
    let mut vm = Vm::new();
    vm.eval_source_raw(&format!(r#"
      (setq own-syms (let (l) (dotimes (i 200) (push (intern (format "own-l%d" i)) l)) l)
            own-a (get-buffer-create "own-a") own-b (get-buffer-create "own-b")
            own-locals {})
      (dolist (s own-syms) (set-default s 0))
      (with-current-buffer (get-buffer-create "own-c")
        (dolist (s own-syms) (set (make-local-variable s) 0)))
      (setq own-run (lambda (buffer locals)
        (with-current-buffer buffer
          (dolist (s own-syms) (if locals (set (make-local-variable s) 1)))
          (dolist (s own-syms) (symbol-value s))
          (dolist (s own-syms) (set s 2))
          (let ((own-l0 3)) own-l0)
          (kill-local-variable 'own-l1))
        (kill-buffer buffer)))
      ;; Once with locals, in both runs: every path's lazy state (caches,
      ;; interned names, the per-buffer index) is made as the measured
      ;; run finds it.
      (funcall own-run (get-buffer-create "own-w") t)"#, if locals { "t" } else { "nil" })).unwrap();
    let named = vm.slot(if locals { "own-a" } else { "own-b" });
    let killed = vm.slots[named].value.index(0).unwrap() - 1;
    vm.collect(&mut []);
    assert!(vm.generations_settled());
    vm.eval_source_raw("(funcall own-run (if own-locals own-a own-b) own-locals)").unwrap();
    let cards = vm.slots.written().map(|range| range.start).collect();
    (Written { objects: vm.objects.dirty_old().collect(), conses: vm.conses.dirty_old().collect(), cards }, killed)
}

/// A buffer's locals are the buffer's: the run with 200 locals writes
/// nothing old that the run without them does not, but its own buffer.
#[test]
fn a_buffers_locals_write_only_the_buffer() {
    let (with, a) = written(true);
    let (without, b) = written(false);
    assert!(without.objects.contains(&b) && with.objects.contains(&a), "each run writes its own buffer");
    let extra_objects: BTreeSet<usize> = with.objects.difference(&without.objects).copied().filter(|&i| i != a).collect();
    let extra_cards: BTreeSet<usize> = with.cards.difference(&without.cards).copied().collect();
    assert_eq!(with.conses, without.conses, "locals write no old cons");
    assert!(extra_objects.is_empty(), "old objects only the run with locals wrote: {extra_objects:?}");
    assert!(extra_cards.is_empty(), "slot cards only the run with locals wrote: {extra_cards:?}");
}

/// A buffer's local of an obarray's own symbol keeps the symbol (the
/// collector frees an obarray symbol nothing reaches, and reuses its slot):
/// while the buffer lives the symbol does; once both are gone the slot is
/// free, and no buffer has a local of whatever symbol takes it next.
#[test]
fn a_local_keeps_its_obarray_symbol_and_a_reused_slot_has_none() {
    let mut vm = Vm::new();
    vm.eval_source_raw(r#"
      (setq rl-ob (obarray-make) rl-sym (intern "rl-local" rl-ob) rl-buf (get-buffer-create "rl"))
      (with-current-buffer rl-buf (set (make-local-variable rl-sym) 42))
      (setq rl-name (symbol-name rl-sym) rl-ob nil rl-sym nil)"#).unwrap();
    let named = vm.slot("rl-buf");
    let buffer = vm.slots[named].value.index(0).unwrap() - 1;
    let [held] = vm.local_slots(buffer)[..] else { panic!("one local") };
    vm.collect(&mut []);
    assert!(!vm.obarray_free_slots.contains(&held), "the buffer's local keeps its symbol");
    assert_eq!(vm.local_slots(buffer), [held]);
    let kept = vm.eval_source_raw(r#"(let ((l (buffer-local-variables rl-buf)) found)
      (while l (if (and (consp (car l)) (equal (symbol-name (car (car l))) "rl-local")) (setq found (cdr (car l)))) (setq l (cdr l)))
      found)"#).unwrap();
    assert_eq!(vm.display(kept).unwrap(), "42");
    vm.eval_source_raw("(kill-buffer rl-buf)").unwrap();
    vm.collect(&mut []);
    assert!(vm.obarray_free_slots.contains(&held), "with the buffer gone the symbol is freed");
    vm.eval_source_raw(r#"(setq rl-ob2 (obarray-make) rl-new (intern "rl-new" rl-ob2) rl-other (get-buffer-create "rl-other"))"#).unwrap();
    let new = vm.eval_source_raw("rl-new").unwrap().index(7).unwrap() - 1;
    assert_eq!(new, held, "the next obarray symbol takes the freed slot");
    let none = vm.eval_source_raw("(list (local-variable-p rl-new rl-other) (boundp rl-new))").unwrap();
    assert_eq!(vm.display(none).unwrap(), "(nil nil)");
}

/// A minor reads an old buffer again for its text, but its locals only
/// when something was stored into them since the last collection: every
/// store goes through `Vm::store_local`, which marks the buffer. So a
/// minor's work on locals does not grow with the old buffers that hold
/// them, and a young value stored into an old buffer's local, by a set or
/// by a new local, survives it.
#[test]
fn a_minor_reads_only_the_locals_stored_into_since() {
    let mut vm = Vm::new();
    vm.eval_source_raw(r#"
      (setq gc-cons-threshold 1000000000 mn-syms nil mn-i 0)
      (while (< mn-i 50) (setq mn-syms (cons (intern (format "mn-l%d" mn-i)) mn-syms) mn-i (1+ mn-i)))
      (setq mn-bufs nil mn-i 0)
      (while (< mn-i 20)
        (setq mn-bufs (cons (get-buffer-create (format "mn-%d" mn-i)) mn-bufs) mn-i (1+ mn-i))
        (save-current-buffer (set-buffer (car mn-bufs))
          (let ((l mn-syms)) (while l (set (make-local-variable (car l)) 0) (setq l (cdr l))))))"#).unwrap();
    vm.collect(&mut []);
    // Counted without the verifier, whose own walk reads every buffer whole.
    let before = crate::local_vars::visited();
    assert!(vm.collect_minor(&[]));
    assert_eq!(crate::local_vars::visited() - before, 0, "no old buffer's locals were stored into");
    let store = r#"
      (save-current-buffer (set-buffer (car mn-bufs))
        (setq mn-l0 (list 'fresh mn-i))
        (set (make-local-variable (intern (format "mn-new%d" mn-i))) (list 'made mn-i)))"#;
    vm.eval_source_raw(&format!("(setq mn-i 1) {store}")).unwrap();
    let before = crate::local_vars::visited();
    assert!(vm.collect_minor(&[]));
    assert_eq!(crate::local_vars::visited() - before, 51, "the one buffer stored into: its 50 locals and the new one");
    // The verifier checks the minor kept what the young stores named.
    vm.set_gc_verify(true);
    vm.eval_source_raw(&format!("(setq mn-i 2) {store}")).unwrap();
    assert!(vm.collect_minor(&[]));
    vm.eval_source_raw("(setq mn-i 0) (while (< mn-i 10000) (setq mn-junk (list mn-i mn-i)) (setq mn-i (1+ mn-i)))").unwrap();
    let kept = vm.eval_source_raw("(list (buffer-local-value 'mn-l0 (car mn-bufs)) (buffer-local-value 'mn-new1 (car mn-bufs)) (buffer-local-value 'mn-new2 (car mn-bufs)))").unwrap();
    assert_eq!(vm.display(kept).unwrap(), "((fresh 2) (made 1) (made 2))");
    // The writes that move a buffer's whole store mark the entries they
    // land in: `buffer-swap-text' moves each buffer's locals to the other's
    // object, a multibyte conversion replaces the buffer's object.
    vm.eval_source_raw(r#"
      (save-current-buffer (set-buffer (nth 1 mn-bufs)) (setq mn-l1 (list 'swapped)) (buffer-swap-text (nth 2 mn-bufs)))
      (save-current-buffer (set-buffer (nth 3 mn-bufs)) (setq mn-l2 (list 'converted)) (set-buffer-multibyte nil))"#).unwrap();
    assert!(vm.collect_minor(&[]));
    vm.eval_source_raw("(setq mn-i 0) (while (< mn-i 10000) (setq mn-junk (list mn-i mn-i)) (setq mn-i (1+ mn-i)))").unwrap();
    let moved = vm.eval_source_raw("(list (buffer-local-value 'mn-l1 (nth 1 mn-bufs)) (buffer-local-value 'mn-l2 (nth 3 mn-bufs)))").unwrap();
    assert_eq!(vm.display(moved).unwrap(), "((swapped) (converted))");
}
