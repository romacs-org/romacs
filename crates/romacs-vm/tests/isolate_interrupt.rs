//! The engine's safepoint turns an isolate interrupt (SIGUSR1) into `quit'
//! inside CPU-bound Lisp, honouring `inhibit-quit', and a termination
//! (SIGTERM) into one `quit' whatever `inhibit-quit' says.
#[path = "support/own_test.rs"]
mod own_test;

use romacs_vm::Vm;
use rustix::process::{Signal, getpid, kill_process};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source)
        .and_then(|value| value.display())
        .unwrap_or_else(|error| format!("ERROR {error}"))
}

fn eval_with_signal(vm: &mut Vm, signal: Signal, delay_ms: u64, source: &str) -> String {
    let timeout = Duration::from_secs(3);
    let deadline = Instant::now() + timeout;
    let (done, finished) = channel::<()>();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            assert_eq!(
                finished.recv_timeout(Duration::from_millis(delay_ms)),
                Err(RecvTimeoutError::Timeout),
                "evaluation ended before sending {signal:?}: {source}"
            );
            if let Err(error) = kill_process(getpid(), signal) {
                eprintln!("could not send {signal:?}: {error}");
                std::process::exit(1);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if finished.recv_timeout(remaining) == Err(RecvTimeoutError::Timeout) {
                eprintln!("timed out after {timeout:?} waiting for {signal:?} to quit: {source}");
                // The evaluator may never poll again. Only this test's child exits.
                std::process::exit(1);
            }
        });
        let value = eval(vm, source);
        drop(done);
        value
    })
}

#[test]
fn sigusr1_quits_a_tight_loop_and_sigterm_quits_once() {
    const CHILD: &str = "ROMACS_ISOLATE_INTERRUPT_TEST_CHILD";
    const NAME: &str = "sigusr1_quits_a_tight_loop_and_sigterm_quits_once";
    if std::env::var_os(CHILD).is_none() {
        // Any VM polling in this process could take the signal first.
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &own_test::name(module_path!(), NAME), "--nocapture"])
            .env(CHILD, "1")
            .status()
            .expect("start isolated signal test");
        assert!(status.success(), "isolated signal test failed: {status}");
        return;
    }
    romacs_vm::host::catch_isolate_signals().unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    check_loop_quits(&mut vm);
    check_claimed_jobs_quit(&mut vm);
    check_runner_quits(&mut vm);
}

fn check_loop_quits(vm: &mut Vm) {
    assert_eq!(eval_with_signal(vm, Signal::USR1, 200, "(condition-case e (while t) (quit 'interrupted))"), "interrupted");
    assert_eq!(eval_with_signal(vm, Signal::USR1, 200, "(condition-case e (let ((n 0)) (while t (setq n (1+ n)))) (quit 'interrupted))"), "interrupted");
    // A lambda form evaluated at run time, as a worker runs a job.
    assert_eq!(eval_with_signal(vm, Signal::USR1, 200, "(condition-case e (funcall (eval '(lambda () (while t)) t)) (quit 'interrupted))"), "interrupted");
    assert_eq!(eval_with_signal(vm, Signal::USR1, 200, "(condition-case e (apply (eval '(lambda (x) (while t x)) t) (list 1)) (quit 'interrupted))"), "interrupted");
    // Held while inhibit-quit is bound, taken once it is not.
    assert_eq!(eval_with_signal(vm, Signal::USR1, 100, "(condition-case e (progn (let ((inhibit-quit t) (n 0)) (while (< n 2000000) (setq n (1+ n)))) (while t)) (quit 'interrupted))"), "interrupted");
}

fn check_claimed_jobs_quit(vm: &mut Vm) {
    // The worker's shape: the loop's condition-case around a running job.
    vm.eval_source("(defun journey-job () (condition-case err (progn (romacs--isolate-claim 1) (while t)) (quit 'caught-in-job)))").unwrap();
    assert_eq!(eval_with_signal(vm, Signal::USR1, 200, "(let ((n 0) r) (while (< n 1) (setq n (1+ n)) (setq r (journey-job))) r)"), "caught-in-job");
    for (name, body) in [("plain", "(set 'journey-var 1)"), ("env", "(set 'process-environment process-environment)"),
                         ("thaw", "(thaw (freeze (list 1 2)))"), ("exec", "(set 'exec-path (thaw (freeze exec-path)))"),
                         ("dir", "(set 'default-directory default-directory)"),
                         ("prepare", "(romacs--isolate-prepare (list (cons 'journey-seed (list 1 2))) nil)"),
                         ("real-seeds", "(romacs--isolate-prepare (thaw (freeze (romacs--isolate-seeds))) nil)"),
                         ("env-seed", "(set 'process-environment (thaw (freeze process-environment)))"),
                         ("load-path-seed", "(set 'load-path (thaw (freeze load-path)))")] {
        assert_eq!(eval_with_signal(vm, Signal::USR1, 200, &format!("(progn (romacs--isolate-claim 1) (condition-case e (while t {body}) (quit 'caught)))")), "caught", "{name}");
    }
}

fn check_runner_quits(vm: &mut Vm) {
    // The real job runner, whose reply fails harmlessly without a parent.
    vm.eval_source("(defun journey-loop () (while t))").unwrap();
    assert_eq!(eval_with_signal(vm, Signal::USR1, 200, "(condition-case e (list 'returned (romacs--isolate-run-job 1 (list 'journey-loop (freeze nil) (freeze nil) nil nil))) (quit 'escaped))"), "(returned nil)");
    assert_eq!(eval_with_signal(vm, Signal::USR1, 200, "(condition-case e (list 'returned (unwind-protect (condition-case err (progn (romacs--isolate-claim 1) (while t)) (quit 'caught)) nil)) (quit 'escaped))"), "(returned caught)");
    // SIGTERM: one quit, then `romacs--isolate-terminated-p' says so.
    assert_eq!(eval_with_signal(vm, Signal::TERM, 200, "(condition-case e (let ((inhibit-quit t)) (while t)) (quit (list 'terminated (romacs--isolate-terminated-p))))"), "(terminated t)");
    assert_eq!(eval(vm, "(let ((n 0)) (while (< n 100000) (setq n (1+ n))) n)"), "100000");
}
