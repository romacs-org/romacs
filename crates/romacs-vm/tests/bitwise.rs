use romacs_vm::Vm;
use sha2::{Digest, Sha256};
const CASES: &str = include_str!("../tools/primh/bitwise/cases.json");
const GNU: &str = include_str!("../tools/primh/bitwise/gnu.json");
const RECORDER: &str = include_str!("../tools/primh/bitwise/oracle.py");
fn digest(s: &str) -> String {
    Sha256::digest(s)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
#[test]
fn bitwise_matches_pinned_gnu() {
    let record: serde_json::Value = serde_json::from_str(GNU).unwrap();
    let specs: serde_json::Value = serde_json::from_str(CASES).unwrap();
    assert_eq!(
        record["oracle"]["revision"],
        "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea"
    );
    assert_eq!(record["cases_sha256"], digest(CASES));
    assert_eq!(record["recorder_sha256"], digest(RECORDER));
    let rows = record["cases"].as_array().unwrap();
    assert_eq!(rows.len(), specs.as_array().unwrap().len());
    assert!(rows.len() >= 20);
    let mut failures = vec![];
    for (row, spec) in rows.iter().zip(specs.as_array().unwrap()) {
        assert_eq!(row["expression"], spec["expression"]);
        assert_eq!(
            row["form"],
            format!(
                "(condition-case e {} (error e))",
                spec["expression"].as_str().unwrap()
            )
        );
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let actual = vm
            .eval_source(row["form"].as_str().unwrap())
            .and_then(|v| v.display());
        let expected = row["gnu"].as_str().unwrap();
        if actual != Ok(expected.to_owned()) {
            failures.push(format!("{}: GNU {expected}; ours {actual:?}", row["id"]));
        }
    }
    assert!(
        failures.is_empty(),
        "BITWISE_GNU_MISMATCH\n{}",
        failures.join("\n")
    );
}

#[test]
fn logior_result_survives_moving_collection() {
    let record: serde_json::Value = serde_json::from_str(GNU).unwrap();
    let row = record["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "logior-5")
        .unwrap();
    let expected = row["gnu"].as_str().unwrap();
    let mut vm = Vm::new();
    vm.set_collection_floor(1_000_000);
    vm.eval_source("(vector (make-string 1000 65))").unwrap();
    let local = vm.eval_source(row["expression"].as_str().unwrap()).unwrap();
    let old = format!("{local:?}");
    let root = local.root();
    let before = vm.heap_stats().collections;
    vm.collect_garbage();
    assert!(
        vm.heap_stats().collections > before,
        "LOGIOR_GC_NOT_EXERCISED"
    );
    let local = root.get(&vm).unwrap();
    assert_ne!(format!("{local:?}"), old, "LOGIOR_GC_NOT_MOVED");
    assert_eq!(
        local.display().unwrap(),
        expected,
        "LOGIOR_GC_RESULT_MISMATCH"
    );
}
