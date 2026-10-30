//! emacs.c Fkill_emacs: `kill-emacs-hook' runs as safe_run_hooks runs it,
//! then the session host ends the session with ARG's exit code (an integer
//! ARG's, else success). With no session host the VM refuses, so a library
//! embedder's process is never ended. romacs-vm-gui's batch_command_line.rs
//! compares the batch session's exit statuses with pinned GNU.
use romacs_vm::Vm;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// The exit code a session host received, by unwinding out of the VM.
fn exit_code(vm: &mut Vm, source: &str) -> i32 {
    let outcome = catch_unwind(AssertUnwindSafe(|| vm.eval_source(source).map(|_| ())));
    *outcome.expect_err("kill-emacs returned").downcast::<i32>().expect("session host exit code")
}

#[test]
fn kill_emacs_runs_its_hook_then_ends_the_session_with_args_code() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let refused = vm.eval_source("(condition-case e (kill-emacs 3) (error (car (cdr e))))").unwrap().display().unwrap();
    assert_eq!(refused, "\"VM2 kill-emacs without a session host is unsupported\"");
    vm.set_session_host(Some(Box::new(|code| std::panic::panic_any(code))));
    vm.eval_source("(setq t-ran nil kill-emacs-hook (list (lambda () (error \"hook failed\")) (lambda () (push 'second t-ran))))").unwrap();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let codes: Vec<i32> = ["(kill-emacs 3)", "(kill-emacs -1)", "(kill-emacs \"text\")", "(kill-emacs)"]
        .iter().map(|form| exit_code(&mut vm, form)).collect();
    std::panic::set_hook(previous);
    assert_eq!(codes, [3, -1, 0, 0]);
    // safe_run_hooks reports the failing function, drops it and runs the next.
    assert_eq!(vm.eval_source("(list (length kill-emacs-hook) t-ran)").unwrap().display().unwrap(), "(1 (second second second second))");
    let restart = vm.eval_source("(condition-case e (kill-emacs nil t) (error (car (cdr e))))").unwrap().display().unwrap();
    assert_eq!(restart, "\"VM2 kill-emacs RESTART is unsupported\"");
}
