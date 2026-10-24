use romacs_vm::Vm;

#[test]
fn bare_vm_requires_gnu_lisp_for_generate_new_buffer() {
    let mut vm = Vm::new();
    let actual = vm.eval_source("(fboundp 'generate-new-buffer)").unwrap().display().unwrap();
    assert_eq!(actual, "nil", "GENERATE_NEW_BUFFER_MUST_COME_FROM_GNU_SUBR_EL");
    for name in ["generate-new-buffer-name", "get-buffer-create"] {
        let actual = vm.eval_source(&format!("(fboundp '{name})")).unwrap().display().unwrap();
        assert_eq!(actual, "t", "GENERATE_NEW_BUFFER_C_SUBSTRATE_REMAINS: {name}");
    }
}
