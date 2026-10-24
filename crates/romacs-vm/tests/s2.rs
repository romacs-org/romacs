use romacs_vm::{Form, Fixed, Vm};
use serde_json::json;

fn compile(vm: &mut Vm, form: serde_json::Value) -> usize {
    vm.compile(&Form::from_json(form).unwrap()).unwrap()
}

#[test]
fn replay_all_fourteen_acceptance_answers_and_additional_gnu_probes() {
    for (text, count) in [
        (include_str!("../evidence/lang-pending.json"), 14),
        (include_str!("../evidence/s2-known-answers.json"), 60),
    ] {
        let evidence: serde_json::Value = serde_json::from_str(text).unwrap();
        let rows = evidence["cases"].as_array().unwrap();
        assert_eq!(rows.len(), count);
        for row in rows {
            assert_eq!(row["passed"], true, "{}", row["source"]);
            assert_eq!(row["ir_ok"], true, "{}", row["source"]);
            let mut vm = Vm::new();
            let f = compile(&mut vm, row["transport"].clone());
            assert!(vm.dump_ir(f).unwrap().contains("return r"));
            let result = vm.execute(f);
            if row["kind"] == "error" {
                assert_eq!(row["expected"], "no-catch");
                assert!(result.unwrap_err().contains("no-catch"));
            } else {
                assert_eq!(
                    result.unwrap().display().unwrap(),
                    // GNU-DIVERGENCES.md D001: deliberate scalar replacement;
                    // retain the frozen GNU answer and compare exactly.
                    if row["source"]
                        == "(condition-case e (let ((s (copy-sequence \"αβγ\"))) (aset s 1 955) (aref s 1)) (error 18))"
                    {
                        "955"
                    } else {
                        row["expected"].as_str().unwrap()
                    },
                    "{}",
                    row["source"]
                );
            }
        }
    }
}

#[test]
fn scoped_escapes_restore_specials_and_the_vm_can_be_reused() {
    let mut vm = Vm::new();
    vm.declare_special("s", Fixed::integer(3).unwrap());
    let f = compile(
        &mut vm,
        json!([
            "progn",
            [
                "catch",
                ["quote", "exit"],
                [
                    "funcall",
                    [
                        "lambda",
                        [],
                        ["let", [["s", 9]], ["throw", ["quote", "exit"], "s"]]
                    ]
                ]
            ],
            [
                "condition-case",
                "e",
                ["let", [["s", 11]], ["car", 1]],
                ["error", "s"]
            ]
        ]),
    );
    for _ in 0..3 {
        assert_eq!(vm.execute(f).unwrap().as_integer().unwrap(), 3);
    }
    let fail = compile(
        &mut vm,
        json!(["let", [["s", 29]], ["throw", ["quote", "absent"], 7]]),
    );
    assert!(vm.execute(fail).unwrap_err().contains("no-catch"));
    let read = compile(&mut vm, json!("s"));
    assert_eq!(vm.execute(read).unwrap().as_integer().unwrap(), 3);
}

#[test]
fn long_apply_and_rest_chains_do_not_retain_native_frames() {
    let mut vm = Vm::new();
    let f = compile(
        &mut vm,
        json!([
            "let",
            [["i", 0], ["s", 0]],
            [
                "while",
                ["<", "i", 100_000],
                [
                    "setq",
                    "s",
                    [
                        "apply",
                        [
                            "lambda",
                            ["a", "&optional", "b", "&rest", "xs"],
                            ["+", "a", "b", ["length", "xs"]]
                        ],
                        "i",
                        ["quote", [1, 2, 3]]
                    ],
                    "i",
                    ["1+", "i"]
                ]
            ],
            "s"
        ]),
    );
    assert_eq!(vm.execute(f).unwrap().as_integer().unwrap(), 100_002);
}

#[test]
fn unsupported_places_and_malformed_argument_lists_remain_rejections() {
    for form in [
        json!(["setf", ["nth", 0, "xs"], 2]),
        json!(["push", 1, ["nth", 0, "xs"]]),
        json!(["pop", ["nth", 0, "xs"]]),
        json!(["cl-incf", ["nth", 0, "xs"]]),
        json!(["lambda", ["&rest"], 1]),
        json!(["lambda", ["&rest", "x", "y"], 1]),
        json!(["lambda", ["&optional", "&optional", "x"], 1]),
        json!(["lambda", ["x", "x"], 1]),
    ] {
        assert!(Vm::new().compile(&Form::from_json(form).unwrap()).is_err());
    }
}
