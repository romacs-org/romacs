// A Local cannot be re-pointed at another VM (or built at all): its fields
// are private, even through struct-update syntax.
fn main() {
    let mut a = romacs_vm::Vm::new();
    let b = romacs_vm::Vm::new();
    let local = a.eval_source("(list 1)").unwrap();
    let _ = romacs_vm::Local { vm: &b, ..local };
}
