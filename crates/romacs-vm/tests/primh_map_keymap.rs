//! Replay live-GNU-generated observations through the public rooted VM API.
use romacs_vm::Vm;
use sha2::{Digest, Sha256};

fn hash(bytes: &[u8]) -> String { Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect() }
fn replay(evidence: &str, spec: &[u8]) {
    let evidence: serde_json::Value = serde_json::from_str(evidence).unwrap();
    assert_eq!(evidence["protocol"], "primh-v1");
    assert_eq!(evidence["spec_sha256"], hash(spec));
    let spec: serde_json::Value = serde_json::from_slice(spec).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/primitives.json"))).unwrap();
    let declaration = manifest.as_array().unwrap().iter().find(|r| r["name"] == spec["name"]).unwrap();
    assert_eq!(spec["arity"], serde_json::json!([declaration["min"], declaration["max"]]));
    assert_eq!(spec["effect"], declaration["effect"].as_str().unwrap().to_lowercase());
    assert_eq!(spec["gnu_def"], declaration["gnu"]);
    let file = |v: &serde_json::Value| v.as_str().map(|s| s.rsplit_once(':').map_or(s, |(f, _)| f).to_owned());
    assert_eq!(file(&spec["rust"]), file(&declaration["rust"]));
    for (key, bytes) in [
        ("generator_sha256", include_bytes!("../tools/primh/primh.py").as_slice()),
        ("driver_sha256", include_bytes!("../tools/primh/gnu_driver.el").as_slice()),
        ("domains_sha256", include_bytes!("../tools/primh/domains.json").as_slice()),
        ("replay_sha256", include_bytes!("../tools/primh/replay.rs.in").as_slice()),
    ] { assert_eq!(evidence[key], hash(bytes), "{key}"); }
    let rows = evidence["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    let mut ids = std::collections::BTreeSet::new();
    for row in rows {
        assert!(ids.insert(row["id"].as_str().unwrap()));
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let result = vm.eval_source(row["source"].as_str().unwrap()).unwrap();
        let printed = result.display().unwrap();
        let actual: String = serde_json::from_str(&printed).unwrap();
        assert_eq!(actual, row["expected"].as_str().unwrap(), "{}", row["id"]);
    }
}
#[test]
fn map_keymap_recorded_history_and_arity() {
    replay(include_str!("../tools/primh/evidence/map-keymap.json"), include_bytes!("../tools/primh/ert-path-specs/map-keymap.json"));
}
#[test]
fn map_keymap_internal_recorded_parent_and_arity() {
    replay(include_str!("../tools/primh/evidence/map-keymap-internal.json"), include_bytes!("../tools/primh/ert-path-specs/map-keymap-internal.json"));
}
