use romacs_vm::Vm;

fn check(source: &str, expected: &str) {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let expression = format!("(let ((actual {source})) (if (equal actual '{expected}) t (list 'got actual)))");
    assert_eq!(vm.eval_source(&expression).unwrap_or_else(|error| panic!("{source}: {error}")).display().unwrap(), "t", "{source}");
}

#[test]
fn documentation_does_not_expose_ordinary_closures() {
    for (source, expected) in [
        ("(let ((n 0)) (let ((f (lambda () (:documentation (progn (setq n (1+ n)) 'kind)) 7))) (garbage-collect) (list n (romacs--oclosure-type f) (funcall f) (condition-case e (length f) (wrong-type-argument (car (cdr e)))) (condition-case e (aref f 4) (wrong-type-argument (car (cdr e)))))))", "(1 kind 7 sequencep arrayp)"),
        ("(list (romacs--oclosure-type (lambda () nil)) (romacs--oclosure-type (lambda () (:documentation 42) nil)) (romacs--oclosure-type (lambda () \"doc\" nil)) (romacs--oclosure-type 'car) (romacs--oclosure-type [1]))", "(nil nil nil nil nil)"),
        ("(let ((f (make-interpreted-closure nil '(7) '(t) 'before nil))) (list (romacs--oclosure-type f) (let ((g (make-interpreted-closure (aref f 0) (aref f 1) (aref f 2) 'after nil))) (garbage-collect) (romacs--oclosure-type g)) (funcall f)))", "(before after 7)"),
    ] { check(source, expected); }
}

#[test]
fn explicit_marker_exposes_ordered_shared_cells() {
    check("(progn (fset 'ignore (lambda (&rest _) nil)) (fset 'oclosure--fix-type (lambda (_ f) f)) (let ((outside 9)) (let ((m 3) (a 2) (z 1)) (let* ((f (oclosure--fix-type (ignore z a m) (lambda () (:documentation 'kind) (+ z a m outside)))) (env (aref f 2))) (setcdr (car env) 20) (garbage-collect) (list (car (car env)) (car (car (cdr env))) (car (car (cdr (cdr env)))) (aref f 4) (funcall f))))))", "(z a m kind 34)");
    check("(progn (fset 'ignore (lambda (&rest _) nil)) (fset 'oclosure--fix-type (lambda (_ f) f)) (let ((outside 9)) (let ((value 1)) (let* ((f (oclosure--fix-type (ignore) (lambda () (:documentation 'kind) (+ value outside)))) (g (make-interpreted-closure (aref f 0) (aref f 1) (cons (cons 'value 7) (cdr (aref f 2))) (aref f 4) nil))) (setq outside 20) (garbage-collect) (list (funcall f) (funcall g))))))", "(21 27)");
}

#[test]
fn explicit_marker_preserves_resolution_and_effects() {
    check("(progn (fset 'ignore (lambda (&rest _) nil)) (fset 'oclosure--fix-type (lambda (_ f) f)) (eq (oclosure--fix-type (ignore) (function car)) 'car))", "t");
    // Compiled GNU (bytecode.c Bcall) evaluates the arguments, then reads the
    // function cell: a redefinition inside an argument is the function called.
    check("(progn (fset 'ignore (lambda (&rest _) nil)) (fset 'oclosure--fix-type (lambda (_ f) (list 'old f))) (let ((result (oclosure--fix-type (ignore) (lambda () (:documentation (progn (fset 'ignore (lambda (&rest _) nil)) (fset 'oclosure--fix-type (lambda (_ f) (list 'new f))) 'kind)) 7)))) (list (car result) (romacs--oclosure-type (car (cdr result))) (funcall (car (cdr result))))))", "(new kind 7)");
    check("(progn (fset 'ignore (lambda (&rest _) nil)) (fset 'oclosure--fix-type (lambda (_ f) 'sentinel)) (oclosure--fix-type (ignore) (lambda () (:documentation 'kind) 7)))", "sentinel");
}

