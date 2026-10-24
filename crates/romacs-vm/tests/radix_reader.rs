//! Exact read-from-string values, end positions and signals recorded from GNU.
use romacs_vm::Vm;
use sha2::{Digest, Sha256};
const CASES: &str = include_str!("../evidence/radix-reader/cases.json");
const GNU: &str = include_str!("../evidence/radix-reader/gnu.json");
const ORACLE: &str = include_str!("../evidence/radix-reader/oracle.py");
fn digest(s: &str) -> String {
    Sha256::digest(s)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
#[test]
fn radix_reader_matches_pinned_gnu() {
    let record: serde_json::Value = serde_json::from_str(GNU).unwrap();
    let cases: serde_json::Value = serde_json::from_str(CASES).unwrap();
    assert_eq!(record["protocol"], "radix-reader-v1");
    assert_eq!(
        record["oracle"]["revision"],
        "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea"
    );
    assert_eq!(
        record["cases_sha256"],
        digest(CASES),
        "radix cases changed: rerecord GNU"
    );
    assert_eq!(
        record["recorder_sha256"],
        digest(ORACLE),
        "radix recorder changed: rerecord GNU"
    );
    let rows = record["cases"].as_array().unwrap();
    assert_eq!(rows.len(), cases.as_array().unwrap().len());
    assert!(rows.len() >= 60, "radix corpus must not shrink");
    let mut failures = vec![];
    for (row, spec) in rows.iter().zip(cases.as_array().unwrap()) {
        assert_eq!(row["id"], spec["id"]);
        assert_eq!(row["input"], spec["input"]);
        let body = spec["expression"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "(read-from-string {})",
                    serde_json::to_string(spec["input"].as_str().unwrap()).unwrap()
                )
            });
        assert_eq!(
            row["form"],
            format!("(condition-case e {body} (error e))"),
            "radix recorded form drift"
        );
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let actual = match vm.eval_source(row["form"].as_str().unwrap()) {
            Ok(v) => v.display().unwrap(),
            Err(e) => format!("VM2-ERROR {e}"),
        };
        let expected = row["gnu"].as_str().unwrap();
        if actual != expected {
            failures.push(format!(
                "{} {}: GNU {expected}; ours {actual}",
                row["id"], row["input"]
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "RADIX_GNU_MISMATCH\n{}",
        failures.join("\n")
    );
}

#[test]
fn radix_bignum_constant_is_traced_through_compaction() {
    let record: serde_json::Value = serde_json::from_str(GNU).unwrap();
    let expected = record["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "form-0")
        .unwrap()["gnu"]
        .as_str()
        .unwrap();
    let mut vm = Vm::new();
    vm.set_collection_floor(1_000_000);
    // Dead objects before the constant guarantee a real index relocation.
    vm.eval_source("(vector (make-string 1000 65))").unwrap();
    let definition = vm.eval_source("(defun radix-retained () #xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF)");
    assert!(definition.is_ok(), "RADIX_CONSTANT_FORWARDING definition: {definition:?}");
    let local = vm
        .eval_source("(car (read-from-string \"#xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF\"))")
        .unwrap();
    let old = format!("{local:?}");
    let root = local.root();
    let before = vm.heap_stats().collections;
    vm.collect_garbage();
    assert!(
        vm.heap_stats().collections > before,
        "RADIX_GC_NOT_EXERCISED"
    );
    let local = root.get(&vm).unwrap();
    assert_ne!(format!("{local:?}"), old, "RADIX_GC_NOT_MOVED");
    assert_eq!(local.display().unwrap(), expected, "RADIX_ROOT_FORWARDING");
    drop(root);
    // The function must own a traced Form::Object, independently of our root.
    vm.collect_garbage();
    let answer = vm.eval_source("(radix-retained)").and_then(|v| v.display());
    assert_eq!(answer, Ok(expected.to_owned()), "RADIX_CONSTANT_FORWARDING");
}

#[test]
fn radix_bignum_discarded_constants_with_disk_store_match_gnu() {
    let record: serde_json::Value = serde_json::from_str(GNU).unwrap();
    let dir = std::env::temp_dir().join(format!("radix-store-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let mut answers = vec![];
    for case in record["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["id"].as_str().unwrap().starts_with("disk-"))
    {
        let mut vm = Vm::new();
        vm.set_source_cache_store(dir.clone()).unwrap();
        vm.set_collection_floor(1);
        let answer = vm
            .eval_source(case["form"].as_str().unwrap())
            .and_then(|v| v.display());
        answers.push((
            case["input"].to_string(),
            answer,
            case["gnu"].as_str().unwrap().to_owned(),
        ));
    }
    std::fs::remove_dir_all(dir).unwrap();
    assert_eq!(answers.len(), 2, "RADIX_DISK_CASES_MISSING");
    for (input, answer, gnu) in answers {
        assert_eq!(answer, Ok(gnu), "RADIX_DISK_STORE_MISMATCH {input}");
    }
}
