//! run-hook-wrapped against pinned GNU (eval.c): WRAP-FUNCTION gets each
//! hook function and ARGS, the first non-nil value stops the walk, local
//! `t' runs the global functions, a whole-function value is wrapped once,
//! and a void or nil hook runs nothing.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(let ((log nil)) (setq romacs-rhw-a (list (lambda (x) (push (list 'a x) log) nil) (lambda (x) (push (list 'b x) log) nil))) (list (run-hook-wrapped 'romacs-rhw-a (lambda (f &rest args) (push (list 'wrap args) log) (apply f args) nil) 7) (nreverse log)))
(let ((log nil)) (setq romacs-rhw-b (list (lambda () 1) (lambda () (push 'unreached log) 2))) (list (run-hook-wrapped 'romacs-rhw-b (lambda (f) (funcall f))) (run-hook-wrapped 'romacs-rhw-b (lambda (f) nil)) log))
(progn (setq romacs-rhw-c (lambda (x y) (list 'whole x y))) (list (run-hook-wrapped 'romacs-rhw-c (lambda (f &rest a) (apply f a)) 1 2) (run-hook-wrapped 'romacs-rhw-void (lambda (f) t)) (progn (setq romacs-rhw-nil nil) (run-hook-wrapped 'romacs-rhw-nil (lambda (f) t))) (run-hook-wrapped 'romacs-rhw-c (lambda (f a b) (list (eq f romacs-rhw-c) a b)) 'p 'q)))
(with-temp-buffer (let ((log nil)) (set-default 'romacs-rhw-d (list (lambda () (push 'global log) nil))) (set (make-local-variable 'romacs-rhw-d) (list (lambda () (push 'local log) nil) t (lambda () (push 'after log) nil))) (list (run-hook-wrapped 'romacs-rhw-d (lambda (f) (funcall f))) (nreverse log) (run-hook-wrapped 'romacs-rhw-d (lambda (f) (and (eq f t) 'saw-t))))))
(list (condition-case e (run-hook-wrapped 1 'ignore) (error e)) (condition-case e (funcall 'run-hook-wrapped 'romacs-rhw-e) (error e)) (progn (setq romacs-rhw-f '(car)) (condition-case e (run-hook-wrapped 'romacs-rhw-f (lambda (f) (error "boom %s" f))) (error e))))
"###;

#[test]
fn run_hook_wrapped_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        // As character codes, as tests/json.rs compares.
        let source = format!("(append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil)");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
