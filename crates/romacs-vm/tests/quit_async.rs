//! A host's asynchronous C-g (romacs_vm::QuitSignal): GNU keyboard.c
//! handle_interrupt sets `quit-flag' out of band and the evaluator's
//! maybe_quit acts on it at its safepoints, honouring `inhibit-quit' and
//! `throw-on-input'. Each press here comes from another thread, as the GUI's
//! render thread sends it, and must end the evaluation within 100 ms.
use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput, QuitSignal, Vm, WindowEnd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A VM with the Lisp prelude, busy running a command, and the signal its
/// host's input thread holds.
fn busy_vm() -> (Vm, Arc<QuitSignal>) {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let signal = Arc::new(QuitSignal::new());
    vm.share_quit_signal(signal.clone());
    vm.enter_busy();
    (vm, signal)
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| format!("ERROR {e}"))
}

/// Press C-g PRESSES times, AFTER from now and then every 50 ms; the
/// moment of the last press.
fn press(signal: &Arc<QuitSignal>, after: Duration, presses: usize) -> std::thread::JoinHandle<Instant> {
    let signal = signal.clone();
    std::thread::spawn(move || {
        std::thread::sleep(after);
        for n in 0..presses {
            if n > 0 { std::thread::sleep(Duration::from_millis(50)); }
            assert!(signal.interrupt(), "a busy evaluator takes the press");
        }
        Instant::now()
    })
}

/// SOURCE's printed value, C-g pressed 200 ms into it; asserts that the
/// evaluation ended within 100 ms of the press.
fn quit_into(vm: &mut Vm, signal: &Arc<QuitSignal>, source: &str) -> String {
    let presser = press(signal, Duration::from_millis(200), 1);
    let value = eval(vm, source);
    let ended = Instant::now();
    let pressed = presser.join().unwrap();
    let latency = ended.saturating_duration_since(pressed);
    assert!(latency < Duration::from_millis(100), "{source}: ended {latency:?} after C-g");
    value
}

