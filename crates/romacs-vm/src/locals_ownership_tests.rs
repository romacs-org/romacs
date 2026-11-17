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
