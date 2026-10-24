// A result cannot outlive the VM that produced it.
fn result() -> romacs_vm::Local<'static> {
    let mut vm = romacs_vm::Vm::new();
    vm.eval_source("(list 1 2 3)").unwrap()
}
fn main() {
    let _ = result();
}
