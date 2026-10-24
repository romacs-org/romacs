// Expected values probed with GNU Emacs 32.0.50, source revision
// 45b0f7699df36b3849edc3bdef37d4cb9e75b9ea (fns.c:2667).
// The destroyed-pair test is a candidate safety check, not a GNU oracle case.
use romacs_vm::Vm;

fn check(form: &str, expected: &str) {
    let mut vm = Vm::new();
    let wrapped = format!("(condition-case err {form} (error (list 'SIGNAL err)))");
    let result = vm.eval_source(&wrapped).unwrap().display().unwrap();
    assert_eq!(result, expected, "{form}");
}

#[test]
fn plist_put_mule_conf_property_identity() {
    check("(let* ((p (list :docstring \"old\")) (tail (cdr p))) (list (eq p (plist-put p :docstring \"new\")) (eq tail (cdr p)) (eq p (plist-put p :name 'ascii)) p))", "(t t t (:docstring \"new\" :name ascii))");
    check("(funcall 'plist-put nil :docstring \"ASCII\")", "(:docstring \"ASCII\")");
}

#[test]
fn plist_put_malformed_and_circular() {
    check("(plist-put 5 'a 2)", "(SIGNAL (wrong-type-argument plistp 5))");
    check("(plist-put (list 'a) 'a 2)", "(SIGNAL (wrong-type-argument plistp (a)))");
    check("(plist-put (cons 'a (cons 1 'tail)) 'b 2)", "(SIGNAL (wrong-type-argument plistp (a 1 . tail)))");
    check("(plist-put (cons 'a (cons 1 'tail)) 'a 2)", "(a 2 . tail)");
    check("(let ((p (list 'a 1))) (setcdr (cdr p) p) (condition-case e (plist-put p 'b 2) (error (list (car e) (eq (car (cdr e)) p)))))", "(circular-list t)");
}

#[test]
fn plist_put_callback_cannot_panic_on_destroyed_pair() {
    // GNU uses unchecked XCDR here; VM2 must safely refuse the destroyed pair.
    check("(let ((p (list 'a 1))) (plist-put p 'b 2 (lambda (old new) (setcdr p nil) nil)))", "(SIGNAL (wrong-type-argument plistp (a)))");
    check("(let ((p (list 'a 1))) (plist-put p 'b 2 (lambda (old new) (setcdr p 7) nil)))", "(SIGNAL (wrong-type-argument plistp (a . 7)))");
}

#[test]
fn plist_put_predicate_state_survives_collection() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let result = vm.eval_source("(let* ((key (vector 4)) (val (vector 8)) (p (list 'a 1 key 2)) (tail (cdr p))) (list (eq p (plist-put p key val (lambda (old new) (let ((n 0)) (while (< n 30000) (vector n) (setq n (1+ n)))) (eq old new)))) (eq tail (cdr p)) (eq (car (cdr (cdr (cdr p)))) val)))").unwrap();
    assert_eq!(result.display().unwrap(), "(t t t)");
    assert!(vm.heap_stats().collections > 0);
}

#[test]
fn plist_put_eq_and_optional_predicate() {
    check("(let* ((a (list 'x)) (b (list 'x)) (p (list a 1))) (plist-put p b 2) (list (length p) (eq (car p) a) (eq (car (cdr (cdr p))) b)))", "(4 t t)");
    check("(let* ((a (list 'x)) (b (list 'x)) (p (list a 1))) (list (eq p (plist-put p b 2 'equal)) p))", "(t ((x) 2))");
    check("(plist-put nil 'a 1 'not-a-function)", "(a 1)");
    check("(let ((seen nil) (p (list 'a 1 'b 2))) (plist-put p 'b 9 (lambda (old new) (setq seen (cons (list old new) seen)) (eq old new))) (list p seen))", "((a 1 b 9) ((b b) (a b)))");
    check("(let ((p (list 'a 1))) (plist-put p 'b 2 (lambda (old new) (setcdr p (list 7)) t)) p)", "(a 2)");
}