/// SOURCE's printed value, C-g pressed once MARKER exists (written by a
/// form SOURCE evaluates), so the quit always lands after work has run
/// however long reading took; asserts the evaluation ended within 100 ms of
/// the press.
fn quit_after_marker(vm: &mut Vm, signal: &Arc<QuitSignal>, source: &str, marker: &std::path::Path) -> String {
    let (signal_, marker_) = (signal.clone(), marker.to_path_buf());
    let presser = std::thread::spawn(move || {
        let start = Instant::now();
        while !marker_.exists() {
            assert!(start.elapsed() < Duration::from_secs(60), "the first form never ran");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(signal_.interrupt(), "a busy evaluator takes the press");
        Instant::now()
    });
    let value = eval(vm, source);
    let ended = Instant::now();
    let pressed = presser.join().unwrap();
    let latency = ended.saturating_duration_since(pressed);
    assert!(latency < Duration::from_millis(100), "{source}: ended {latency:?} after C-g");
    value
}

#[test]
fn a_tight_loop_and_a_calling_loop_quit() {
    let (mut vm, signal) = busy_vm();
    assert_eq!(quit_into(&mut vm, &signal, "(condition-case nil (while t) (quit 'quit))"), "quit");
    eval(&mut vm, "(defun quit-async-f (x) (1+ x))");
    assert_eq!(quit_into(&mut vm, &signal, "(condition-case nil (let ((n 0)) (while t (setq n (quit-async-f n)))) (quit 'quit))"), "quit");
    // The signal is consumed: nothing is pending afterwards.
    assert_eq!(eval(&mut vm, "(list quit-flag (input-pending-p))"), "(nil nil)");
    assert_eq!(eval(&mut vm, "(let ((n 0)) (while (< n 100000) (setq n (1+ n))) n)"), "100000");
}

#[test]
fn a_pathological_regexp_quits() {
    let (mut vm, signal) = busy_vm();
    // Exponential backtracking through a back reference.
    let pattern = r#""\\(a*\\)*\\1b""#;
    assert_eq!(quit_into(&mut vm, &signal, &format!("(condition-case nil (string-match {pattern} (make-string 40 ?a)) (quit 'quit))")), "quit");
    assert_eq!(quit_into(&mut vm, &signal, &format!(
        "(with-temp-buffer (insert (make-string 40 ?a)) (goto-char (point-min)) (condition-case nil (re-search-forward {pattern} nil t) (quit 'quit)))")), "quit");
    assert_eq!(eval(&mut vm, "quit-flag"), "nil");
}

/// The Lisp a read's wait runs quits as any other, and its caller sees `quit',
/// which `romacs--host-event' takes for the read's key, rather than an error
/// named after one. A press stops a loop at a safepoint and a regexp in its
/// matcher: two routes out.
#[test]
fn a_quit_in_a_reads_wait_stays_a_quit() {
    let pattern = r#""\\(a*\\)*\\1b""#;
    for body in [format!("(string-match {pattern} (make-string 40 ?a))"), "(while t)".to_string()] {
        let (mut vm, signal) = busy_vm();
        vm.set_frame_geometry(800, 600, 20, 10).unwrap();
        vm.set_host_input(Some(Box::new(Arriving(Arc::new(AtomicBool::new(false)), vec![]))));
        eval(&mut vm, &format!("(defun romacs--wait-state () {body})"));
        let read = "(condition-case err (romacs--read-host-event 1) (quit (list 'quit err)) (error (list 'error err)))";
        assert_eq!(quit_into(&mut vm, &signal, read), "(quit (quit))", "{body}");
    }
}

/// A host snapshot outside any read has no engine to take the escape: the
/// failure is the host's, and the next evaluation does not raise it again.
#[test]
fn a_quit_in_a_hosts_snapshot_does_not_outlive_it() {
    let (mut vm, signal) = busy_vm();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    eval(&mut vm, "(defun romacs--run-window-change-functions () (while t))");
    let presser = press(&signal, Duration::from_millis(200), 1);
    assert!(vm.frame_snapshot(800, 600).is_err(), "the press ended the window change functions");
    presser.join().unwrap();
    eval(&mut vm, "(defun romacs--run-window-change-functions () nil)");
    assert_eq!(eval(&mut vm, "(eval '(+ 1 1) t)"), "2");
}

/// Quitting out of `eval-buffer' and `load' keeps the forms evaluated
/// before the quit, as GNU's readevalloop does.
#[test]
fn eval_buffer_and_load_quit_between_forms_keeping_what_ran() {
    let (mut vm, signal) = busy_vm();
    // Forms that reach no safepoint of their own: a primitive's work, then
    // a count. Only the poll between forms can end the load.
    const FORMS: usize = 200_000;
    let marker = std::env::temp_dir().join(format!("quit-async-{}.ran", std::process::id()));
    // The eleventh form leaves MARKER, so C-g comes after ten counted forms
    // ran even when reading the file is slow under load.
    let form = "(progn (make-string 30000 ?a) (setq quit-async-n (1+ quit-async-n)))\n";
    let text = form.repeat(10) + &format!("(write-region \"\" nil {:?})\n", marker.display().to_string())
        + &form.repeat(FORMS - 10);
    let file = std::env::temp_dir().join(format!("quit-async-{}.el", std::process::id()));
    // Removed however the test ends: a failed run must not leave its
    // megabytes in TMPDIR for other suites to list.
    let _cleanup = Cleanup([file.clone(), marker.clone()]);
    std::fs::write(&file, &text).unwrap();
    eval(&mut vm, &format!("(progn (defvar quit-async-n 0) (set-buffer (get-buffer-create \"quit-async\")) (insert-file-contents {:?}) t)", file.display().to_string()));
    let quit = quit_after_marker(&mut vm, &signal, "(condition-case nil (progn (eval-buffer) 'done) (quit 'quit))", &marker);
    std::fs::remove_file(&marker).unwrap();
    let n: usize = eval(&mut vm, "quit-async-n").parse().unwrap();
    assert_eq!(quit, "quit");
    assert!(n > 0 && n < FORMS, "eval-buffer ran {n} forms");
    eval(&mut vm, "(setq quit-async-n 0)");
    let quit = quit_after_marker(&mut vm, &signal, &format!("(condition-case nil (progn (load {:?} nil t t) 'done) (quit 'quit))", file.display().to_string()), &marker);
    let n: usize = eval(&mut vm, "quit-async-n").parse().unwrap();
    assert_eq!(quit, "quit");
    assert!(n > 0 && n < FORMS, "load ran {n} forms");
}

// A macro whose expansion never ends quits like any loop, to the handler
// around the expansion: in `eval' at the call's reach, inside a function
// `eval' defined, through `macroexpand', and during `load''s expansion of
// a top-level form (a quit is no error, so no "Eager macro-expansion
// failure" wraps it).
fn endless_macro_vm() -> (Vm, Arc<QuitSignal>) {
    let (mut vm, signal) = busy_vm();
    eval(&mut vm, "(defmacro quit-async-endless () (while t) nil)");
    (vm, signal)
}

#[test]
fn an_endless_expansion_in_eval_quits_to_its_handler() {
    let (mut vm, signal) = endless_macro_vm();
    assert_eq!(quit_into(&mut vm, &signal, "(eval '(condition-case nil (quit-async-endless) (quit 'quit)) t)"), "quit");
    assert_eq!(eval(&mut vm, "(list quit-flag (input-pending-p))"), "(nil nil)");
}

#[test]
fn an_endless_expansion_in_a_function_eval_defined_quits() {
    let (mut vm, signal) = endless_macro_vm();
    eval(&mut vm, "(eval '(defun quit-async-g () (quit-async-endless)) t)");
    assert_eq!(quit_into(&mut vm, &signal, "(condition-case nil (quit-async-g) (quit 'quit))"), "quit");
}

#[test]
fn an_endless_macroexpand_quits() {
    let (mut vm, signal) = endless_macro_vm();
    assert_eq!(quit_into(&mut vm, &signal, "(condition-case nil (macroexpand '(quit-async-endless)) (quit 'quit))"), "quit");
}

#[test]
fn an_endless_expansion_during_load_quits_unwrapped() {
    let (mut vm, signal) = endless_macro_vm();
    let file = std::env::temp_dir().join(format!("quit-async-macro-{}.el", std::process::id()));
    let _cleanup = Cleanup([file.clone()]);
    std::fs::write(&file, ";;; -*- lexical-binding: t -*-\n(quit-async-endless)\n").unwrap();
    let load = format!("(condition-case nil (progn (load {:?} nil t t) 'done) (quit 'quit))", file.display().to_string());
    assert_eq!(quit_into(&mut vm, &signal, &load), "quit");
}

/// The escape that leaves a quit expansion is raised at the form's reach,
/// whatever the quit became on its way out, and the macro does not run
/// again: here the macro's cleanup replaces the quit with its own error,
/// as GNU's unwind-protect lets it.
#[test]
fn a_quit_expansion_raises_what_left_it() {
    let (mut vm, signal) = endless_macro_vm();
    eval(&mut vm, "(defmacro quit-async-cleanup () (unwind-protect (while t) (error \"cleanup\")) nil)");
    let form = "(eval '(condition-case e (quit-async-cleanup) (error (list 'error e)) (quit 'quit)) t)";
    assert_eq!(quit_into(&mut vm, &signal, form), r#"(error (error "cleanup"))"#);
}

/// A C-g that lands while a function's macro call expands quits that call,
/// and only that call: the quit is raised at the call's reach once, and
/// the next call expands the macro again and runs, as GNU's does (pinned
/// GNU batch: `quit`, then `(before expanded)` twice; and `caught` for the
/// handler in the same form). The quit must not stay with the function.
#[test]
fn a_quit_in_an_expansion_does_not_stay_in_the_function() {
    let (mut vm, _signal) = busy_vm();
    eval(&mut vm, "(defvar quit-async-arm nil) (defvar quit-async-expansions 0)");
    // Armed, the expansion sets `quit-flag' and reaches a safepoint: the
    // quit `maybe_quit` raises there is a C-g's.
    eval(&mut vm, "(defmacro quit-async-once ()
                     (setq quit-async-expansions (1+ quit-async-expansions))
                     (when quit-async-arm
                       (setq quit-async-arm nil quit-flag t)
                       (let ((i 0)) (while (< i 100000) (setq i (1+ i)))))
                     ''expanded)");
    let first = "(condition-case nil
                     (progn (setq quit-async-arm t)
                            (eval '(defun quit-async-h () (list 'before (quit-async-once))) t)
                            (quit-async-h))
                   (quit 'quit))";
    assert_eq!(eval(&mut vm, first), "quit");
    let again = "(condition-case nil (quit-async-h) (quit 'quit-again))";
    assert_eq!(eval(&mut vm, again), "(before expanded)", "the quit stayed in the function");
    assert_eq!(eval(&mut vm, again), "(before expanded)");
    // The interrupted expansion, and one more when the call is reached again.
    assert_eq!(eval(&mut vm, "quit-async-expansions"), "2");
    // A handler in the same form still catches the quit, and the form runs
    // the next time it is evaluated.
    eval(&mut vm, "(eval '(defun quit-async-k () (condition-case nil (quit-async-once) (quit 'caught))) t)");
    assert_eq!(eval(&mut vm, "(progn (setq quit-async-arm t) (quit-async-k))"), "caught");
    assert_eq!(eval(&mut vm, "(quit-async-k)"), "expanded");
}

/// The same through `load`, whose forms compile outside `eval`: whether
/// the quit ends the load or the first call, it is raised once, and no
/// later call quits.
#[test]
fn a_quit_in_a_loaded_function_s_expansion_is_raised_once() {
    let (mut vm, _signal) = busy_vm();
    eval(&mut vm, "(defvar quit-async-arm nil)");
    eval(&mut vm, "(defmacro quit-async-once ()
                     (when quit-async-arm
                       (setq quit-async-arm nil quit-flag t)
                       (let ((i 0)) (while (< i 100000) (setq i (1+ i)))))
                     ''expanded)");
    let file = std::env::temp_dir().join(format!("quit-async-once-{}.el", std::process::id()));
    let _cleanup = Cleanup([file.clone()]);
    std::fs::write(&file, ";;; -*- lexical-binding: t -*-\n(defun quit-async-l () (list 'before (quit-async-once)))\n").unwrap();
    let first = format!("(condition-case nil (progn (setq quit-async-arm t) (load {:?} nil t t) (quit-async-l)) (quit 'quit))",
                        file.display().to_string());
    assert_eq!(eval(&mut vm, &first), "quit");
    let again = "(condition-case nil (quit-async-l) (quit 'quit-again) (void-function 'not-loaded))";
    assert_ne!(eval(&mut vm, again), "quit-again", "the quit stayed in the function");
    assert_ne!(eval(&mut vm, again), "quit-again");
}

/// A `throw-on-input` throw that leaves an expansion is thrown once, to the
/// form that bound `throw-on-input` (as `while-no-input` does); the
/// function then runs anywhere.
#[test]
fn a_throw_on_input_in_an_expansion_is_thrown_once() {
    let (mut vm, _signal) = busy_vm();
    eval(&mut vm, "(defvar quit-async-arm nil)");
    eval(&mut vm, "(defmacro quit-async-input ()
                     (when quit-async-arm
                       (setq quit-async-arm nil quit-flag throw-on-input)
                       (let ((i 0)) (while (< i 100000) (setq i (1+ i)))))
                     ''expanded)");
    let first = "(progn (setq quit-async-arm t)
                        (catch 'quit-async-input
                          (let ((throw-on-input 'quit-async-input))
                            (eval '(defun quit-async-i () (list 'before (quit-async-input))) t)
                            (quit-async-i))))";
    assert_eq!(eval(&mut vm, first), "t");
    assert_eq!(eval(&mut vm, "(quit-async-i)"), "(before expanded)", "the throw stayed in the function");
}

