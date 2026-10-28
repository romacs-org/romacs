//! buffer.c:4033 overlay-lists: (OVERLAYS), every overlay of the current
//! buffer whatever the narrowing, by start and among equal starts newest
//! first. Expected answers are pinned GNU 32.0.50's.
use romacs_vm::Vm;

#[test]
fn overlay_lists_returns_every_overlay_in_interval_tree_order() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let source = "(progn (set-buffer (get-buffer-create \"ol\")) (insert \"abcdefgh\")
      (let ((os (list (make-overlay 3 5) (make-overlay 1 4) (make-overlay 1 2) (make-overlay 3 3) (make-overlay 1 6) (make-overlay 1 2))))
        (overlay-put (nth 2 os) 'n 'first12) (overlay-put (nth 5 os) 'n 'second12)
        (narrow-to-region 2 3)
        (list (mapcar (lambda (o) (list (overlay-start o) (overlay-end o) (overlay-get o 'n))) (car (overlay-lists)))
              (cdr (overlay-lists)))))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(),
               "(((1 2 second12) (1 6 nil) (1 2 first12) (1 4 nil) (3 3 nil) (3 5 nil)) nil)");
}
