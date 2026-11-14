use romacs_vm::Vm;
use serde_json::Value;
use sha2::{Digest, Sha256};

const SPEC: &[u8] = include_bytes!("../tools/primh/format-message-specs/format-message.json");
const EVIDENCE: &str = include_str!("../tools/primh/evidence/format-message.json");

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn format_message_exact_pinned_gnu_behavior() {
    let evidence: Value = serde_json::from_str(EVIDENCE).unwrap();
    assert_eq!(evidence["spec_sha256"], hash(SPEC));
    assert_eq!(evidence["gnu"]["revision"], "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea");
    assert_eq!(evidence["gnu_binary_sha256"], "a31ba7aa12a7e4bbdda6ab9e19dab8338340f81f8100c33ac3843d10e176c8d8");
    for (field, bytes) in [
        ("generator_sha256", include_bytes!("../tools/primh/primh.py").as_slice()),
        ("driver_sha256", include_bytes!("../tools/primh/gnu_driver.el").as_slice()),
        ("domains_sha256", include_bytes!("../tools/primh/domains.json").as_slice()),
        ("replay_sha256", include_bytes!("../tools/primh/replay.rs.in").as_slice()),
    ] {
        assert_eq!(evidence[field], hash(bytes), "stale {field}");
    }
    let rows = evidence["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 64);
    let mut failures = Vec::new();
    for row in rows {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let source = row["source"].as_str().unwrap();
        let actual = match vm.eval_source(source) {
            Ok(value) => {
                let display = value.display().unwrap();
                serde_json::from_str::<String>(&display).unwrap_or(display)
            }
            Err(error) => format!("VM-ERROR {error}"),
        };
        if actual != row["expected"].as_str().unwrap() {
            failures.push(format!("{}: {source}\nGNU: {}\nVM2: {actual}", row["id"], row["expected"]));
        }
    }
    assert!(failures.is_empty(), "format-message exact behavioral mismatches:\n{}", failures.join("\n"));
}

#[test]
fn format_message_declaration_matches_oracle_spec() {
    let spec: Value = serde_json::from_slice(SPEC).unwrap();
    let manifest: Value = serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/primitives.json"))).unwrap();
    let declaration = manifest.as_array().unwrap().iter()
        .find(|entry| entry["name"] == spec["name"])
        .expect("format-message declaration missing");
    assert_eq!(spec["arity"], serde_json::json!([declaration["min"], declaration["max"]]));
    assert_eq!(spec["effect"].as_str().unwrap(), declaration["effect"].as_str().unwrap().to_lowercase());
    assert_eq!(spec["gnu_def"], declaration["gnu"]);
    // The file, not its line: a line moves whenever code above the declaration does, and two
    // branches that each move it would fail on their merge though neither is wrong.
    let file = |v: &Value| v.as_str().map(|s| s.rsplit_once(':').map_or(s, |(f, _)| f).to_owned());
    assert_eq!(file(&spec["rust"]), file(&declaration["rust"]));
}

#[test]
fn format_message_startup_metadata_does_not_allocate_lisp_objects() {
    let vm = Vm::new();
    assert_eq!(vm.heap_stats().allocations, 0);
    assert_eq!(vm.heap_stats().live_objects, 0);
}