/// A quit that left the expansion of a call in a branch the interrupted
/// evaluation does not take quits that evaluation all the same, at the end
/// of the body that was compiling: the call the C-g interrupted. Nothing is
/// kept for a later evaluation, of the branch or of anything else.
#[test]
fn a_quit_in_an_untaken_branch_s_expansion_quits_the_call_that_compiled() {
    let (mut vm, _signal) = busy_vm();
    eval(&mut vm, "(defvar quit-async-arm nil) (defvar quit-async-expansions 0)");
    eval(&mut vm, "(defmacro quit-async-once ()
                     (setq quit-async-expansions (1+ quit-async-expansions))
                     (when quit-async-arm
                       (setq quit-async-arm nil quit-flag t)
                       (let ((i 0)) (while (< i 100000) (setq i (1+ i)))))
                     ''expanded)");
    eval(&mut vm, "(eval '(defun quit-async-b (x) (if x (list 'took (quit-async-once)) 'skipped)) t)");
    assert_eq!(eval(&mut vm, "(condition-case nil (progn (setq quit-async-arm t) (quit-async-b nil)) (quit 'quit))"), "quit");
    assert_eq!(eval(&mut vm, "(list quit-flag (and (boundp 'romacs--deferred-escapes) romacs--deferred-escapes))"), "(nil nil)");
    assert_eq!(eval(&mut vm, "(condition-case nil (quit-async-b nil) (quit 'quit-later))"), "skipped");
    let taken = "(condition-case nil (quit-async-b t) (quit 'quit-later))";
    assert_eq!(eval(&mut vm, taken), "(took expanded)", "the quit waited in the function");
    assert_eq!(eval(&mut vm, taken), "(took expanded)");
    assert_eq!(eval(&mut vm, "quit-async-expansions"), "2");
}

