//! emacs.c Fkill_emacs: `kill-emacs-hook' runs as safe_run_hooks runs it,
//! then the session host ends the session with ARG's exit code (an integer
//! ARG's, else success). With no session host the VM refuses, so a library
//! embedder's process is never ended. romacs-vm-gui's batch_command_line.rs
//! compares the batch session's exit statuses with pinned GNU.
use romacs_vm::Vm;

/// The exit code a session host received, by unwinding out of the VM. These
/// tests go on using the VM, which no session does (`Vm::catching`).
fn exit_code(vm: &mut Vm, source: &str) -> i32 {
    let outcome = vm.catching(|vm| vm.eval_source(source).map(|_| ()));
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

/// shut_down_emacs's order: `kill-emacs-hook', then Fdo_auto_save, then
/// unlock_all_files, so the hook and `auto-save-hook' both see the lock a
/// modified file buffer holds and the session ends with it gone. (The bare
/// VM has no `utf-8-emacs', so the buffer has no auto-save file to write;
/// command_errors_live.py compares the written auto-save with pinned GNU.)
#[test]
fn kill_emacs_auto_saves_then_unlocks_modified_files() {
    let root = std::env::temp_dir().join(format!("romacs-kill-emacs-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let (file, lock) = (root.join("target"), root.join(".#target"));
    let q = |p: &std::path::Path| format!("{:?}", p.to_str().unwrap());
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_session_host(Some(Box::new(|code| std::panic::panic_any(code))));
    vm.eval_source(&format!(
        "(progn (defun make-lock-file-name (f) {lock}) (write-region \"\" nil {file} nil t)
           (setq buffer-file-truename buffer-file-name buffer-auto-save-file-name nil auto-save-list-file-name nil)
           (insert \"text\") (lock-buffer)
           (setq t-seen nil kill-emacs-hook (list (lambda () (push (list 'hook (file-locked-p {file})) t-seen)))
                 auto-save-hook (list (lambda () (push (list 'auto-save (file-locked-p {file})) t-seen)))))",
        lock = q(&lock), file = q(&file))).unwrap();
    assert!(std::fs::read_link(&lock).is_ok(), "the modified buffer holds its lock");
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let code = exit_code(&mut vm, "(kill-emacs 4)");
    std::panic::set_hook(previous);
    assert_eq!(code, 4);
    assert_eq!(vm.eval_source("(reverse t-seen)").unwrap().display().unwrap(), "((hook t) (auto-save t))");
    assert!(std::fs::symlink_metadata(&lock).is_err(), "unlock_all_files removed the lock");
}
