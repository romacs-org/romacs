use romacs_vm::Vm;
fn answer(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) { Ok(value) => value.display().unwrap(), Err(e) => format!("FAULT {e}") }
}
#[test]
fn leaf_non_special_clears_declaration() {
    let mut vm=Vm::new();
    assert_eq!(answer(&mut vm,"(progn (defvar leaf-special 7) (internal-make-var-non-special 'leaf-special) (list (special-variable-p 'leaf-special) leaf-special))"),"(nil 7)","LEAF_NON_SPECIAL_CLEAR");
    assert_eq!(answer(&mut vm,"(let ((s (make-symbol \"leaf-special\"))) (internal--define-uninitialized-variable s) (internal-make-var-non-special s) (special-variable-p s))"),"nil","LEAF_NON_SPECIAL_IDENTITY");
    assert_eq!(answer(&mut vm,"(condition-case e (internal-make-var-non-special 3) (error e))"),"(wrong-type-argument symbolp 3)","LEAF_NON_SPECIAL_TYPE");
}
#[test]
fn leaf_debug_variable_default_and_binding() {
    let mut vm=Vm::new();
    assert_eq!(answer(&mut vm,"(list (boundp 'debug-on-error) debug-on-error (special-variable-p 'debug-on-error))"),"(t nil t)","LEAF_DEBUG_DEFAULT");
    vm.eval_source("(defun leaf-debug-read () debug-on-error)").unwrap();
    assert_eq!(answer(&mut vm,"(list (let ((debug-on-error '(error))) (leaf-debug-read)) debug-on-error)"),"((error) nil)","LEAF_DEBUG_BINDING");
}
#[test]
fn leaf_float_format_default_and_binding() {
    let mut vm=Vm::new();
    assert_eq!(answer(&mut vm,"(list (boundp 'float-output-format) float-output-format (special-variable-p 'float-output-format))"),"(t nil t)","LEAF_FLOAT_DEFAULT");
    vm.eval_source("(defun leaf-float-read () float-output-format)").unwrap();
    assert_eq!(answer(&mut vm,"(list (let ((float-output-format \"%.2f\")) (leaf-float-read)) float-output-format)"),"(\"%.2f\" nil)","LEAF_FLOAT_BINDING");
}
