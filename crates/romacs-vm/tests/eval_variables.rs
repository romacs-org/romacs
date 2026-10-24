use romacs_vm::Vm;
fn answer(vm: &mut Vm, source: &str) -> Result<String, String> {
    vm.eval_source(source).and_then(|v| v.display())
}
#[test]
fn fboundp_checks_function_cells_without_calling_them() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(list (fboundp 'car) (fboundp 'missing-s124) (fboundp nil) (fboundp t))"), Ok("(t nil nil nil)".into()));
    assert!(vm.eval_source("(fboundp 3)").is_err());
}
#[test]
fn set_uses_symbol_value_not_lexical_binding() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(setq x 9) (let ((x 2)) (list (set 'x (list 'changed)) x (symbol-value 'x)))"), Ok("((changed) 2 (changed))".into()));
    assert!(vm.eval_source("(set t 9)").is_err());
}
#[test]
fn makunbound_voids_dynamic_binding_then_unwinds() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(defvar s124-special 8) (list (let ((s124-special 3)) (makunbound 's124-special) (boundp 's124-special)) s124-special)"), Ok("(nil 8)".into()));
}
#[test]
fn explicit_local_only_affects_current_buffer_and_keeps_void_local() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(setq local-s124 11) (set-buffer (get-buffer-create \"s124-local-a\")) (make-local-variable 'local-s124) (set 'local-s124 (list 'local)) (set-buffer (get-buffer-create \"s124-local-b\")) (set 'local-s124 19) (set-buffer (get-buffer-create \"s124-local-a\")) (list local-s124 (progn (makunbound 'local-s124) (boundp 'local-s124)))"), Ok("((local) nil)".into()));
    assert_eq!(answer(&mut vm, "(set-buffer (get-buffer-create \"s124-local-b\")) local-s124"), Ok("19".into()));
    assert_eq!(answer(&mut vm, "(set-buffer (get-buffer-create \"s124-local-c\")) local-s124"), Ok("19".into()));
}
#[test]
fn local_values_survive_counted_collection() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(set-buffer (get-buffer-create \"s124-gc\")) (make-local-variable 's124-local-root) (set 's124-local-root (list (copy-sequence \"kept\"))) (car s124-local-root)"), Ok("\"kept\"".into()));
    let before = vm.heap_stats().collections;
    vm.collect_garbage();
    assert!(vm.heap_stats().collections > before);
    assert_eq!(answer(&mut vm, "s124-local-root"), Ok("(\"kept\")".into()));
}
#[test]
fn ordinary_startup_variables_are_bound_without_fake_functions() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(list menu-bar-lines truncate-lines)"), Ok("(0 nil)".into()));
}
