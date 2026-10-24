use romacs_vm::Vm;
#[test]
fn minor_mode_map_alist_is_native_special_nil() {
    let mut vm = Vm::new();
    assert_eq!(vm.eval_source("(condition-case e (list (boundp 'minor-mode-map-alist) (special-variable-p 'minor-mode-map-alist) minor-mode-map-alist) (error e))").unwrap().display().unwrap(), "(t t nil)", "NATIVE_MINOR_MODE_MAP_ALIST");
    assert_eq!(vm.eval_source("(let ((minor-mode-map-alist '(sentinel))) (symbol-value 'minor-mode-map-alist))").unwrap().display().unwrap(), "(sentinel)", "NATIVE_MINOR_MODE_MAP_ALIST_DYNAMIC");
    assert_eq!(vm.eval_source("minor-mode-map-alist").unwrap().display().unwrap(), "nil");
}
