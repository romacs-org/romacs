use romacs_vm::Vm;
use sha2::{Digest, Sha256};
const CASES: &str = include_str!("../evidence/head-order/cases.json");
const GNU: &str = include_str!("../evidence/head-order/gnu.json");
const RECORDER: &str = include_str!("../evidence/head-order/oracle.py");
fn digest(s: &str) -> String {
    Sha256::digest(s)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
#[test]
fn head_order_matches_pinned_gnu() {
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
    assert!(rows.len() >= 18);
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
        if row["id"] == "gc-snapshot" {
            assert!(vm.heap_stats().collections > 0, "HEAD_ORDER_GC_NOT_EXERCISED");
        }
        let expected = row["gnu"].as_str().unwrap();
        if actual != Ok(expected.to_owned()) {
            failures.push(format!("{}: GNU {expected}; ours {actual:?}", row["id"]));
        }
    }
    assert!(
        failures.is_empty(),
        "HEAD_ORDER_GNU_MISMATCH\n{}",
        failures.join("\n")
    );
}
