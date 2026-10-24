use romacs_vm::Vm;
#[test]
fn localizing_inside_plain_let_does_not_convert_its_binding_kind() {
    let mut vm=Vm::new();
    let result=vm.eval_source("(defvar s124-review-x 'outer) (list (let ((s124-review-x 'inner)) (make-variable-buffer-local 's124-review-x) (set 's124-review-x 'local)) s124-review-x (with-temp-buffer s124-review-x))").and_then(|v|v.display());
    assert_eq!(result,Ok("(local local outer)".into()));
}
