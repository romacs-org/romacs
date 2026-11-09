//! The engine's safepoint turns an isolate interrupt (SIGUSR1) into `quit'
//! inside CPU-bound Lisp, honouring `inhibit-quit', and a termination
//! (SIGTERM) into one `quit' whatever `inhibit-quit' says.
use romacs_vm::Vm;

fn signal_self_after(signal: &str, millis: u64) {
    let pid = std::process::id().to_string();
    let signal = signal.to_owned();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(millis));
        let _ = std::process::Command::new("kill").arg(format!("-{signal}")).arg(pid).status();
    });
}

#[test]
fn sigusr1_quits_a_tight_loop_and_sigterm_quits_once() {
    romacs_vm::host::catch_isolate_signals().unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let eval = |vm: &mut Vm, src: &str| vm.eval_source(src).and_then(|v| v.display()).unwrap_or_else(|e| format!("ERROR {e}"));
    signal_self_after("USR1", 200);
    assert_eq!(eval(&mut vm, "(condition-case e (while t) (quit 'interrupted))"), "interrupted");
    signal_self_after("USR1", 200);
    assert_eq!(eval(&mut vm, "(condition-case e (let ((n 0)) (while t (setq n (1+ n)))) (quit 'interrupted))"), "interrupted");
    // A lambda form evaluated at run time, as a worker runs a job.
    signal_self_after("USR1", 200);
    assert_eq!(eval(&mut vm, "(condition-case e (funcall (eval '(lambda () (while t)) t)) (quit 'interrupted))"), "interrupted");
    signal_self_after("USR1", 200);
    assert_eq!(eval(&mut vm, "(condition-case e (apply (eval '(lambda (x) (while t x)) t) (list 1)) (quit 'interrupted))"), "interrupted");
    // Held while inhibit-quit is bound, taken once it is not.
    signal_self_after("USR1", 100);
    assert_eq!(eval(&mut vm, "(condition-case e (progn (let ((inhibit-quit t) (n 0)) (while (< n 2000000) (setq n (1+ n)))) (while t)) (quit 'interrupted))"), "interrupted");
    // The worker's shape: the loop's condition-case around a running job.
    vm.eval_source("(defun journey-job () (condition-case err (progn (romacs--isolate-claim 1) (while t)) (quit 'caught-in-job)))").unwrap();
    signal_self_after("USR1", 200);
    assert_eq!(eval(&mut vm, "(let ((n 0) r) (while (< n 1) (setq n (1+ n)) (setq r (journey-job))) r)"), "caught-in-job");
    for (name, body) in [("plain", "(set 'journey-var 1)"), ("env", "(set 'process-environment process-environment)"),
                         ("thaw", "(thaw (freeze (list 1 2)))"), ("exec", "(set 'exec-path (thaw (freeze exec-path)))"),
                         ("dir", "(set 'default-directory default-directory)"),
                         ("prepare", "(romacs--isolate-prepare (list (cons 'journey-seed (list 1 2))) nil)"),
                         ("real-seeds", "(romacs--isolate-prepare (thaw (freeze (romacs--isolate-seeds))) nil)"),
                         ("env-seed", "(set 'process-environment (thaw (freeze process-environment)))"),
                         ("load-path-seed", "(set 'load-path (thaw (freeze load-path)))")] {
        signal_self_after("USR1", 200);
        assert_eq!(eval(&mut vm, &format!("(progn (romacs--isolate-claim 1) (condition-case e (while t {body}) (quit 'caught)))")), "caught", "{name}");
    }
    // The real job runner, whose reply fails harmlessly without a parent.
    vm.eval_source("(defun journey-loop () (while t))").unwrap();
    signal_self_after("USR1", 200);
    assert_eq!(eval(&mut vm, "(condition-case e (list 'returned (romacs--isolate-run-job 1 (list 'journey-loop (freeze nil) (freeze nil) nil nil))) (quit 'escaped))"), "(returned nil)");
    signal_self_after("USR1", 200);
    assert_eq!(eval(&mut vm, "(condition-case e (list 'returned (unwind-protect (condition-case err (progn (romacs--isolate-claim 1) (while t)) (quit 'caught)) nil)) (quit 'escaped))"), "(returned caught)");
    // SIGTERM: one quit, then `romacs--isolate-terminated-p' says so.
    signal_self_after("TERM", 200);
    assert_eq!(eval(&mut vm, "(condition-case e (let ((inhibit-quit t)) (while t)) (quit (list 'terminated (romacs--isolate-terminated-p))))"), "(terminated t)");
    assert_eq!(eval(&mut vm, "(let ((n 0)) (while (< n 100000) (setq n (1+ n))) n)"), "100000");
}
