//! An isolate's rule against threads overrides everything that asks for
//! them, for a zygote that holds one cannot fork. A process of its own: the
//! rule is for a whole process.
use romacs_vm::Vm;

fn collector_threads() -> usize {
    std::fs::read_dir("/proc/self/task").unwrap()
        .filter(|task| std::fs::read_to_string(task.as_ref().unwrap().path().join("comm")).is_ok_and(|name| name.starts_with("romacs-gc")))
        .count()
}

#[test]
fn a_process_that_may_fork_never_starts_collector_threads() {
    romacs_vm::forbid_gc_threads();
    romacs_vm::set_gc_workers(4);
    let mut vm = Vm::new();
    vm.set_gc_threads(4);
    vm.eval_source("(let ((i 0) (kept nil)) (while (< i 200000) (setq kept (cons i kept)) (setq i (1+ i))) (garbage-collect) (length kept))").unwrap();
    assert_eq!(collector_threads(), 0);
}
