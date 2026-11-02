//! Per-item costs the scaling probes found over growing collections
//! (harness/scaling/probes.el), each measured by the evaluating thread's
//! instruction count (`romacs-instructions') at two sizes. A machine
//! without the counter fails these tests: they never pass unmeasured.
use romacs_vm::Vm;

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| panic!("{source}: {e}"))
}

/// Instructions FORM costs in VM, after SETUP.
fn cost(vm: &mut Vm, setup: &str, form: &str) -> f64 {
    eval(vm, setup);
    let text = eval(vm, &format!("(let ((a (romacs-instructions))) {form} (and a (- (romacs-instructions) a)))"));
    text.parse().unwrap_or_else(|_| panic!("no instruction counter (romacs-instructions gave {text})"))
}

/// `featurep' is Fmemq over `features': a walk, not a copy of the list
/// per call. Copying cost ~75 instructions an element; the walk ~33.
#[test]
fn featurep_walks_features_without_copying_them() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let provide = |n: usize| format!("(dotimes (i {n}) (provide (intern (format \"sf-%d\" i))))");
    let probe = "(dotimes (_ 100) (featurep 'scaling-absent))";
    let small = cost(&mut vm, &provide(2000), probe);
    let large = cost(&mut vm, &provide(10_000), probe);
    let per_element = (large - small) / 100.0 / 8000.0;
    assert!(per_element < 45.0, "featurep costs {per_element:.1} instructions per feature");
    assert_eq!(eval(&mut vm, "(list (featurep 'sf-0) (featurep 'sf-9999) (featurep 'scaling-absent))"), "(t t nil)");
    assert_eq!(eval(&mut vm, "(let ((features (list 'a 'b))) (setcdr (cdr features) features) (condition-case e (featurep 'c) (error 'circular)))"),
        "circular");
}

/// `equal' of conses compares them structurally without first asking
/// whether either is a string; a cons is never equal to a string.
#[test]
fn equal_of_conses_and_strings_keeps_its_answers() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    assert_eq!(eval(&mut vm, "(list (equal '(a \"b\" (1 . 2)) (list 'a \"b\" (cons 1 2))) (equal '(a) \"a\") (equal \"a\" '(a)) (equal \"ab\" (concat \"a\" \"b\")) (equal '(a b) '(a c)) (member (list 'x 2) '((x 1) (x 2))))"),
        "(t nil nil t nil ((x 2)))");
}

/// kill-buffer finds the indirect buffers of the buffer it kills from the
/// base's own record, not by walking every live buffer: a kill costs the
/// same beside 200 buffers as beside 1,000. A base's indirect buffers
/// still die first, in buffer-list order.
#[test]
fn kill_buffer_does_not_walk_every_buffer() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let kill = "(dotimes (i 50) (kill-buffer (format \"sk-%d\" i)))";
    let make = |n: usize| format!("(dotimes (i {n}) (get-buffer-create (format \"sb-%d-{n}\" i))) (dotimes (i 50) (get-buffer-create (format \"sk-%d\" i)))");
    let small = cost(&mut vm, &make(150), kill);
    let large = cost(&mut vm, &make(800), kill);
    assert!(large < small * 2.0, "50 kills beside {} buffers cost {large}, beside {} {small}", 1050, 200);
    assert_eq!(eval(&mut vm, "(let* ((base (get-buffer-create \"sk-base\")) (one (make-indirect-buffer base \"sk-one\")) (two (make-indirect-buffer base \"sk-two\")) (seen nil))
                               (with-current-buffer one (set (make-local-variable 'kill-buffer-hook) (list (lambda () (setq seen (cons 'one seen))))))
                               (with-current-buffer two (set (make-local-variable 'kill-buffer-hook) (list (lambda () (setq seen (cons 'two seen))))))
                               (list (mapcar #'buffer-name (romacs--indirect-buffers base)) (kill-buffer base) (mapcar #'buffer-live-p (list base one two)) (nreverse seen)))"),
        "((\"sk-one\" \"sk-two\") t (nil nil nil) (one two))");
}
