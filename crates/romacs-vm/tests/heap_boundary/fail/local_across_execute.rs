// A result held across executing compiled code, which polls and can collect
// mid-request.
fn main() {
    let mut vm = romacs_vm::Vm::new();
    let f = vm.compile(&romacs_vm::Form::Int(1)).unwrap();
    let list = vm.eval_source("(list 1 2 3)").unwrap();
    vm.execute(f).unwrap();
    println!("{}", list.display().unwrap());
}
