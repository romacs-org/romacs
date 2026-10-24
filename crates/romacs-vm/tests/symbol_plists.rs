//! Symbol property lists belong to the symbol OBJECT, as in GNU
//! (`struct Lisp_Symbol` `u.s.plist`, src/lisp.h; `Fget`/`Fput`
//! src/fns.c:2653,2734; `Fsymbol_plist`/`Fsetplist` src/data.c:818,1029).
//!
//! Every expected value below is GNU Emacs 32.0.50's own answer, recorded by
//! `evidence/plist/plist-oracle.el` into `evidence/plist/gnu-transcript.txt`
//! (/usr/bin/emacs, sha256 20a94ee7...). Each case runs in a fresh VM.
use romacs_vm::Vm;

fn eval(form: &str) -> String {
    let mut vm = Vm::new();
    let wrapped = format!("(condition-case err {form} (error (list 'SIGNAL err)))");
    match vm.eval_source(&wrapped) {
        Ok(v) => v.display().unwrap(),
        Err(e) => format!("VM-ERROR {e}"),
    }
}

#[track_caller]
fn gnu(form: &str, expected: &str) {
    assert_eq!(eval(form), expected, "GNU answers {expected} for {form}");
}

#[test]
fn uninterned_symbols_carry_their_own_plist() {
    // The reported defect: VM2 keyed plists by NAME, so an uninterned symbol
    // from `make-symbol' signalled (error "symbol required").
    gnu(
        "(let ((a (make-symbol \"foo\")) (b (make-symbol \"foo\"))) (put a 'p 1) (put 'foo 'p 3) \
         (list (get a 'p) (get b 'p) (get 'foo 'p) (symbol-plist a) (symbol-plist b) (eq a b)))",
        "(1 nil 3 (p 1) nil nil)",
    );
    // pcase.el:978 and :1136, verbatim shape: a gensym marks itself used.
    gnu("(let ((s (make-symbol \"x\"))) (put s 'pcase-used t) (get s 'pcase-used))", "t");
}

#[test]
fn put_replaces_in_place_and_appends_at_the_end() {
    gnu("(let ((s (make-symbol \"s\"))) (put s 'a 1) (put s 'b 2) (put s 'a 3) (symbol-plist s))", "(a 3 b 2)");
    gnu(
        "(let* ((s (make-symbol \"s\")) (l (list 'x 1))) (setplist s l) (put s 'x 2) (put s 'y 3) (list l (eq (symbol-plist s) l)))",
        "((x 2 y 3) t)",
    );
}

#[test]
fn setplist_installs_the_exact_list_and_returns_it() {
    gnu(
        "(let ((s (make-symbol \"s\")) (l (list 'x 1))) (list (eq (setplist s l) l) (eq (symbol-plist s) l) (get s 'x)))",
        "(t t 1)",
    );
}

#[test]
fn malformed_plists_follow_gnu_get_and_put() {
    gnu("(let ((s (make-symbol \"s\"))) (setplist s 5) (list (get s 'a) (symbol-plist s)))", "(nil 5)");
    gnu("(let ((s (make-symbol \"s\"))) (setplist s 5) (put s 'a 1))", "(SIGNAL (wrong-type-argument plistp 5))");
    gnu("(let ((s (make-symbol \"s\"))) (setplist s (list 'a)) (list (get s 'a) (get s 'b)))", "(nil nil)");
    gnu("(let ((s (make-symbol \"s\"))) (setplist s (list 'a)) (put s 'b 1))", "(SIGNAL (wrong-type-argument plistp (a)))");
    gnu(
        "(let ((s (make-symbol \"s\"))) (setplist s (list 'a 1 'b)) (put s 'c 1))",
        "(SIGNAL (wrong-type-argument plistp (a 1 b)))",
    );
    // FOR_EACH_TAIL_SAFE: get stops quietly on a cycle; put signals.
    gnu(
        "(let ((s (make-symbol \"s\")) (l (list 'a 1))) (setcdr (cdr l) l) (setplist s l) (list (get s 'a) (get s 'z)))",
        "(1 nil)",
    );
    gnu(
        "(let ((s (make-symbol \"s\")) (l (list 'a 1))) (setcdr (cdr l) l) (setplist s l) \
         (condition-case e (put s 'z 1) (error (list (car e) (eq (car (cdr e)) l)))))",
        "(circular-list t)",
    );
}

#[test]
fn any_object_is_a_property_key_compared_with_eq() {
    gnu(
        "(let ((s (make-symbol \"s\")) (k \"k\")) (put s 1 'one) (put s k 'str) (list (get s 1) (get s \"k\") (get s k) (symbol-plist s)))",
        "(one nil str (1 one \"k\" str))",
    );
}

#[test]
fn non_symbols_signal_symbolp() {
    for form in ["(get 1 'a)", "(put 1 'a 2)", "(symbol-plist 1)", "(setplist 1 nil)"] {
        gnu(form, "(SIGNAL (wrong-type-argument symbolp 1))");
    }
}

#[test]
fn nil_t_and_keywords_have_plists() {
    gnu(
        "(progn (put nil 'plist-oracle-p 1) (put t 'plist-oracle-p 2) \
         (list (get nil 'plist-oracle-p) (get t 'plist-oracle-p) \
               (plist-get (symbol-plist nil) 'plist-oracle-p) (plist-get (symbol-plist t) 'plist-oracle-p)))",
        "(1 2 1 2)",
    );
    gnu("(progn (put :plist-oracle-kw 'p 1) (get :plist-oracle-kw 'p))", "1");
}

#[test]
fn buffer_local_machinery_is_not_on_the_lisp_plist() {
    // GNU keeps buffer-local bindings in the symbol's value redirect, never in
    // its plist: symbol-plist does not show them and setplist cannot undo them.
    gnu(
        "(progn (defvar plist-oracle-local 1) (make-variable-buffer-local 'plist-oracle-local) \
         (list (symbol-plist 'plist-oracle-local) \
               (progn (setplist 'plist-oracle-local nil) (local-variable-if-set-p 'plist-oracle-local))))",
        "(nil t)",
    );
    gnu(
        "(progn (defvar plist-oracle-l2 1) (with-temp-buffer (make-local-variable 'plist-oracle-l2) (setq plist-oracle-l2 5) \
         (list (symbol-plist 'plist-oracle-l2) (progn (setplist 'plist-oracle-l2 '(q 1)) plist-oracle-l2) \
               (local-variable-if-set-p 'plist-oracle-l2))))",
        "(nil 5 t)",
    );
}
