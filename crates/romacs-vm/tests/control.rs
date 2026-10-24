use romacs_vm::{Form, Vm};
use serde_json::json;

#[test]
fn unwind_normal_completion_and_cleanup_effect() {
    let form = Form::from_json(json!([
        "let",
        [["x", 0]],
        ["vector", ["unwind-protect", 17, ["setq", "x", 9]], "x"]
    ]))
    .unwrap();
    let mut vm = Vm::new();
    let f = vm.compile(&form).unwrap();
    let outcome = vm.execute(f);
    assert_eq!(outcome.and_then(|v| v.display()), Ok("[17 9]".into()));
}

#[test]
fn replay_live_gnu_unwinds_with_collection_at_region_boundaries() {
    let evidence: serde_json::Value =
        serde_json::from_str(include_str!("../evidence/control-known-answers.json")).unwrap();
    assert_eq!(evidence["version"], "32.0.50");
    assert_eq!(evidence["regime"], "GNU byte-compiled / VM2 compiled");
    assert_eq!(evidence["cases"].as_array().unwrap().len(), 15);
    for row in evidence["cases"].as_array().unwrap() {
        for floor in [1, usize::MAX] {
            let mut vm = Vm::new();
            vm.set_collection_floor(floor);
            let form = json!([
                "progn",
                [
                    "dotimes",
                    ["s49-garbage", 20],
                    ["cons", "s49-garbage", "nil"]
                ],
                row["transport"]
            ]);
            let f = vm.compile(&Form::from_json(form).unwrap()).unwrap();
            let result = vm.execute(f).and_then(|v| v.display());
            assert_eq!(
                result,
                Ok(row["expected"].as_str().unwrap().to_owned()),
                "{} (floor={floor}): {}",
                row["name"],
                row["source"]
            );
            if floor == 1 {
                assert!(
                    vm.heap_stats().collections > 0,
                    "no forced collection: {}",
                    row["name"]
                );
                assert!(
                    vm.heap_stats().reclaimed > 0,
                    "no compaction: {}",
                    row["name"]
                );
            }
        }
    }
}