#[test]
fn explicit_interactive_marker_preserves_slots_and_spec() {
    check("(progn (fset 'ignore (lambda (&rest _) nil)) (fset 'oclosure--fix-type (lambda (_ f) f)) (let ((value 7)) (let* ((f (oclosure--fix-type (ignore value) (lambda (n) (:documentation 'kind) (interactive \"p\") n))) (env (aref f 2))) (garbage-collect) (list (aref f 4) (car (car env)) (cdr (car env)) (funcall f 12) (interactive-form f)))))", "(kind value 7 12 (interactive \"p\"))");
}

fn load_gnu_generic(vm: &mut Vm) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let paths = ["gnu-lisp", "gnu-lisp/emacs-lisp", "gnu-lisp/progmodes", "gnu-lisp/international", "gnu-lisp/textmodes", "rlisp"]
        .map(|path| format!("{:?}", root.join(path).to_str().unwrap())).join(" ");
    vm.eval_source(&format!("(setq load-path '({paths}))")).unwrap();
    for file in [
        "rlisp/vm/prelude.el", "gnu-lisp/emacs-lisp/debug-early.el",
        "gnu-lisp/emacs-lisp/byte-run.el", "gnu-lisp/emacs-lisp/backquote.el",
        "gnu-lisp/subr.el", "gnu-lisp/keymap.el", "gnu-lisp/emacs-lisp/pcase.el",
        "gnu-lisp/emacs-lisp/gv.el", "gnu-lisp/emacs-lisp/inline.el",
        "gnu-lisp/bindings.el", "gnu-lisp/emacs-lisp/easy-mmode.el",
        "gnu-lisp/emacs-lisp/cl-lib.el", "gnu-lisp/button.el",
        "gnu-lisp/emacs-lisp/derived.el", "gnu-lisp/obarray.el",
        "gnu-lisp/emacs-lisp/rx.el", "gnu-lisp/abbrev.el",
        "gnu-lisp/emacs-lisp/cl-preloaded.el",
    ] {
        vm.load_file(&root.join(file)).unwrap_or_else(|error| panic!("{file}: {error}"));
    }
    vm.eval_source("(require 'romacs-cl-generic)").unwrap();
}