/// A quit the compiling evaluation neither reached nor ended on, because an
/// error left it first, ends with the request: no later evaluation quits.
#[test]
fn a_deferred_quit_an_error_passed_over_ends_with_the_request() {
    let (mut vm, _signal) = busy_vm();
    eval(&mut vm, "(defvar quit-async-arm nil)");
    eval(&mut vm, "(defmacro quit-async-once ()
                     (when quit-async-arm
                       (setq quit-async-arm nil quit-flag t)
                       (let ((i 0)) (while (< i 100000) (setq i (1+ i)))))
                     ''expanded)");
    eval(&mut vm, "(eval '(defun quit-async-e (x) (if x (quit-async-once) (error \"left\"))) t)");
    assert_eq!(eval(&mut vm, "(condition-case e (progn (setq quit-async-arm t) (quit-async-e nil)) (quit 'quit) (error (car (cdr e))))"), "\"left\"");
    assert_eq!(eval(&mut vm, "(list quit-flag (and (boundp 'romacs--deferred-escapes) romacs--deferred-escapes))"), "(nil nil)");
    assert_eq!(eval(&mut vm, "(condition-case nil (quit-async-e t) (quit 'quit-later))"), "expanded");
}

/// A worker serves every job in one request, so a request's end is not a
/// job's. A quit deferred in one job's compile, whose reach and body's end
/// an error then passed over, ends with that job
/// (`romacs--isolate-run-job`): the next job to take the branch runs, and
/// does not quit for an interrupt it never had.
#[test]
fn a_deferred_quit_a_job_left_behind_ends_with_the_job() {
    let (mut vm, _signal) = busy_vm();
    eval(&mut vm, "(defvar quit-async-arm nil) (defvar quit-async-second nil)");
    eval(&mut vm, "(defmacro quit-async-once ()
                     (when quit-async-arm
                       (setq quit-async-arm nil quit-flag t)
                       (let ((i 0)) (while (< i 100000) (setq i (1+ i)))))
                     ''expanded)");
    eval(&mut vm, "(eval '(defun quit-async-w (x) (if x (list 'took (quit-async-once)) (error \"left\"))) t)");
    eval(&mut vm, "(defun quit-async-job-1 () (setq quit-async-arm t) (quit-async-w nil))");
    eval(&mut vm, "(defun quit-async-job-2 () (setq quit-async-second (condition-case nil (quit-async-w t) (quit 'quit))))");
    // Both jobs in one request, as the worker's loop runs them; a reply
    // fails harmlessly without a parent.
    let jobs = "(progn (romacs--isolate-run-job 1 (list 'quit-async-job-1 (freeze nil) (freeze nil) nil nil))
                       (romacs--isolate-run-job 2 (list 'quit-async-job-2 (freeze nil) (freeze nil) nil nil))
                       (list quit-async-arm quit-async-second))";
    assert_eq!(eval(&mut vm, jobs), "(nil (took expanded))", "the second job took the first job's quit");
}

/// A handler landing between the deferred quit and its reach does not turn
/// the quit into an ordinary error for an expansion it then leaves: taken
/// at the reach it is a quit unwinding again, so the outer macro's
/// expansion is left once and the outer macro does not run again.
#[test]
fn a_deferred_quit_taken_after_a_handler_landed_is_still_a_quit() {
    let (mut vm, _signal) = busy_vm();
    eval(&mut vm, "(defvar quit-async-arm nil) (defvar quit-async-outer-runs 0)");
    eval(&mut vm, "(defmacro quit-async-once ()
                     (when quit-async-arm
                       (setq quit-async-arm nil quit-flag t)
                       (let ((i 0)) (while (< i 100000) (setq i (1+ i)))))
                     ''expanded)");
    eval(&mut vm, "(defmacro quit-async-landing ()
                     (setq quit-async-outer-runs (1+ quit-async-outer-runs))
                     (eval '(progn (condition-case nil (error \"x\") (error nil)) (quit-async-once)) t)
                     nil)");
    let form = "(progn (setq quit-async-arm t) (eval '(condition-case nil (quit-async-landing) (quit 'quit)) t))";
    assert_eq!(eval(&mut vm, form), "quit");
    assert_eq!(eval(&mut vm, "quit-async-outer-runs"), "1");
}

/// A `throw-on-input` throw that left the expansion of a call in an
/// untaken branch is thrown at the end of the body that was compiling, to
/// the form that bound `throw-on-input` (as `while-no-input` does), which
/// the C-g interrupted: nothing is thrown later, to a tag no one catches.
#[test]
fn a_throw_on_input_in_an_untaken_branch_s_expansion_ends_with_its_form() {
    let (mut vm, _signal) = busy_vm();
    eval(&mut vm, "(defvar quit-async-arm nil)");
    eval(&mut vm, "(defmacro quit-async-input ()
                     (when quit-async-arm
                       (setq quit-async-arm nil quit-flag throw-on-input)
                       (let ((i 0)) (while (< i 100000) (setq i (1+ i)))))
                     ''expanded)");
    let first = "(progn (setq quit-async-arm t)
                        (catch 'quit-async-input
                          (let ((throw-on-input 'quit-async-input))
                            (eval '(defun quit-async-j (x) (if x (list 'took (quit-async-input)) 'skipped)) t)
                            (quit-async-j nil))))";
    assert_eq!(eval(&mut vm, first), "t");
    assert_eq!(eval(&mut vm, "(condition-case e (quit-async-j nil) (error (list 'error (car e))))"), "skipped");
    assert_eq!(eval(&mut vm, "(condition-case e (quit-async-j t) (error (list 'error (car e))))"), "(took expanded)");
    assert_eq!(eval(&mut vm, "(list quit-flag (and (boundp 'romacs--deferred-escapes) romacs--deferred-escapes))"), "(nil nil)");
}

/// A quit taken by an `eval' nested in a macro's expansion leaves the
/// outer expansion too: one C-g, and the outer macro does not run again.
#[test]
fn a_quit_in_a_nested_eval_leaves_the_outer_expansion() {
    let (mut vm, signal) = endless_macro_vm();
    eval(&mut vm, "(defmacro quit-async-outer () (eval '(quit-async-endless) t) nil)");
    assert_eq!(quit_into(&mut vm, &signal, "(eval '(condition-case nil (quit-async-outer) (quit 'quit)) t)"), "quit");
}

/// Quits `quit-flag' raises inside a macro's expansion, against pinned GNU
/// batch: a quit the macro catches itself leaves nothing to raise, so its
/// later error expands again at the reach, under the form's own binding
/// (GNU `(error inner)'); a `no-catch' a cleanup signals stays a signal
/// (GNU `caught'); a cleanup's error replaces the quit (GNU `(error (error
/// "cleanup"))'); a quit through a nested `eval' leaves the outer
/// expansion, which does not run again (GNU `(quit (outer-ran))'); and a
/// `throw-on-input' throw stays a throw through a cleanup (GNU `t'), while
/// a `no-catch' a cleanup signals stays a signal though a throw elsewhere
/// made one meanwhile, or though it carries a caught throw's very data
/// (GNU `caught', both); and a throw stays a throw though its cleanup
/// catches another throw's `no-catch', or sixteen nested ones (GNU `t').
#[test]
fn quit_flag_in_an_expansion_raises_what_gnu_does() {
    let (mut vm, _signal) = busy_vm();
    eval(&mut vm, "(progn (defvar rt-log nil) (defvar rt-phase 'outer)
      (defmacro rt-cq () (condition-case nil (progn (setq quit-flag t) (while t)) (quit nil)) (signal 'error (list rt-phase)))
      (defmacro rt-nc () (unwind-protect (progn (setq quit-flag t) (while t)) (signal 'no-catch '(done 7))))
      (defmacro rt-cu () (unwind-protect (progn (setq quit-flag t) (while t)) (error \"cleanup\")))
      (defmacro rt-q0 () (setq quit-flag t) (while t))
      (defmacro rt-nq () (push 'outer-ran rt-log) (eval '(rt-q0) t))
      (defmacro rt-ti () (unwind-protect (progn (setq quit-flag 'done) (while t)) nil))
      (defmacro rt-swallow () (condition-case nil (throw 'missing 1) (no-catch nil)))
      (defmacro rt-stale () (unwind-protect (unwind-protect (progn (setq quit-flag t) (while t)) (signal 'no-catch '(done 7))) (eval '(rt-swallow) t)))
      (defvar rt-saved (condition-case e (eval '(throw 'done 7) t) (no-catch e)))
      (defmacro rt-resignal () (unwind-protect (progn (setq quit-flag t) (while t)) (signal 'no-catch (cdr rt-saved))))
      (defmacro rt-ti-nested () (unwind-protect (progn (setq quit-flag 'done) (while t)) (eval '(rt-swallow) t)))
      (defun rt-nested-no-catch (n) (when (> n 0) (condition-case nil (unwind-protect (throw 'missing n) (rt-nested-no-catch (1- n))) (no-catch nil))))
      (defmacro rt-swallow-deep () (rt-nested-no-catch 16) nil)
      (defmacro rt-ti-deep () (unwind-protect (progn (setq quit-flag 'done) (while t)) (eval '(rt-swallow-deep) t))))");
    let cases = [
        ("(eval '(let ((rt-phase 'inner)) (condition-case e (rt-cq) (error e))) t)", "(error inner)"),
        ("(eval '(catch 'done (condition-case nil (rt-nc) (no-catch 'caught))) t)", "caught"),
        ("(eval '(condition-case e (rt-cu) (error (list 'error e)) (quit 'quit)) t)", r#"(error (error "cleanup"))"#),
        ("(progn (setq rt-log nil) (list (eval '(condition-case nil (rt-nq) (quit 'quit)) t) rt-log))", "(quit (outer-ran))"),
        ("(let ((throw-on-input 'done)) (eval '(catch 'done (list 'body (rt-ti))) t))", "t"),
        ("(eval '(catch 'done (condition-case nil (rt-stale) (no-catch 'caught))) t)", "caught"),
        ("(eval '(catch 'done (condition-case nil (rt-resignal) (no-catch 'caught))) t)", "caught"),
        ("(let ((throw-on-input 'done)) (eval '(catch 'done (condition-case nil (rt-ti-nested) (no-catch 'wrong-handler))) t))", "t"),
        ("(let ((throw-on-input 'done)) (eval '(catch 'done (condition-case nil (rt-ti-deep) (no-catch 'wrong-handler))) t))", "t"),
    ];
    for (form, gnu) in cases {
        assert_eq!(eval(&mut vm, form), gnu, "{form}");
    }
}

/// Removes its paths when dropped, so a panicking test cleans up too.
struct Cleanup<const N: usize>([std::path::PathBuf; N]);

impl<const N: usize> Drop for Cleanup<N> {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[test]
fn inhibit_quit_holds_the_quit_until_it_is_unbound() {
    let (mut vm, signal) = busy_vm();
    let presser = press(&signal, Duration::from_millis(100), 1);
    let started = Instant::now();
    let value = eval(&mut vm, "(condition-case nil
                                 (progn (let ((inhibit-quit t) (end (+ (float-time) 0.4)))
                                          (while (< (float-time) end))
                                          (setq quit-async-held quit-flag))
                                        (while t))
                               (quit (list 'quit quit-async-held)))");
    presser.join().unwrap();
    assert_eq!(value, "(quit t)", "held in `quit-flag' while inhibited, then quit");
    assert!(started.elapsed() >= Duration::from_millis(400));
}

/// GNU's forced quit for a graphical frame: the third C-g while the first
/// is still pending clears `inhibit-quit'.
#[test]
fn a_third_press_forces_the_quit() {
    let (mut vm, signal) = busy_vm();
    let presser = press(&signal, Duration::from_millis(100), 3);
    let value = eval(&mut vm, "(condition-case nil (let ((inhibit-quit t)) (while t)) (quit 'forced))");
    let pressed = presser.join().unwrap();
    assert_eq!(value, "forced");
    assert!(pressed.elapsed() < Duration::from_millis(100));
}

/// A host keyboard whose input arrives FROM_NOW + 200 ms and holds C-g.
struct Arriving(Arc<AtomicBool>, Vec<i64>);
impl HostInput for Arriving {
    fn input_pending(&mut self) -> bool { self.0.load(Ordering::SeqCst) || !self.1.is_empty() }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<WindowEnd>, String> { Ok(Vec::new()) }
    fn next_event(&mut self, _: Option<Duration>) -> Result<HostEvent, String> {
        Ok(match self.1.pop() { Some(c) => HostEvent::Key(CommandEvent::Character(c)), None => HostEvent::Timeout })
    }
}

/// `while-no-input''s mechanism: input arriving while `throw-on-input' is
/// bound sets `quit-flag' to its tag, and the next safepoint throws there.
#[test]
fn input_throws_to_throw_on_input() {
    let (mut vm, _signal) = busy_vm();
    let arrived = Arc::new(AtomicBool::new(false));
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    vm.set_host_input(Some(Box::new(Arriving(arrived.clone(), Vec::new()))));
    let flag = arrived.clone();
    let typist = std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(200)); flag.store(true, Ordering::SeqCst); Instant::now() });
    let value = eval(&mut vm, "(catch 'quit-async-input (let ((throw-on-input 'quit-async-input)) (while t)))");
    let typed = typist.join().unwrap();
    assert_eq!(value, "t");
    assert!(typed.elapsed() < Duration::from_millis(100));
    assert_eq!(eval(&mut vm, "quit-flag"), "nil");
}

/// read_char's quit: a C-g read from the keyboard quits `read-event'
/// unless `inhibit-quit' is non-nil, when it is the event; a press that
/// came while the evaluator was busy is the next read's C-g; a key
/// sequence reads it as a key and clears `quit-flag'.
#[test]
fn a_keyboard_read_takes_c_g_as_read_char_does() {
    let (mut vm, signal) = busy_vm();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    let keys = |vm: &mut Vm, keys: Vec<i64>| vm.set_host_input(Some(Box::new(Arriving(Arc::new(AtomicBool::new(false)), keys))));
    keys(&mut vm, vec![7]);
    assert_eq!(eval(&mut vm, "(condition-case nil (read-event) (quit 'quit))"), "quit");
    keys(&mut vm, vec![7]);
    assert_eq!(eval(&mut vm, "(let ((inhibit-quit t)) (list (read-event) quit-flag))"), "(7 nil)");
    keys(&mut vm, vec![]);
    let presser = press(&signal, Duration::from_millis(100), 1);
    assert_eq!(eval(&mut vm, "(let ((inhibit-quit t) (end (+ (float-time) 0.3))) (while (< (float-time) end)) (list (read-event) quit-flag))"),
               "(7 nil)", "a press held by `inhibit-quit' is the next read's C-g");
    presser.join().unwrap();
    keys(&mut vm, vec![7]);
    assert_eq!(eval(&mut vm, "(list (read-key-sequence-vector nil) quit-flag)"), "([7] nil)");
    // An unread C-g is no keyboard quit.
    assert_eq!(eval(&mut vm, "(let ((unread-command-events (list 7))) (read-event))"), "7");
}

/// The input side counts a press only while the evaluator is busy; a
/// waiting evaluator reads its C-g as a key.
#[test]
fn a_waiting_evaluator_reads_c_g_itself() {
    let (mut vm, signal) = busy_vm();
    vm.leave_busy();
    assert!(!signal.interrupt());
    assert!(!vm.take_quit_key());
    vm.enter_busy();
    assert!(signal.interrupt());
    vm.leave_busy();
    assert_eq!(eval(&mut vm, "(list quit-flag (input-pending-p))"), "(t t)", "untaken, it is pending input");
    assert!(vm.take_quit_key(), "the host reads it as the next key");
    assert_eq!(eval(&mut vm, "quit-flag"), "nil");
}

/// `quit-flag' set by Lisp, as GNU's eval.c and subr.el's with-local-quit
/// use it (the same forms run against pinned GNU in the lane's oracle).
#[test]
fn lisp_quit_flag_follows_maybe_quit() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    assert_eq!(eval(&mut vm, "(condition-case nil (progn (setq quit-flag t) (while t)) (quit 'quit))"), "quit");
    assert_eq!(eval(&mut vm, "(let ((inhibit-quit t))
                                (prog1 (list (condition-case nil
                                                 (let ((inhibit-quit nil)) (setq quit-flag t) (while t) 'not)
                                               (quit (setq quit-flag t) (eval '(progn nil) t) nil))
                                             quit-flag)
                                  (setq quit-flag nil)))"), "(nil t)");
    assert_eq!(eval(&mut vm, "(catch 'quit-async-x (let ((throw-on-input 'quit-async-x)) (setq quit-flag 'quit-async-x) (while t)))"), "t");
    assert_eq!(eval(&mut vm, "(let ((inhibit-quit t)) (setq quit-flag t) (prog1 (input-pending-p) (setq quit-flag nil)))"), "t");
}

/// GNU's eval_sub and funcall run maybe_quit before the body: a quit due at
/// an engine's entry ends it before straight-line code runs.
#[test]
fn a_due_quit_ends_an_entry_before_its_first_instruction() {
    let (mut vm, signal) = busy_vm();
    assert!(signal.interrupt());
    assert!(eval(&mut vm, "(setq quit-async-side 1)").starts_with("ERROR"), "quit before the setq");
    assert_eq!(eval(&mut vm, "(boundp 'quit-async-side)"), "nil");
    assert_eq!(eval(&mut vm, "(list (condition-case nil (progn (setq quit-flag t) (eval '(setq quit-async-side 3) t)) (quit 'quit)) (boundp 'quit-async-side))"), "(quit nil)");
}

/// A keyboard read leaves the evaluator busy while it redisplays (its Lisp
/// stays quittable, as GNU's is) and marks it waiting only while it blocks.
#[test]
fn a_read_is_waiting_only_while_it_blocks() {
    struct Watching(Arc<QuitSignal>, Arc<std::sync::Mutex<Vec<(&'static str, bool)>>>);
    impl HostInput for Watching {
        fn input_pending(&mut self) -> bool { false }
        fn redisplay(&mut self, _: HostFrame) -> Result<Vec<WindowEnd>, String> {
            self.1.lock().unwrap().push(("redisplay", self.0.busy_for().is_some()));
            Ok(Vec::new())
        }
        fn next_event(&mut self, _: Option<Duration>) -> Result<HostEvent, String> {
            self.1.lock().unwrap().push(("wait", self.0.busy_for().is_some()));
            Ok(HostEvent::Key(CommandEvent::Character(120)))
        }
    }
    let (mut vm, signal) = busy_vm();
    vm.set_frame_geometry(800, 600, 20, 10).unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    vm.set_host_input(Some(Box::new(Watching(signal.clone(), seen.clone()))));
    assert_eq!(eval(&mut vm, "(read-event)"), "120");
    assert_eq!(*seen.lock().unwrap(), [("redisplay", true), ("wait", false)]);
    assert!(signal.busy_for().is_some(), "busy again after the read");
}

/// Text long enough that one motion over it outlasts the press.
const LONG_BUFFER: &str = "(progn (insert (make-string 30000000 ?\\n)) (goto-char 1))";

/// A motion loops in Rust, with no Lisp to poll: GNU's iterator would finish.
#[test]
fn c_g_stops_a_long_motion() {
    let (mut vm, signal) = busy_vm();
    eval(&mut vm, LONG_BUFFER);
    let value = quit_into(&mut vm, &signal, "(condition-case nil (vertical-motion 30000000) (quit 'quit))");
    assert_eq!(value, "quit");
}

/// `inhibit-quit' holds the press through the motion, for the next safepoint.
#[test]
fn inhibit_quit_holds_a_long_motion() {
    let (mut vm, signal) = busy_vm();
    eval(&mut vm, LONG_BUFFER);
    let presser = press(&signal, Duration::from_millis(100), 1);
    let value = eval(&mut vm, "(let ((inhibit-quit t)) (let ((moved (vertical-motion 3000000))) (setq quit-flag nil) moved))");
    presser.join().unwrap();
    assert_eq!(value, "3000000");
}
