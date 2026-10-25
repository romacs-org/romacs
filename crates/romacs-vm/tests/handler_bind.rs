use romacs_vm::Vm;

#[test]
fn uncaught_macro_errors_retain_their_native_diagnostic() {
    for (body, expected) in [
        ("(missing-handler-macro-function)", "(void-function missing-handler-macro-function)"),
        ("missing-handler-macro-variable", "(void-variable missing-handler-macro-variable)"),
    ] {
        let mut vm = Vm::new();
        vm.eval_source(&format!("(defmacro failing-handler-macro () {body})")).unwrap();
        let error = vm.eval_source("(failing-handler-macro)").map(drop).unwrap_err();
        assert_eq!(error, expected);
        assert_eq!(vm.eval_source("(+ 17 25)").unwrap().display().unwrap(), "42");
    }
}

fn cases() -> Vec<(&'static str, &'static str)> {
    vec![
        ("normal", "(handler-bind-1 (lambda () 42))"),
        ("nil_conditions", "(handler-bind-1 (lambda () 42) nil 17)"),
        ("trailing_clause", "(condition-case e (handler-bind-1 (lambda () 42) 'error) (error e))"),
        ("wrong_body", "(condition-case e (handler-bind-1 17 'error (lambda (e) (throw 'done e))) (error (car e)))"),
        ("callback_before_cleanup", "(let (log) (condition-case e (handler-bind-1 (lambda () (unwind-protect (error \"body\") (setq log (cons 'cleanup log)))) 'error (lambda (e) (setq log (cons (list 'handler (cdr e)) log)))) (error (setq log (cons 'caught log)))) (nreverse log))"),
        ("inner_condition_barrier", "(let (log) (handler-bind-1 (lambda () (condition-case e (error \"inner\") (error (setq log (cons 'inner-catch log))))) 'error (lambda (e) (setq log (cons 'bad-handler log)))) (nreverse log))"),
        ("ordered_returning_handlers", "(let (log) (condition-case e (handler-bind-1 (lambda () (error \"body\")) 'error (lambda (e) (setq log (cons 'first log))) '(error t) (lambda (e) (setq log (cons 'second log)))) (error (setq log (cons 'caught log)))) (nreverse log))"),
        ("inner_handler_failure", "(let (log) (condition-case e (handler-bind-1 (lambda () (handler-bind-1 (lambda () (error \"body\")) 'error (lambda (e) (setq log (cons 'inner-first log)) (error \"handler\")) 'error (lambda (e) (setq log (cons 'bad-sibling log))))) 'error (lambda (e) (setq log (cons (list 'outer (cdr e)) log)))) (error (setq log (cons (list 'caught (cdr e)) log)))) (nreverse log))"),
        ("masked_body_condition", "(let (log) (condition-case e (handler-bind-1 (lambda () (handler-bind-1 (lambda () (condition-case e (signal 'arith-error '(body)) (wrong-type-argument (setq log (cons 'bad-body-case log))))) 'arith-error (lambda (e) (setq log (cons 'inner log)) (car 1)) 'wrong-type-argument (lambda (e) (setq log (cons 'bad-sibling log))))) 'error (lambda (e) (setq log (cons 'outer log)))) (error (setq log (cons 'caught log)))) (nreverse log))"),
        ("new_callback_condition", "(let (log) (condition-case e (handler-bind-1 (lambda () (error \"body\")) 'error (lambda (e) (setq log (cons 'first log)) (condition-case e (error \"callback\") (error (setq log (cons 'inner-catch log))))) 'error (lambda (e) (setq log (cons 'second log)))) (error (setq log (cons 'caught log)))) (nreverse log))"),
        ("nonlocal_callback", "(let ((cleanup nil)) (list (catch 'done (handler-bind-1 (lambda () (unwind-protect (error \"body\") (setq cleanup t))) t (lambda (e) (throw 'done (car e))))) cleanup))"),
        ("shared_error_object", "(let (seen same) (condition-case e (handler-bind-1 (lambda () (error \"identity\")) 'error (lambda (e) (setq seen e)) 'error (lambda (e) (setq same (eq seen e)))) (error (list same (eq seen e)))))"),
        ("body_does_not_resume", "(let ((continued nil)) (condition-case e (handler-bind-1 (lambda () (car 1) (setq continued t)) 'error (lambda (e) 'ignored)) (error (list (car e) continued))))"),
        ("quit_matches_t", "(catch 'done (handler-bind-1 (lambda () (signal 'quit nil)) t (lambda (e) (throw 'done (car e)))))"),
        ("eval_visibility", "(progn (defvar hbi-mark 'outside) (let (log) (condition-case e (handler-bind-1 (lambda () (let ((hbi-mark 'inside)) (unwind-protect (eval '(error \"child\")) (setq log (cons 'cleanup log))))) 'error (lambda (e) (setq log (cons (list 'handler hbi-mark) log)))) (error (setq log (cons 'caught log)))) (nreverse log)))"),
        ("eval_condition_barrier", "(let (log) (handler-bind-1 (lambda () (eval '(condition-case e (error \"child\") (error 'caught)))) 'error (lambda (e) (setq log (cons 'bad-handler log)))) log)"),
        ("dynamic_cleanup", "(progn (defvar hbi-mark 'outside) (let (seen) (list (catch 'done (handler-bind-1 (lambda () (let ((hbi-mark 'inside)) (error \"stop\"))) 'error (lambda (e) (setq seen hbi-mark) (throw 'done 'escaped)))) seen hbi-mark)))"),
        ("replaced_conditions", "(let (log) (condition-case e (handler-bind-1 (lambda () (signal 'arith-error '(body))) 'arith-error (lambda (e) (setq log (cons 'first log)) (put 'arith-error 'error-conditions '(changed error))) 'arith-error (lambda (e) (setq log (cons 'second log)))) (arith-error (setq log (cons 'caught-arith log))) (error (setq log (cons 'caught-error log)))) (nreverse log))"),
        ("captured_conditions_gc", "(let (log) (condition-case e (handler-bind-1 (lambda () (signal 'arith-error '(body))) 'arith-error (lambda (e) (put 'arith-error 'error-conditions '(changed error)) (setq log (cons (consp (garbage-collect)) log))) 'arith-error (lambda (e) (setq log (cons 'second log)))) (arith-error (setq log (cons 'caught-arith log))) (error (setq log (cons 'caught-error log)))) (nreverse log))"),
        ("first_callback_observes_search_projection", "(let (log) (condition-case e (handler-bind-1 (lambda () (signal 'arith-error '(body))) 'arith-error (lambda (e) (setq log (cons 'first log)) (setcar (get 'arith-error 'error-conditions) 'changed)) 'arith-error (lambda (e) (setq log (cons 'bad-second log)))) (arith-error (setq log (cons 'bad-catch log))) (error (setq log (cons 'caught-error log)))) (nreverse log))"),
        ("mutated_conditions_list", "(let ((original (get 'arith-error 'error-conditions)) log) (condition-case e (handler-bind-1 (lambda () (signal 'arith-error '(body))) 'arith-error (lambda (e) (setq log (cons 'first log)) (put 'arith-error 'error-conditions '(other error)) (garbage-collect) (setcar original 'changed)) 'arith-error (lambda (e) (setq log (cons 'bad-second log)))) (arith-error (setq log (cons 'bad-catch log))) (error (setq log (cons 'caught-error log)))) (nreverse log))"),
        ("callable_alias", "(let ((function (symbol-function 'handler-bind-1))) (funcall function (lambda () 71)))"),
        ("hierarchy", "(progn (define-error 'hbi-parent \"Parent\") (define-error 'hbi-child \"Child\" 'hbi-parent) (catch 'done (handler-bind-1 (lambda () (signal 'hbi-child '(value))) '(hbi-parent) (lambda (e) (throw 'done e)))))"),
    ]
}

