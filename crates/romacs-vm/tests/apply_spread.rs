//! `apply` spreads its arguments into a scratch window at the end of the
//! register storage. A callee entered after the storage widens must read
//! them all, and the backtrace its call records, before writing its frame
//! over that window.
use romacs_vm::Vm;

#[test]
fn spread_arguments_survive_storage_growth_at_callee_entry() {
    let mut vm = Vm::new();
    vm.eval_source("(setq spread-target (let ((cap 1000)) (lambda (&rest r) (setq spread-capture (lambda () cap)) (setq spread-trace (backtrace-frame--internal (lambda (_evald _f args _flags) args) 0 spread-target)) (setq spread-result r))))").unwrap();
    vm.eval_source("(setq spread-hook '(apply)) (setq spread-args (cons 'spread-hook (cons spread-target '(777 777 777 777 777 777 777 777 777 777 777 777 777 777 nil))))").unwrap();
    vm.eval_source("(defun spread-recur (n) (if (= n 0) (apply 'run-hook-with-args spread-args) (spread-recur (1- n))))").unwrap();
    for depth in 0..40 {
        vm.eval_source(&format!("(spread-recur {depth})")).unwrap();
        // The arguments, and the backtrace's record of them.
        let first = vm.eval_source("(list (car spread-result) (length spread-result) (car spread-trace) (length spread-trace))").unwrap().display().unwrap();
        assert_eq!(first, "(777 14 777 14)", "depth {depth}");
    }
}
