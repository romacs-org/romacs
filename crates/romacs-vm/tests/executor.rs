use romacs_vm::{Form, Vm};
use serde_json::{Value, json};

fn run(form: Value) -> i64 {
    let mut vm = Vm::new();
    let function = vm.compile(&Form::from_json(form).unwrap()).unwrap();
    vm.execute(function).unwrap().as_integer().unwrap()
}

#[test]
fn warm_named_and_primitive_sites_observe_in_execution_redefinition() {
    assert_eq!(
        run(json!([
            "progn",
            ["defun", "target", [], 1],
            ["defun", "caller", [], ["target"]],
            [
                "let",
                [["i", 0], ["s", 0]],
                [
                    "while",
                    ["<", "i", 4],
                    ["if", ["=", "i", 2], ["defun", "target", [], 9]],
                    ["setq", "s", ["+", "s", ["caller"]], "i", ["1+", "i"]]
                ],
                "s"
            ]
        ])),
        20
    );
    assert_eq!(
        run(json!([
            "progn",
            ["defun", "caller", [], ["+", 9, 4]],
            [
                "let",
                [["i", 0], ["s", 0], ["before", 0]],
                [
                    "while",
                    ["<", "i", 2],
                    ["setq", "s", ["caller"]],
                    [
                        "if",
                        ["=", "i", 0],
                        [
                            "progn",
                            ["setq", "before", "s"],
                            ["defun", "+", ["a", "b"], ["-", "a", "b"]]
                        ]
                    ],
                    ["setq", "i", ["1+", "i"]]
                ],
                ["*", "before", "s"]
            ]
        ])),
        65
    );
}

#[test]
fn funcall_cache_handles_polymorphic_targets() {
    assert_eq!(
        run(json!([
            "let",
            [
                ["f", ["lambda", ["x"], "x"]],
                ["g", ["lambda", ["x"], ["1+", "x"]]],
                ["i", 0],
                ["s", 0]
            ],
            [
                "while",
                ["<", "i", 4],
                ["if", ["=", "i", 2], ["setq", "f", "g"]],
                ["setq", "s", ["funcall", "f", "i"], "i", ["1+", "i"]]
            ],
            "s"
        ])),
        4
    );
}

#[test]
fn large_captures_and_argument_windows_use_cold_entry() {
    assert_eq!(
        run(json!([
            "let",
            [["a", 1], ["b", 2], ["c", 3]],
            [
                "funcall",
                [
                    "lambda",
                    ["x", "y", "z"],
                    ["+", "a", "b", "c", "x", "y", "z"]
                ],
                4,
                5,
                6
            ]
        ])),
        21
    );
}

#[test]
fn tagged_arithmetic_checks_both_sign_boundaries() {
    let lo = -(1i64 << 61);
    let hi = (1i64 << 61) - 1;
    for (form, expected) in [
        (json!(["+", lo, hi]), -1),
        (json!(["-", lo, lo]), 0),
        (json!(["*", lo, 1]), lo),
        (json!(["1+", lo]), lo + 1),
        (json!(["1-", hi]), hi - 1),
        (json!(["if", ["<", lo, hi], 1, 0]), 1),
    ] {
        assert_eq!(run(form), expected);
    }
    for (form, expected) in [
        (json!(["+", hi, 1]), "2305843009213693952"),
        (json!(["+", lo, -1]), "-2305843009213693953"),
        (json!(["-", lo, 1]), "-2305843009213693953"),
        (json!(["-", hi, -1]), "2305843009213693952"),
        (json!(["*", lo, -1]), "2305843009213693952"),
        (json!(["*", hi, hi]), "5316911983139663487003542222693990401"),
        (json!(["1-", lo]), "-2305843009213693953"),
        (json!(["1+", hi]), "2305843009213693952"),
    ] {
        let mut vm = Vm::new();
        let context = form.to_string();
        let f = vm.compile(&Form::from_json(form).unwrap()).unwrap();
        let actual = vm.execute(f).unwrap().display().unwrap();
        assert_eq!(actual, expected, "compiled overflow form {context}");
    }
}

#[test]
fn long_hot_and_cold_handler_chains_have_bounded_native_stack() {
    // Covers both dev's driver and release sibling calls, with repeated generic
    // primitive calls, cell allocation, closure creation and the >2-arg entry.
    assert_eq!(
        run(json!([
            "let",
            [["i", 0], ["s", 0]],
            [
                "while",
                ["<", "i", 100_000],
                [
                    "setq",
                    "s",
                    [
                        "funcall",
                        ["lambda", ["a", "b", "c"], ["+", "a", "b", "c"]],
                        "i",
                        1,
                        2
                    ]
                ],
                ["setq", "i", ["1+", "i"]]
            ],
            "s"
        ])),
        100_002
    );
}

#[test]
fn foreign_arena_reference_cannot_bypass_entry_validation() {
    let mut owner = Vm::new();
    let f = owner
        .compile(&Form::from_json(json!(["lambda", [], 42])).unwrap())
        .unwrap();
    // The sealed boundary rejects the foreign reference before it can reach
    // entry validation; that validation is covered in-crate by
    // heap_boundary_tests::foreign_arena_reference_cannot_bypass_entry_validation.
    let closure = owner.execute(f).unwrap().root();
    let mut other = Vm::new();
    let error = other.declare_special_rooted("foreign", &closure).unwrap_err();
    assert!(error.contains("another VM"), "{error}");
}

#[test]
fn independent_vms_execute_on_parallel_threads() {
    let threads: Vec<_> = (0..4)
        .map(|n| {
            std::thread::spawn(move || {
                assert_eq!(
                    run(json!([
                        "let",
                        [["i", 0], ["s", n]],
                        [
                            "while",
                            ["<", "i", 10_000],
                            ["setq", "s", ["+", "s", 1], "i", ["1+", "i"]]
                        ],
                        "s"
                    ])),
                    10_000 + n
                );
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}
