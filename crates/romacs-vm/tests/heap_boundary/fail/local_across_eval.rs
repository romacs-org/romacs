// A result held across another evaluation: the second call may compact the
// heap, so the first result must not be usable after it.
fn main() {
    let mut vm = romacs_vm::Vm::new();
    let list = vm.eval_source("(list 1 2 3)").unwrap();
    vm.eval_source("(make-list 100 'x)").unwrap();
    println!("{}", list.display().unwrap());
}
