use romacs_vm::{Form, Vm};
use serde_json::json;

#[test]
fn replay_entire_observed_gnu_condition_catalog() {
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("../evidence/conditions-catalog.json")).unwrap();
    assert_eq!(evidence["version"], "32.0.50");
    assert_eq!(evidence["conditions"].as_array().unwrap().len(), 96);
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    for row in evidence["conditions"].as_array().unwrap() {
        for (property, expected) in [
            (
                "error-conditions",
                format!(
                    "({})",
                    row[1]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap())
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
            ),
            (
                "error-message",
                serde_json::to_string(row[2].as_str().unwrap()).unwrap(),
            ),
        ] {
            let form = json!([
                "unwind-protect",
                ["get", ["quote", row[0]], ["quote", property]],
                "nil"
            ]);
            let entry = vm.compile(&Form::from_json(form).unwrap()).unwrap();
            let result = vm.execute(entry).and_then(|v| v.display());
            assert_eq!(result, Ok(expected), "{} / {property}", row[0]);
        }
    }
}

/// Romacs's own built-in condition, which GNU's catalog lacks, reads as
/// GNU's do: its ancestry and its message, found by its name.
#[test]
fn romacs_builtin_condition_has_its_conditions_and_message() {
    let mut vm = Vm::new();
    for (property, expected) in [
        ("error-conditions", "(not-unicode-string error)"),
        ("error-message", "\"String is not Unicode text\""),
    ] {
        let form = json!(["get", ["quote", "not-unicode-string"], ["quote", property]]);
        let entry = vm.compile(&Form::from_json(form).unwrap()).unwrap();
        assert_eq!(vm.execute(entry).and_then(romacs_vm::Local::display), Ok(expected.to_owned()), "{property}");
    }
}

#[test]
fn replay_live_gnu_conditions_with_moving_projection_graphs() {
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("../evidence/conditions-known-answers.json")).unwrap();
    assert_eq!(evidence["version"], "32.0.50");
    assert_eq!(evidence["regime"], "GNU byte-compiled / VM2 compiled");
    assert_eq!(evidence["cases"].as_array().unwrap().len(), 35);
    for row in evidence["cases"].as_array().unwrap() {
        for floor in [1, usize::MAX] {
            let mut vm = Vm::new();
            vm.set_collection_floor(floor);
            let form = json!([
                "unwind-protect",
                [
                    "progn",
                    [
                        "dotimes",
                        ["s49-garbage", 20],
                        ["cons", "s49-garbage", "nil"]
                    ],
                    row["transport"]
                ],
                "nil"
            ]);
            let entry = vm.compile(&Form::from_json(form).unwrap()).unwrap();
            let result = vm.execute(entry).and_then(|v| v.display());
            assert_eq!(
                result,
                Ok(row["expected"].as_str().unwrap().to_owned()),
                "{}: {}",
                row["name"],
                row["source"]
            );
            if floor == 1 {
                assert!(vm.heap_stats().collections > 0);
                assert!(vm.heap_stats().reclaimed > 0);
            }
        }
    }
}

#[test]
fn runtime_hierarchy_and_success_arm() {
    let mut vm = Vm::new();
    let form = Form::from_json(json!(["progn",
        ["define-error", ["quote", "s49-parent"], {"string": "Parent"}],
        ["define-error", ["quote", "s49-child"], {"string": "Child"}, ["quote", "s49-parent"]],
        ["vector",
            ["condition-case", "e", ["signal", ["quote", "s49-child"], "nil"], ["s49-parent", 7], ["error", 9]],
            ["condition-case", "e", 42, [":success", ["+", "e", 1]]]]])).unwrap();
    let entry = vm.compile(&form).unwrap();
    let result = vm.execute(entry).and_then(|v| v.display());
    assert_eq!(result, Ok("[7 43]".into()));

    // Replay the actual json.el load-order ancestry transition without claiming
    // that VM2 can load the GNU library. define-error is the supported substrate.
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("../evidence/conditions-catalog.json")).unwrap();
    let worlds = catalog["json_load_order"].as_array().unwrap();
    assert_eq!(worlds.len(), 2);
    assert_ne!(worlds[0], worlds[1]);
    for (index, ancestors) in worlds.iter().enumerate() {
        let ancestors = ancestors.as_array().unwrap();
        if index == 1 {
            let redefine = Form::from_json(json!([
                "define-error", ["quote", "json-end-of-file"],
                {"string": "End of file while parsing JSON"},
                ["quote", &ancestors[1..]]
            ]))
            .unwrap();
            vm.eval_form(&redefine).unwrap();
        }
        let projection = Form::from_json(json!([
            "get",
            ["quote", "json-end-of-file"],
            ["quote", "error-conditions"]
        ]))
        .unwrap();
        let expected = format!(
            "({})",
            ancestors
                .iter()
                .map(|a| a.as_str().unwrap())
                .collect::<Vec<_>>()
                .join(" ")
        );
        let actual = vm
            .eval_form(&projection)
            .and_then(|value| value.display());
        assert_eq!(actual, Ok(expected), "json ancestry world {index}");
        for parent in &ancestors[1..] {
            let handler = Form::from_json(json!([
                "condition-case",
                "nil",
                ["signal", ["quote", "json-end-of-file"], "nil"],
                [parent, 7],
                ["t", 9]
            ]))
            .unwrap();
            let actual = vm.eval_form(&handler).and_then(|value| value.display());
            assert_eq!(
                actual,
                Ok("7".into()),
                "json ancestry world {index}: {parent}"
            );
        }
    }
}
