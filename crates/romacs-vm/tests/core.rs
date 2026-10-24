use romacs_vm::{Form, Fixed, Vm};
use serde_json::json;

fn compile(vm: &mut Vm, json: serde_json::Value) -> usize {
    vm.compile(&Form::from_json(json).unwrap()).unwrap()
}
fn eval(vm: &mut Vm, json: serde_json::Value) -> String {
    let f = compile(vm, json);
    let v = vm.execute(f).unwrap();
    v.display().unwrap()
}

#[test]
fn same_call_site_invalidates_after_redefinition() {
    let mut vm = Vm::new();
    eval(&mut vm, json!(["defun", "target", [], 1]));
    let caller = compile(&mut vm, json!(["target"]));
    let first = vm.execute(caller).unwrap();
    assert_eq!(first.as_integer().unwrap(), 1);
    eval(&mut vm, json!(["defun", "target", [], 9]));
    let second = vm.execute(caller).unwrap();
    assert_eq!(second.as_integer().unwrap(), 9);
}

#[test]
fn primitive_cache_invalidates_too() {
    let mut vm = Vm::new();
    let call = compile(&mut vm, json!(["+", 9, 4]));
    assert_eq!(vm.execute(call).unwrap().as_integer().unwrap(), 13);
    eval(&mut vm, json!(["defun", "+", ["a", "b"], ["-", "a", "b"]]));
    assert_eq!(vm.execute(call).unwrap().as_integer().unwrap(), 5);
}

#[test]
fn dynamic_bindings_restore_on_failure() {
    let mut vm = Vm::new();
    vm.declare_special("s", Fixed::integer(3).unwrap());
    let bad = compile(&mut vm, json!(["let", [["s", 7]], ["+", "nil", 1]]));
    assert!(vm.execute(bad).is_err());
    assert_eq!(eval(&mut vm, json!("s")), "3");
    // The register stack and binding stack are reusable after an error.
    assert_eq!(eval(&mut vm, json!(["let", [["s", 8]], "s"])), "8");
    assert_eq!(eval(&mut vm, json!("s")), "3");
}

#[test]
fn recursion_error_unwinds_all_special_bindings() {
    let mut vm = Vm::new();
    vm.declare_special("s", Fixed::integer(3).unwrap());
    eval(
        &mut vm,
        json!(["defun", "loop", ["s"], ["loop", ["1+", "s"]]]),
    );
    let call = compile(&mut vm, json!(["loop", 0]));
    assert!(vm.execute(call).unwrap_err().contains("depth"));
    assert_eq!(eval(&mut vm, json!("s")), "3");
}

#[test]
fn nil_is_an_empty_binding_and_argument_list() {
    let mut vm = Vm::new();
    assert_eq!(
        eval(
            &mut vm,
            json!(["let", "nil", ["funcall", ["lambda", "nil", 42]]])
        ),
        "42"
    );
}

#[test]
fn ir_query_is_readable_stable_and_does_not_execute() {
    let mut vm = Vm::new();
    let code = compile(&mut vm, json!(["progn", ["setq", "q", 99], ["+", 20, 22]]));
    let dump = vm.dump_ir(code).unwrap();
    assert!(dump.contains("symbol s0 = +"));
    assert!(dump.contains("const 20"));
    assert!(dump.contains("cache c0"));
    assert_eq!(dump, vm.dump_ir(code).unwrap());
    let q = compile(&mut vm, json!("q"));
    assert!(vm.execute(q).is_err());
    assert_eq!(vm.execute(code).unwrap().as_integer().unwrap(), 42);
}

#[test]
fn boundary_fixnums_and_rejected_transport() {
    for n in [-(1i64 << 61), -1, 0, 1, (1i64 << 61) - 1] {
        assert_eq!(Fixed::integer(n).unwrap().as_integer().unwrap(), n);
    }
    assert!(Fixed::integer(1i64 << 61).is_err());
    assert!(Fixed::integer(-(1i64 << 61) - 1).is_err());
    assert!(Form::from_json(json!({"not": "a form"})).is_err());
    assert!(matches!(Form::from_json(json!(1.5)).unwrap(), Form::Float(n) if n == 1.5));
}
