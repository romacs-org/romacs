//! `equal` and `sxhash-equal` on closures, against pinned GNU (#201): two
//! closures of lambdas with the same source, dialect, docstring and
//! interactive spec are `equal` when their captured values are, so a
//! re-evaluated `add-hook`, `remove-hook` or `add-to-list` of a lambda finds
//! the copy already there.
#[path = "support/gnu_lisp.rs"]
mod gnu_lisp;
#[path = "support/gnu_batch.rs"]
mod gnu_batch;
use gnu_lisp::gnu_lisp;

const CASES: &[&str] = &[
        "(progn (defvar my-h nil) (dotimes (_ 3) (add-hook 'my-h (lambda () (message \"hi\")))) (length my-h))",
        "(progn (defvar my-h2 nil) (add-hook 'my-h2 (lambda () 1)) (remove-hook 'my-h2 (lambda () 1)) my-h2)",
        "(progn (defvar ce-l nil) (add-to-list 'ce-l (lambda (x) (* x 2))) (add-to-list 'ce-l (lambda (x) (* x 2))) (length ce-l))",
        "(let ((h (make-hash-table :test 'equal))) (puthash (lambda (x) x) 'first h) (list (gethash (lambda (x) x) h) (hash-table-count h)))",
        "(list (equal (lambda (x) x) (lambda (x) x)) (equal (lambda (x) x) (lambda (y) y)) (equal (lambda (x) \"a\" x) (lambda (x) \"b\" x)) (equal (lambda (x) (interactive) x) (lambda (x) x)))",
        "(let ((mk (lambda (n) (lambda () n)))) (list (equal (funcall mk 1) (funcall mk 1)) (equal (funcall mk 1) (funcall mk 2)) (= (sxhash-equal (funcall mk 1)) (sxhash-equal (funcall mk 1)))))",
        "(list (equal (let ((a 1)) (lambda () a)) (let ((b 1)) (lambda () b))) (equal (let ((a 1)) (lambda () a)) (let ((a 1)) (lambda () a))))",
        "(equal (eval '(lambda (x) x) nil) (eval '(lambda (x) x) t))",
        "(equal (eval '(function (lambda (x) x)) t) (lambda (x) x))",
        "(progn (defvar ce-h nil) (let ((file (make-temp-file \"ce\" nil \".el\"))) (with-temp-file file (insert \"(add-hook (quote ce-h) (lambda () (message \\\"hi\\\")))\")) (load file nil t) (load file nil t) (delete-file file)) (length ce-h))",
        "(let ((a (lambda (x) (interactive \"p\") x)) (b (lambda (x) (interactive \"p\") x)) (c (lambda (x) (interactive \"P\") x))) (list (equal a b) (equal a c)))",
        "(list (equal (let ((a 1)) (lambda () (setq a (1+ a)))) (let ((a 1)) (lambda () (setq a (1+ a))))) (let ((f (let ((a 1)) (lambda () (setq a (1+ a)))))) (equal f (progn (funcall f) f))))",
        "(let* ((f (lambda (x) (interactive \"p\") x)) (g (lambda (x) (interactive \"p\") x)) (h (make-hash-table :test 'equal))) (puthash f 'f h) (list (equal f g) (progn (funcall g 1) (equal f g)) (gethash g h)))",
        "(list (equal (lambda (x) x) (make-interpreted-closure '(x) '(x) '(t))) (equal (make-interpreted-closure '() '(ce-x) '(ce-x t)) (make-interpreted-closure '() '(ce-x) '(t))))",
        "(let ((f (make-interpreted-closure nil '(1) '(t) nil '(interactive \"p\" a-mode))) (g (make-interpreted-closure nil '(1) '(t) nil '(interactive \"p\" a-mode)))) (list (equal f g) (progn (aset (aref g 5) 1 '(b-mode)) (equal f g))))",
        "(let* ((body (list 1)) (f (make-interpreted-closure nil (list 1) '(t))) (g (make-interpreted-closure nil body '(t)))) (list (equal f g) (progn (setcar body 2) (equal f g))))",
        "(let ((f (lambda () 1)) (g (make-interpreted-closure nil (list 1) '(t)))) (list (equal f g) (progn (setcar (aref g 1) 2) (equal f g))))",
        "(let* ((f (make-interpreted-closure (list 'x) '(1) '(t))) (args (list 'x 'y)) (g (make-interpreted-closure args '(1) '(t))) (h (make-hash-table :test 'equal))) (puthash f 'f h) (setcdr args nil) (list (equal f g) (= (sxhash-equal f) (sxhash-equal g)) (gethash g h)))",
        "(let* ((f (let ((x 2)) (lambda () x))) (env (list (cons 'x 1) t)) (g (make-interpreted-closure nil '(x) env)) (env2 (list (cons 'x 1) t)) (h (make-interpreted-closure nil '(x) env2))) (setcar env (cons 'x 2)) (list (equal f g) (equal g h) (progn (setcar env2 (cons 'x 2)) (equal g h))))",
        "(let* ((env (list (cons 'x 1) (cons 'y 2) t)) (f (make-interpreted-closure nil '(x) env)) (h (make-hash-table :test 'equal))) (puthash f 'f h) (setcdr env (list t)) (let ((g (make-interpreted-closure nil '(x) env))) (list (equal f g) (= (sxhash-equal f) (sxhash-equal g)) (gethash g h))))",
];

#[test]
fn closures_compare_as_pinned_gnu() {
    let expected = gnu_batch::outputs(&CASES.iter().map(|c| format!("(prin1 {c})")).collect::<Vec<_>>());
    let actual = std::thread::Builder::new().stack_size(256 << 20).spawn(|| {
        let mut vm = gnu_lisp(&[]);
        CASES.iter().map(|c| vm.eval_source(&format!("(prin1-to-string {c})")).and_then(romacs_vm::Local::display)
            .map_or_else(|e| format!("error: {e}"), |s| serde_json::from_str::<String>(&s).unwrap_or(s))).collect::<Vec<_>>()
    }).unwrap().join().unwrap();
    for ((case, expected), actual) in CASES.iter().zip(&expected).zip(&actual) {
        assert_eq!(actual, expected, "{case}");
    }
}
