use romacs_vm::Vm;
#[test]
fn generated_buffers_are_named_and_unique_and_scratch_is_lazy() {
    let mut vm=Vm::new();
    let result=vm.eval_source("(list (eq (get-buffer \"*scratch*\") (current-buffer)) (let ((a (get-buffer-create (generate-new-buffer-name \"s124-generated\"))) (b (get-buffer-create (generate-new-buffer-name \"s124-generated\")))) (list (eq a (get-buffer \"s124-generated\")) (eq b (get-buffer \"s124-generated<2>\")) (eq a b))))").and_then(|v| v.display());
    assert_eq!(result,Ok("(t (t t nil))".into()));
}
