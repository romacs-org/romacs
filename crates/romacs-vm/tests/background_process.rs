use romacs_vm::Vm;
use std::time::{Duration, Instant};

fn printed(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).unwrap().display().unwrap()
}

#[test]
fn jobs_cross_the_lisp_boundary_without_blocking_or_repeating_callbacks() {
    let mut vm = Vm::new();
    vm.eval_source(include_str!("../../../lisp/romacs/vm/process.el")).unwrap();
    vm.eval_source(r#"(setq completed nil)
      (setq job (romacs-process-start '("/bin/sh" "-c" "cat; printf ERR >&2; exit 7")
        "hello λ" "/" 2 (lambda (output) (setq completed output) nil)))"#).unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    while printed(&mut vm, "completed") == "nil" {
        vm.eval_source("(romacs-process-dispatch-ready)").unwrap();
        assert!(Instant::now() < deadline, "LISP_PROCESS_COMPLETION");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(printed(&mut vm, "completed"), "(7 \"hello λ\" \"ERR\" nil)");
    assert_eq!(printed(&mut vm, "(list romacs-process-handlers (romacs-process-dispatch-ready))"), "(nil nil)");
}

#[test]
fn a_quitting_callback_does_not_strand_other_completed_jobs() {
    let mut vm = Vm::new();
    vm.eval_source(include_str!("../../../lisp/romacs/vm/process.el")).unwrap();
    vm.eval_source(r#"(setq delivered nil)
      (setq first (romacs-process-start '("/bin/true") "" "/" 2
                    (lambda (_) (signal 'quit nil))))
      (setq second (romacs-process-start '("/bin/true") "" "/" 2
                    (lambda (_) (setq delivered 'second) nil)))"#).unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    while printed(&mut vm, "(and (romacs--process-result first) (romacs--process-result second))") == "nil" {
        assert!(Instant::now() < deadline, "BOTH_JOBS_MUST_COMPLETE_BEFORE_DISPATCH");
        std::thread::sleep(Duration::from_millis(5));
    }
    vm.eval_source("(setq quit-seen (condition-case nil (progn (romacs-process-dispatch-ready) nil) (quit t)))").unwrap();
    vm.eval_source("(romacs-process-dispatch-ready)").unwrap();
    assert_eq!(printed(&mut vm, "(list delivered romacs-process-handlers quit-seen)"),
               "(second nil t)", "CALLBACK_QUIT_CANNOT_STRAND_ACKNOWLEDGED_COMPLETIONS");
}

#[test]
fn json_is_plain_data_and_distinguishes_false_and_null() {
    let mut vm = Vm::new();
    assert_eq!(printed(&mut vm, r#"(romacs-json-read "{\"items\":[1,true,false,null,\"λ\"]}")"#),
        "((\"items\" 1 t json-false json-null \"λ\"))");
    assert!(vm.eval_source(r#"(romacs-json-read "[1]garbage")"#).is_err());
}
