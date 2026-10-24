use romacs_vm::{Form, Vm};
use std::collections::BTreeSet;

#[test]
fn replay_gnu_32_known_answers_and_dump_every_form() {
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("../evidence/lang-known-answers.json")).unwrap();
    assert_eq!(evidence["version"], "32.0.50");
    let mut categories = BTreeSet::new();
    for row in evidence["cases"].as_array().unwrap() {
        categories.insert(row["form"].as_str().unwrap());
        let mut vm = Vm::new();
        let form = Form::from_json(row["transport"].clone()).unwrap();
        let function = vm
            .compile(&form)
            .unwrap_or_else(|e| panic!("{}: {e}", row["source"]));
        let dump = vm.dump_ir(function).unwrap();
        assert!(dump.contains("return r"), "{}", row["source"]);
        let result = vm.execute(function);
        if row["kind"] == "error" {
            assert_eq!(row["expected"], "void-variable");
            assert_eq!(
                result.unwrap_err(), "(void-variable i)",
                "{}",
                row["source"]
            );
        } else {
            let got = result.unwrap().display().unwrap();
            assert_eq!(got, row["expected"].as_str().unwrap(), "{}", row["source"]);
        }
    }
    assert_eq!(categories.len(), 20);
}

#[test]
fn lexical_binding_is_constant_by_a10_a11_and_the_owner_ruling() {
    for json in [
        serde_json::json!(["setq", "lexical-binding", "nil"]),
        serde_json::json!(["let", [["lexical-binding", "nil"]], "lexical-binding"]),
        serde_json::json!(["lambda", ["lexical-binding"], "lexical-binding"]),
    ] {
        let error = Vm::new()
            .compile(&Form::from_json(json).unwrap())
            .unwrap_err();
        assert!(error.contains("constant"));
    }
}

#[test]
fn newly_supported_generalized_places_compile_with_readable_setters() {
    for json in [
        serde_json::json!(["setf", ["car", "xs"], 2]),
        serde_json::json!(["push", 1, ["car", "xs"]]),
        serde_json::json!(["pop", ["car", "xs"]]),
        serde_json::json!(["cl-incf", ["car", "xs"]]),
    ] {
        let mut vm = Vm::new();
        let function = vm.compile(&Form::from_json(json).unwrap()).unwrap();
        assert!(vm.dump_ir(function).unwrap().contains("setcar"));
        // The original input intentionally has no xs binding: runtime lookup,
        // rather than compile-time place rejection, is now the correct failure.
        assert_eq!(vm.execute(function).unwrap_err(), "(void-variable xs)");
    }
}
