// A result held across an explicit collection.
fn main() {
    let mut vm = romacs_vm::Vm::new();
    let list = vm.eval_source("(list 1 2 3)").unwrap();
    vm.collect_garbage();
    println!("{}", list.display().unwrap());
}
