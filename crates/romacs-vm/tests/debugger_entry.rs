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
    compare(CASES, &oracle(), &romacs());
}

// The call boundary's hooks, against GNU byte-code (bytecode.c's call
// opcode and Ffuncall): `debug-on-next-call' stops the next Lisp call with
// `(lambda)' and flags it debug-on-exit; a flagged call that returns gives
// the debugger `(exit VALUE)' and returns its value; one that a throw or an
// error ends gives it nothing, and leaves no flag behind. GNU compiles the
// functions, as Romacs's evaluator always does.
const HOOK_SETUP: &str = r#"(progn
  (defun h-target (n) (* n 2))
  (defun h-throw () (throw 'h-out 'thrown))
  (defun h-error () (car 1))
  (defun h-flagged () (backtrace-debug 0 t 'h-flagged) 5)
  (defun h-unflagged () (backtrace-debug 0 t 'h-unflagged) (backtrace-debug 0 nil 'h-unflagged) 6)
  (defun h-let () (let ((debug-on-next-call t)) (h-target 3)))
  (defun h-quoted () (setq debug-on-next-call t) (prog1 (funcall '(lambda () 42)) (setq debug-on-next-call nil)))
  (defun h-plain (n) (if (> n 0) (h-plain (1- n)) (h-target 1)))
  (defun h-armed (n) (if (> n 0) (h-armed (1- n)) (setq debug-on-next-call t) (prog1 (h-target 1) (setq debug-on-next-call nil))))
  (defun h-limit () (let ((max-lisp-eval-depth 200) (n 0))
    (while (condition-case nil (progn (h-plain n) t) (error nil)) (setq n (1+ n)))
    (h-armed (1- n))))
  (defun h-next (thunk) (setq debug-on-next-call t) (prog1 (funcall thunk) (setq debug-on-next-call nil)))
  (defun h-exit-recorder (&rest args)
    (push (list args inhibit-debugger debugger-may-continue debug-on-next-call) t-log)
    (if (eq (car args) 'exit) (list 'replaced (car (cdr args))) nil))
  (defun h-rearm-recorder (&rest args)
    (push (list args inhibit-debugger debugger-may-continue debug-on-next-call) t-log)
    (when (eq (car args) 'error) (setq debug-on-next-call t) (h-target 5) (setq debug-on-next-call nil))
    nil)
  (mapc #'byte-compile '(h-rearm-recorder h-target h-throw h-error h-flagged h-unflagged h-let h-quoted h-plain h-armed h-limit h-next h-exit-recorder)))"#;

const HOOK_CASES: &[&str] = &[
    "(lambda () (let ((debugger #'h-exit-recorder)) (h-next (lambda () (h-target 7)))))",
    "(lambda () (let ((debugger #'h-exit-recorder)) (list (catch 'h-out (h-next #'h-throw)) (h-target 1))))",
    "(lambda () (let ((debugger #'h-exit-recorder)) (list (condition-case nil (h-next #'h-error) (error 'caught)) (h-target 2))))",
    "(lambda () (let ((debugger #'h-exit-recorder)) (h-let)))",
    "(lambda () (let ((debugger #'h-exit-recorder)) (h-next (lambda () (h-target (+ 1 2))))))",
    "(lambda () (let ((debugger #'h-exit-recorder)) (list (h-flagged) (h-target 4))))",
    "(lambda () (let ((debugger #'h-exit-recorder)) (h-unflagged)))",
    // A quoted lambda's activation, which the funcall made.
    "(lambda () (let ((debugger #'h-exit-recorder)) (h-quoted)))",
    // An error's debugger arms the next call again (debug.el's `d').
    "(lambda () (let ((debugger #'h-rearm-recorder) (debug-on-error t)) (condition-case nil (h-error) ((debug error) 'caught))))",
    // At the depth limit, both stops borrow the debugger's reserve.
    "(lambda () (let ((debugger #'h-exit-recorder)) (h-limit)))",
];

fn hook_oracle() -> Vec<String> {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let cases: String = HOOK_CASES.iter().map(|case| format!("(princ {}) (terpri)", wrap(case))).collect();
    let source = format!("(progn {SETUP} {HOOK_SETUP} (byte-compile 't-run) {cases})");
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &source])
        .env("LANG", "C.UTF-8").output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().lines().map(str::to_owned).collect()
}

fn hook_romacs() -> Vec<String> {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(SETUP).unwrap();
    vm.eval_source(&HOOK_SETUP.replace("(mapc #'byte-compile", "(quote")).unwrap();
    HOOK_CASES.iter().map(|case| match vm.eval_source(&wrap(case)).and_then(|v| v.display()) {
        Ok(text) => serde_json::from_str::<String>(&text).unwrap_or(text),
        Err(error) => format!("VM error: {error}"),
    }).collect()
}

#[test]
fn call_hooks_enter_the_debugger_as_gnus_byte_code_does() {
    compare(HOOK_CASES, &hook_oracle(), &hook_romacs());
}

fn compare(cases: &[&str], gnu: &[String], ours: &[String]) {
    assert_eq!(gnu.len(), cases.len(), "GNU printed {gnu:?}");
    let failures: Vec<String> = cases.iter().zip(gnu.iter().zip(ours))
        .filter(|(_, (gnu, ours))| gnu != ours)
        .map(|(case, (gnu, ours))| format!("{case}\n  gnu:    {gnu}\n  romacs: {ours}"))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
