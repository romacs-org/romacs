// A Local cannot be handed back into the VM: inputs are `Fixed` values or
// `Rooted` handles, never a borrowed result.
fn main() {
    let mut a = romacs_vm::Vm::new();
    let mut b = romacs_vm::Vm::new();
    let local = a.eval_source("(list 1)").unwrap();
    b.declare_special("foreign", local);
}
