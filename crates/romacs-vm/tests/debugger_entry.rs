//! When a signal enters the debugger, against pinned GNU's eval.c
//! signal_or_quit, maybe_call_debugger and call_debugger: only with
//! `debug-on-signal', no catching handler, or a clause listing `debug';
//! as `debug-on-error', `debug-on-quit', `debug-ignored-errors' and
//! `inhibit-debugger' say; once per input event; for errors raised by
//! primitives as by `signal'; and with the debugger's escape replacing the
//! error's. `debugger' is a recorder, so neither engine's debug.el runs.
use romacs_vm::Vm;

const SETUP: &str = r#"(progn
  (defvar t-log nil)
  (defun t-record (&rest args)
    (push (list args inhibit-debugger debugger-may-continue debug-on-next-call) t-log)
    nil)
  (defun t-run (thunk)
    (setq t-log nil internal-when-entered-debugger -1)
    (let ((value (condition-case e (let ((debugger #'t-record) (debug-ignored-errors nil)) (funcall thunk))
                   (t (list 'escaped e)))))
      (list value (reverse t-log)))))"#;

const CASES: &[&str] = &[
    // A clause naming the condition: no debugger, as GNU's Lisp handlers.
    "(lambda () (let ((debug-on-error t)) (condition-case e (car 1) (error (list 'caught e)))))",
    "(lambda () (let ((debug-on-error t)) (condition-case e (car 1) ((error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error t)) (condition-case e (car 1) (t (list 'caught e)))))",
    // A clause listing `debug' (condition-case-unless-debug's).
    "(lambda () (let ((debug-on-error t)) (condition-case e (car 1) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error nil)) (condition-case e (car 1) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error t)) (condition-case e (car 1) ((debug t) (list 'caught e)))))",
    // `debug-on-signal' enters it even inside an ordinary handler.
    "(lambda () (let ((debug-on-error t) (debug-on-signal t)) (condition-case e (car 1) (error (list 'caught e)))))",
    "(lambda () (let ((debug-on-error nil) (debug-on-signal t)) (condition-case e (car 1) (error (list 'caught e)))))",
    // `debug-on-error' as a list of conditions.
    "(lambda () (let ((debug-on-error '(arith-error))) (condition-case e (car 1) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error '(arith-error))) (condition-case e (/ 1 0) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error '(error))) (condition-case e (/ 1 0) ((debug error) (list 'caught e)))))",
    // `debug-ignored-errors': a condition, or a regexp over the message.
    "(lambda () (let ((debug-on-error t) (debug-ignored-errors '(wrong-type-argument))) (condition-case e (car 1) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error t) (debug-ignored-errors '(\"^Wrong type\"))) (condition-case e (car 1) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error t) (debug-ignored-errors '(\"^Nope\" arith-error))) (condition-case e (car 1) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error t) (debug-ignored-errors '(error))) (condition-case e (signal 'file-missing '(\"x\")) ((debug error) (list 'caught e)))))",
    // `inhibit-debugger', and quits under `debug-on-quit'.
    "(lambda () (let ((debug-on-error t) (inhibit-debugger t)) (condition-case e (car 1) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error t)) (condition-case e (signal 'quit nil) ((debug t) (list 'caught e)))))",
    "(lambda () (let ((debug-on-quit t)) (condition-case e (signal 'quit nil) ((debug t) (list 'caught e)))))",
    // Errors primitives raise, as those `signal' and `error' raise.
    "(lambda () (let ((debug-on-error t)) (condition-case e (aref [1] (+ 4 1)) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error t)) (condition-case e (+ 1 'x) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error t)) (condition-case e (with-temp-buffer (setq buffer-read-only t) (insert \"x\")) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error t)) (condition-case e (funcall 't-no-such-function) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error t)) (condition-case e (symbol-value 't-no-such-variable) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error t)) (condition-case e (error \"boom %d\" 1) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error t)) (condition-case e (signal 'wrong-type-argument '(foo)) ((debug error) (list 'caught e)))))",
    "(lambda () (let ((debug-on-error t)) (condition-case e (string-to-number 'x) ((debug error) (list 'caught e)))))",
    // A catch is no handler; the clause past it, t, does not debug.
    "(lambda () (catch 't-done (let ((debug-on-error t) (debugger (lambda (&rest a) (throw 't-done (cons 'debugged a))))) (car 1))))",
    // The debugger's own escape replaces the error.
    "(lambda () (catch 't-tag (let ((debug-on-error t) (debugger (lambda (&rest _) (throw 't-tag 'thrown)))) (condition-case nil (car 1) ((debug error) 'handled)))))",
    "(lambda () (let ((debug-on-error t) (debugger (lambda (&rest _) (error \"in debugger\")))) (condition-case e (car 1) ((debug error) (list 'caught e)))))",
    // Once per input event.
    "(lambda () (let ((debug-on-error t)) (list (condition-case nil (car 1) ((debug error) 'one)) (condition-case nil (cdr 2) ((debug error) 'two)))))",
    // Handler-bind callbacks run first; the debugger then sees the error.
    "(lambda () (let ((debug-on-error t)) (condition-case e (handler-bind-1 (lambda () (car 1)) '(error) (lambda (err) (push (list 'bound err) t-log))) ((debug error) (list 'caught e)))))",
    // signal-hook-function gets the condition and data before any of it.
    "(lambda () (let ((signal-hook-function (lambda (c d) (push (list 'hook c d) t-log)))) (condition-case e (car 1) (error (list 'caught e)))))",
];

fn wrap(case: &str) -> String {
    format!("(let ((print-escape-newlines t)) (prin1-to-string (t-run {case})))")
}

fn oracle() -> Vec<String> {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let cases: String = CASES.iter().map(|case| format!("(princ {}) (terpri)", wrap(case))).collect();
    let source = format!("(progn {SETUP} {cases})");
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &source])
        .env("LANG", "C.UTF-8").output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().lines().map(str::to_owned).collect()
}

fn romacs() -> Vec<String> {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(SETUP).unwrap();
    CASES.iter().map(|case| match vm.eval_source(&wrap(case)).and_then(|v| v.display()) {
        Ok(text) => serde_json::from_str::<String>(&text).unwrap_or(text),
        Err(error) => format!("VM error: {error}"),
    }).collect()
}

#[test]
fn signals_enter_the_debugger_as_gnus_do() {
    let (gnu, ours) = (oracle(), romacs());
    assert_eq!(gnu.len(), CASES.len(), "GNU printed {gnu:?}");
    let failures: Vec<String> = CASES.iter().zip(gnu.iter().zip(&ours))
        .filter(|(_, (gnu, ours))| gnu != ours)
        .map(|(case, (gnu, ours))| format!("{case}\n  gnu:    {gnu}\n  romacs: {ours}"))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
