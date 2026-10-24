//! Slot reads retain the live redirect and moving-heap value, not a cached Value.
use super::*;

fn text(vm: &mut Vm, source: &str) -> String {
    let value = vm.eval_source_raw(source).unwrap();
    vm.display(value).unwrap()
}

#[test]
fn plain_global_read_bypasses_cold_dispatch() {
    let mut vm = Vm::new();
    let slot = vm.slot("global-hot-value");
    let inst = lower(&vm, Op::Global(0, slot));
    assert!(!std::ptr::fn_addr_eq(inst.handler, cold as Handler),
        "plain slot reads must not enter the cold opcode match");
}

#[test]
fn warmed_global_reads_observe_localization_and_live_values() {
    let mut vm = Vm::new();
    assert_eq!(text(&mut vm, "(defvar global-hot-value 17) (defun global-hot-read () global-hot-value) (global-hot-read) (global-hot-read)"), "17");
    assert_eq!(text(&mut vm, "(let ((global-hot-value 23)) (global-hot-read))"), "23", "dynamic slot value");
    assert_eq!(text(&mut vm, "(set-buffer (get-buffer-create \"global-hot-A\")) (make-local-variable 'global-hot-value) (setq global-hot-value 31) (global-hot-read)"), "31", "localization after lowering");
    assert_eq!(text(&mut vm, "(set-buffer (get-buffer-create \"global-hot-B\")) (global-hot-read)"), "17", "buffer-qualified redirect");
    assert_eq!(text(&mut vm, "(set-buffer (get-buffer-create \"global-hot-A\")) (global-hot-read)"), "31");
}

#[test]
fn warmed_global_reads_forward_heap_values_and_signal_unbound() {
    let mut vm = Vm::new();
    text(&mut vm, "(list 'dead) (defvar global-hot-heap (list 1 2)) (defun global-hot-heap-read () global-hot-heap) (global-hot-heap-read)");
    let before = vm.heap_stats().collections;
    vm.collect(&mut []);
    assert!(vm.heap_stats().collections > before);
    assert_eq!(text(&mut vm, "(global-hot-heap-read)"), "(1 2)", "forwarded live slot");
    let caught = vm.intern("caught");
    assert_eq!(vm.eval_source_raw("(makunbound 'global-hot-heap) (condition-case nil (global-hot-heap-read) (void-variable 'caught))"), Ok(caught), "unbound goes through the signal path");
}
