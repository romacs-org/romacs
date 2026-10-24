use romacs_vm::Vm;
#[test]
fn generate_buffer_name_target() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let result = vm.eval_source("(generate-new-buffer-name \"gnbn-target-free\")")
        .map(|v| v.display().unwrap()).unwrap_or_else(|e| e.to_string());
    assert_eq!(result, "\"gnbn-target-free\"", "generate-new-buffer-name TARGET free name");
}