#[test]
fn whole_gnu_oclosure_and_generic_dispatch() {
    let mut vm = Vm::new();
    load_gnu_generic(&mut vm);
    assert_eq!(vm.eval_source("(featurep 'oclosure)").unwrap().display().unwrap(), "t");
    let source = "(oclosure-define (compat-object (:copier compat-copy)) z a (m :mutable t))
        (setq compat-instance (let ((outside 9)) (oclosure-lambda (compat-object (a 2) (m 3) (z 1)) () (+ z a m outside))))
        (list (compat-object--z compat-instance) (compat-object--a compat-instance) (compat-object--m compat-instance) (funcall compat-instance))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(1 2 3 15)");
    assert_eq!(vm.eval_source("(setf (compat-object--m compat-instance) 7) (list (funcall compat-instance) (funcall (compat-copy compat-instance :z 20)))").unwrap().display().unwrap(), "(19 38)");
    vm.eval_source("(require 'romacs-cl-generic)").unwrap();
    assert_eq!(vm.eval_source("(featurep 'cl-generic)").unwrap().display().unwrap(), "t");
    let source = "(cl-defgeneric compat-dispatch (x))
        (cl-defmethod compat-dispatch ((x t)) 'ordinary)
        (cl-defmethod compat-dispatch ((x integer)) (list 'integer (cl-call-next-method)))
        (list (compat-dispatch 3) (compat-dispatch 'symbol) (compat-dispatch (lambda () nil)))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "((integer ordinary) ordinary ordinary)");

    let source = "(oclosure-define (compat-child (:parent compat-object) (:copier compat-child-copy)) extra)
        (setq compat-child-instance (oclosure-lambda (compat-child (extra 4) (m 3) (a 2) (z 1)) (n) (+ z a m extra n)))
        (setq compat-child-duplicate (compat-child-copy compat-child-instance :z 20))
        (setf (compat-object--m compat-child-instance) 10)
        (garbage-collect)
        (list (compat-child--extra compat-child-instance) (funcall compat-child-instance 5) (funcall compat-child-duplicate 5))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(4 22 34)");

    let source = "(oclosure-define compat-shadow x)
        (list (let ((f (oclosure-lambda (compat-shadow (x 7)) (x) x))) (funcall f 99))
              (let ((f (oclosure-lambda (compat-shadow (x 8)) () (let ((x 20)) (setq x 21))))) (funcall f)))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(99 21)");

    let source = "(let* ((a (make-symbol \"same\")) (b (make-symbol \"same\"))
            (f (eval `(oclosure--lambda 'compat-anonymous ((,a 2) (,b 3)) nil () (list ,a ,b)) t))
            (env (aref f 2)))
            (garbage-collect) (list (eq (car (car env)) a) (eq (car (car (cdr env))) b) (funcall f)))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(t t (2 3))");

    let source = "(let ((z-out 20) (a-out 10))
        (let ((factory (lambda (n) (oclosure-lambda (compat-object (z n) (a 2) (m 3)) () (list z a m z-out a-out)))))
            (let* ((f (funcall factory 1)) (g (compat-copy f :z 7)))
                (setq z-out 40) (garbage-collect) (list (funcall f) (funcall g)))))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "((1 2 3 40 10) (7 2 3 40 10))");

    let source = "(cl-defgeneric compat-kind (x))
        (cl-defmethod compat-kind ((x interpreted-function)) '(function))
        (cl-defmethod compat-kind ((x oclosure)) (cons 'oclosure (cl-call-next-method)))
        (cl-defmethod compat-kind ((x compat-object)) (cons 'object (cl-call-next-method)))
        (cl-defmethod compat-kind ((x compat-child)) (cons 'child (cl-call-next-method)))
        (list (compat-kind (lambda () nil)) (compat-kind compat-instance) (compat-kind compat-child-instance))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "((function) (object oclosure function) (child object oclosure function))");

    let source = "(cl-defmethod compat-dispatch ((x integer)) (list 'replacement (cl-call-next-method)))
        (cl-defmethod compat-dispatch ((x (eql 7))) (list 'seven (cl-call-next-method)))
        (garbage-collect)
        (list (compat-dispatch 3) (compat-dispatch 7) (compat-dispatch 'symbol))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "((replacement ordinary) (seven (replacement ordinary)) ordinary)");

    let source = "(defvar compat-events nil)
        (cl-defgeneric compat-order (x))
        (cl-defmethod compat-order ((x t)) (push 'primary compat-events) x)
        (cl-defmethod compat-order :before ((x t)) (push 'before compat-events))
        (cl-defmethod compat-order :after ((x t)) (push 'after compat-events))
        (cl-defmethod compat-order :around ((x t)) (push 'enter compat-events) (prog1 (cl-call-next-method) (push 'leave compat-events)))
        (list (compat-order 4) (nreverse compat-events))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(4 (enter before primary after leave))");

    let source = "(cl-defgeneric compat-next (x))
        (cl-defmethod compat-next ((x t)) (cl-next-method-p))
        (cl-defmethod compat-next ((x integer)) (list (cl-next-method-p) (cl-call-next-method)))
        (list (compat-next 7) (compat-next 'symbol))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "((t nil) nil)");
}

#[test]
fn precise_types_preserve_legacy_type_of() {
    check("(let ((xs (list nil t 's 7 1.5 '(a) [a] \"s\" (symbol-function 'car) (lambda () nil) (make-char-table nil) (record 'kind 1))) out) (while xs (setq out (cons (list (type-of (car xs)) (cl-type-of (car xs))) out)) (setq xs (cdr xs))) (nreverse out))", "((symbol null) (symbol boolean) (symbol symbol) (integer fixnum) (float float) (cons cons) (vector vector) (string string) (subr primitive-function) (interpreted-function interpreted-function) (char-table char-table) (kind kind))");
    check("(list purify-flag (special-variable-p 'purify-flag) (let ((purify-flag t)) (garbage-collect) purify-flag) purify-flag)", "(nil t t nil)");
}

#[test]
fn lexical_dispatcher_assertion_does_not_change_binding_mode() {
    check("(list (let ((lexical-binding t)) lexical-binding) (let* ((lexical-binding t) (x lexical-binding)) x))", "(t t)");
    // Binding it nil does not select dynamic binding, here as in GNU's
    // already-lexical code: the closure still captures x.
    check("(let ((lexical-binding nil)) (let ((x 1)) (funcall (let ((x 2)) (lambda () x)))))", "2");
    check("(let ((lexical-binding (progn t))) 7)", "7");
}
