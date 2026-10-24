use romacs_vm::Vm;
#[test]
fn truncate_lines_is_buffer_local_when_set() {
    let mut vm = Vm::new();
    let result=vm.eval_source("(set-buffer (get-buffer-create \"s124-truncate-a\")) (setq truncate-lines t) (set-buffer (get-buffer-create \"s124-truncate-b\")) (list truncate-lines (progn (set-buffer (get-buffer-create \"s124-truncate-a\")) truncate-lines))").and_then(|v|v.display());
    assert_eq!(result,Ok("(nil t)".into()));
}