#[test]
fn non_unwinding_handlers_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    for (name, source) in cases() {
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")])
            .output().unwrap();
        assert!(output.status.success(), "{name}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        let actual = vm.eval_source(source).and_then(|value| value.display());
        assert_eq!(actual.as_deref(), Ok(expected.as_str()), "{name}: {source}");
    }
}

#[test]
fn callback_collection_forwards_pending_error_and_suspended_frames() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let source = "(let ((payload (list 17)) (collected nil) (same nil))
      (list (catch 'done
        (handler-bind-1 (lambda () (signal 'error (list payload))) 'error
          (lambda (error)
            (let ((i 0)) (while (< i 12000) (cons i i) (setq i (1+ i))))
            (setq collected (consp (garbage-collect)) same (eq (car (cdr error)) payload))
            (throw 'done (car (cdr error)))))) collected same payload))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "((17) t t (17))");
    assert!(vm.heap_stats().collections > 0);
    assert!(vm.heap_stats().reclaimed > 0);
}

#[test]
fn callback_reserve_is_restored_after_nonlocal_exit() {
    let mut vm = Vm::new();
    let source = "(let ((max-lisp-eval-depth 100) (lisp-eval-depth-reserve 200) (seen nil))
      (vector (catch 'done
        (handler-bind-1 (lambda () (error \"stop\")) 'error
          (lambda (e)
            (setq seen (and (> max-lisp-eval-depth 100)
                            (= (+ max-lisp-eval-depth lisp-eval-depth-reserve) 300)))
            (setq max-lisp-eval-depth (+ max-lisp-eval-depth 5))
            (throw 'done 'escaped))))
        seen max-lisp-eval-depth lisp-eval-depth-reserve))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "[escaped t 100 205]");
    assert_eq!(vm.eval_source("(handler-bind-1 (lambda () 42))").unwrap().display().unwrap(), "42");
}

#[test]
fn load_errors_reach_outer_handlers_before_load_cleanup() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let path = std::env::temp_dir().join(format!("handler-bind-load-{}.el", std::process::id()));
    std::fs::write(&path, "(let ((hbi-mark 'loaded)) (unwind-protect (signal 'arith-error '(from-load)) (setq hbi-log (cons 'cleanup hbi-log))))").unwrap();
    let source = format!("(progn (defvar hbi-mark 'outside) (defvar hbi-log nil)
      (condition-case e (handler-bind-1 (lambda () (load {:?} nil t t)) 'arith-error
        (lambda (e) (setq hbi-log (cons (list hbi-mark e) hbi-log))))
        (arith-error (setq hbi-log (cons 'caught hbi-log)))) (nreverse hbi-log))", path.to_str().unwrap());
    let output = std::process::Command::new(&binary)
        .args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    let actual = vm.eval_source(&source).and_then(|value| value.display());
    std::fs::remove_file(path).unwrap();
    assert_eq!(actual.as_deref(), Ok(expected.as_str()));
}
