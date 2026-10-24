//! Evaluator-only calibration: no JSON adapter and no inherited runtime.
//! This is not a source cache. Prepared mode retains one VM2 compiled form.
use romacs_vm::Vm;
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let mode = &args[0];
    let count: usize = args[1].parse().unwrap();
    let expected: i64 = args[2].parse().unwrap();
    let source = &args[3];
    let mut vm = Vm::new();
    let function = if mode == "prepared" {
        Some(vm.compile_source(source).unwrap())
    } else {
        None
    };
    let handle = if mode == "handle" { Some(vm.prepare_source(&format!("(lambda () {source})")).unwrap()) } else { None };
    for _ in 0..count {
        if mode == "cold" { vm.clear_source_cache(); }
        let value = if let Some(handle) = handle { vm.invoke_prepared(handle, &[]).unwrap() } else if let Some(function) = function {
            vm.execute(function).unwrap()
        } else {
            vm.eval_source(source).unwrap()
        };
        assert_eq!(value.as_integer().unwrap(), expected);
    }
    println!("{expected}");
}
