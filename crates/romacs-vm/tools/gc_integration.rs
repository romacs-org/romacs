//! Integration gate for the combined moving collector and language runtime.
//! Run via check_gc_integration.py in both dev and optimized profiles.
use romacs_vm::{Form, Vm};
use serde_json::{Value, json};

fn compile(vm: &mut Vm, json: Value) -> usize {
    vm.compile(&Form::from_json(json).unwrap()).unwrap()
}
fn main() {
    let expected: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let expected_stress: i64 = std::env::args().nth(2).unwrap().parse().unwrap();
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    // The dead prefix forces relocation; do not assume any private index stays put.
    let make = compile(
        &mut vm,
        json!(["progn",
            ["vector", ["cons", -1, "nil"], ["copy-sequence", {"string":"garbage"}]],
            ["vector", ["cons", 42, "nil"], ["copy-sequence", {"string":"abc"}],
                ["vector", ["cons", 7, "nil"]]]
        ]),
    );
    let mut roots = [vm.execute(make).unwrap()];
    assert_eq!(vm.heap_stats().allocations, 10, "exact cons/string/vector allocation accounting");
    vm.collect(&mut roots);
    vm.declare_special("held", roots[0]);
    let stress = compile(
        &mut vm,
        json!(["let", [["tag", ["progn", ["let", [["j", 0]], ["while", ["<", "j", 100], ["vector", "j"], ["setq", "j", ["1+", "j"]]]], ["vector", 99]]], ["i", 0]],
            ["catch", "tag",
                ["condition-case", "e", ["progn",
                    ["while", ["<", "i", 200_000],
                        ["vector", ["cons", "i", "nil"], ["copy-sequence", {"string":"abc"}]],
                        ["setq", "i", ["1+", "i"]]],
                    ["signal", ["quote", "error"], ["quote", [42]]]],
                    [["error"], ["throw", "tag", ["car", ["cdr", "e"]]]]]]
        ]),
    );
    assert_eq!(
        vm.execute_rooted(stress, &mut roots)
            .unwrap()
            .as_integer()
            .unwrap(),
        expected_stress
    );
    assert!(
        vm.heap_stats().collections > 2,
        "automatic collection did not run"
    );
    assert!(
        vm.heap_stats().allocations >= 600_000,
        "new object kinds missing from accounting"
    );
    vm.collect(&mut roots);
    vm.declare_special("held", roots[0]);
    let inspect = compile(
        &mut vm,
        json!([
            "+",
            ["car", ["aref", "held", 0]],
            ["aref", ["aref", "held", 1], 1],
            ["car", ["aref", ["aref", "held", 2], 0]]
        ]),
    );
    assert_eq!(
        vm.execute_rooted(inspect, &mut roots)
            .unwrap()
            .as_integer()
            .unwrap(),
        expected
    );
    // Re-execute old lowered code after compaction. Literal and persistent entry
    // cache copies must be rewritten too, not only active register roots.
    assert_eq!(
        vm.execute_rooted(inspect, &mut roots)
            .unwrap()
            .as_integer()
            .unwrap(),
        expected
    );
    let drop_global = compile(&mut vm, json!(["setq", "held", "nil"]));
    vm.execute_rooted(drop_global, &mut roots).unwrap();
    vm.collect(&mut []);
    assert!(
        vm.heap_objects() < 64,
        "dead string/vector/cons objects remain"
    );
    assert!(
        vm.heap_stats().reclaimed > 599_900,
        "new arenas were not reclaimed"
    );
    assert_eq!(vm.heap_stats().allocations, 600_115);
    assert_eq!(vm.heap_stats().reclaimed, 600_110);
    // Five compiler-owned literals remain: two strings in make and a string,
    // selector list and signal-data list in stress. Compiled functions are
    // retained for their VM lifetime, and therefore deliberately root literals.
    assert_eq!(vm.heap_objects(), 5);
    eprintln!("allocations={} reclaimed={} live={} collections={}", vm.heap_stats().allocations, vm.heap_stats().reclaimed, vm.heap_objects(), vm.heap_stats().collections);
    println!("{}", expected);
}
