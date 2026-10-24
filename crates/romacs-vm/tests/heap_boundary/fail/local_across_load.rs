// A car read out of a result, held across a nested load.
fn main() {
    let mut vm = romacs_vm::Vm::new();
    let (car, _) = vm.eval_source("(list (list 1))").unwrap().cons_pair().unwrap();
    vm.load_file(std::path::Path::new("/dev/null")).unwrap();
    println!("{}", car.display().unwrap());
}
