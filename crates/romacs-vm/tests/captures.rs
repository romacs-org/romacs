use romacs_vm::Vm;

#[test]
fn large_single_functions_keep_sequential_and_wide_captures_correct() {
    for n in [1000, 2000, 4000, 8000] {
        let body: String = (0..n)
            .map(|i| format!("(let ((a{i} {i})) (setq s (+ s (funcall (lambda () a{i})))))"))
            .collect();
        let mut vm = Vm::new();
        let entry = vm
            .compile_source(&format!("(let ((s 0)) {body} s)"))
            .unwrap();
        assert_eq!(
            vm.execute(entry).unwrap().as_integer().unwrap(),
            n * (n - 1) / 2
        );
    }
    let n = 1000;
    let bindings: String = (0..n).map(|i| format!("(a{i} {i})")).collect();
    let references = (0..n)
        .map(|i| format!("a{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut vm = Vm::new();
    let entry = vm
        .compile_source(&format!(
            "(let ({bindings}) (funcall (lambda () (+ {references}))))"
        ))
        .unwrap();
    assert_eq!(
        vm.execute(entry).unwrap().as_integer().unwrap(),
        n * (n - 1) / 2
    );
}

#[test]
fn binder_resolved_mutability_preserves_aliasing_and_scope() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    for (source, expected) in [
        (
            "(let ((x 0)) (let ((writer (lambda (n) (setq x n))) (reader (lambda () x))) (setq x 3) (funcall writer 7) (list (funcall reader) (let ((x 99)) (funcall reader)))))",
            "(7 7)",
        ),
        (
            "(defun factory (x) (cons (lambda () (setq x (1+ x))) (lambda () x))) (let ((a (factory 1)) (b (factory 10))) (funcall (car a)) (funcall (car b)) (funcall (car b)) (list (funcall (cdr a)) (funcall (cdr b))))",
            "(2 12)",
        ),
        (
            "(let ((x 1)) (let ((mk (lambda () (lambda () (setq x (1+ x))))) (read (lambda () x))) (let ((w (funcall mk))) (funcall w) (funcall w) (funcall read))))",
            "3",
        ),
        (
            "(defun maker (&optional x &rest r) (cons (lambda () (setq x (1+ x)) (setcar r x)) (lambda () (list x r)))) (let ((p (maker 3 9 10))) (funcall (car p)) (funcall (cdr p)))",
            "(4 (4 10))",
        ),
        (
            "(let ((x 1)) (let ((f (lambda () (let ((x 2)) (setq x 3) x))) (g (lambda () x))) (list (funcall f) (funcall g))))",
            "(3 1)",
        ),
        (
            "(let ((a 7)) (let ((f (lambda () (lambda () a)))) (setq a 9) (funcall (funcall f))))",
            "9",
        ),
        (
            "(defmacro write-hidden () '(setq hidden (1+ hidden))) (let ((hidden 0)) (let ((a (lambda () (write-hidden))) (b (lambda () (write-hidden)))) (funcall a) (funcall b)))",
            "2",
        ),
    ] {
        let value = vm
            .eval_source(source)
            .unwrap_or_else(|e| panic!("{source}: {e}"));
        assert_eq!(value.display().unwrap(), expected, "{source}");
    }
}
