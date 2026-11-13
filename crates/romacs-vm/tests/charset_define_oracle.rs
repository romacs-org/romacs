// Replay live GNU evidence offline, with exact outcomes and no divergence pins.
use romacs_vm::Vm;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn validate_declaration(spec: &Value) {
    let manifest: Value = serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/primitives.json"))).unwrap();
    let current = manifest.as_array().unwrap().iter()
        .find(|m| m["name"] == spec["name"])
        .expect("declaration name missing from current runtime manifest");
    assert_eq!(spec["arity"], serde_json::json!([current["min"], current["max"]]), "declaration arity stale");
    assert_eq!(spec["effect"].as_str().unwrap(), current["effect"].as_str().unwrap().to_lowercase(), "declaration effect stale");
    assert_eq!(spec["gnu_def"], current["gnu"], "declaration gnu stale");
    // The file, not its line: a line moves whenever code above the declaration does, and two
    // branches that each move it would fail on their merge though neither is wrong.
    let file = |v: &Value| v.as_str().map(|s| s.rsplit_once(':').map_or(s, |(f, _)| f).to_owned());
    assert_eq!(file(&spec["rust"]), file(&current["rust"]), "declaration rust stale");
}

#[test]
fn define_charset_internal_matches_recorded_gnu() {
    let spec = include_bytes!("../tools/primh/eval-specs/define-charset-internal.json");
    let spec_value: Value = serde_json::from_slice(spec).unwrap();
    validate_declaration(&spec_value);
    let ev: Value = serde_json::from_str(include_str!("../tools/primh/evidence/define-charset-internal.json")).unwrap();
    assert_eq!(ev["protocol"], "primh-v1");
    assert_eq!(ev["primitive"], "define-charset-internal");
    assert_eq!(ev["spec_sha256"], hash(spec), "spec changed; re-record");
    for (key, bytes) in [
        ("generator_sha256", include_bytes!("../tools/primh/primh.py").as_slice()),
        ("driver_sha256", include_bytes!("../tools/primh/gnu_driver.el").as_slice()),
        ("domains_sha256", include_bytes!("../tools/primh/domains.json").as_slice()),
        ("replay_sha256", include_bytes!("../tools/primh/replay.rs.in").as_slice()),
    ] {
        assert_eq!(ev[key], hash(bytes), "{key} changed; re-record");
    }
    assert!(ev.get("pinned").is_none(), "no divergence pins permitted");
    assert_eq!(spec_value["probes"].as_array().unwrap().len(), 79);
    let rows = ev["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 80, "79 probes plus generated arity case");
    let ids: BTreeSet<_> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), rows.len(), "duplicate case IDs");
    let mut bad = Vec::new();
    for row in rows {
        let id = row["id"].as_str().unwrap();
        let src = row["source"].as_str().unwrap();
        let expected = row["expected"].as_str().unwrap();
        assert_ne!(expected, "GNU-TIMEOUT", "no skipped cases: {id}");
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let actual = match vm.eval_source(src) {
            Ok(v) => v.display().map(|s| serde_json::from_str::<String>(&s).unwrap_or(s)).unwrap_or_else(|e| format!("VM-ERROR {e}")),
            Err(e) => format!("VM-ERROR {e}"),
        };
        if actual != expected {
            bad.push(format!("{id}: {src}\n  GNU {expected}\n  VM2 {actual}"));
        }
    }
    assert!(bad.is_empty(), "{} exact mismatches:\n{}", bad.len(), bad.join("\n"));
}
