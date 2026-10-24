// Positive control: the sealed API is usable. Results are read inside one
// borrow, Fixed values are kept across collections, and every evaluating
// entry point is reachable.
use romacs_vm::{Fixed, Vm};
fn main() {
    let mut vm = Vm::new();
    let kept = Fixed::integer(7).unwrap();
    vm.declare_special("hb-kept", kept);
    let text = vm.eval_source("(list 1 2 3)").unwrap().display().unwrap();
    assert_eq!(text, "(1 2 3)");
    vm.collect_garbage();
    assert_eq!(vm.eval_source("hb-kept").unwrap(), kept);
    let (car, cdr) = vm.eval_source("(cons 1 2)").unwrap().cons_pair().unwrap();
    assert_eq!((car.as_integer().unwrap(), cdr.as_integer().unwrap()), (1, 2));
    assert!(vm.eval_source("nil").unwrap().is_nil());
    let f = vm.compile(&romacs_vm::Form::Int(4)).unwrap();
    assert_eq!(vm.execute(f).unwrap().as_integer().unwrap(), 4);
    let read = vm.read_source("(a . b)").unwrap().map(|(v, end)| (v.display().unwrap(), end));
    assert_eq!(read, Some(("(a . b)".to_string(), 7)));
}
