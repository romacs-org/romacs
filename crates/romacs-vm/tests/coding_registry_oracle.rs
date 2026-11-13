// Mutation plan (run only in an isolated source/target copy):
// p: make nil false or force lazy forms in the predicate; lazy counter catches it.
// check: return nil, change error datum, retain lazy form; identity/errors catch it.
// base: return alias/subsidiary instead of attrs[0]; alias/subsidiary rows catch it.
// plist: copy the list or sever the input tail; eq and mutation rows catch it.
// aliases: copy/truncate the list or omit duplicate appends; shared-list rows catch it.
// eol: swap dos/mac or return the stored vector; integer and aset rows catch it.
// put: skip normalization/validation, copy attrs, or replace the plist; setter rows catch it.
// alias: omit subsidiary publication or delete the old alias-list entry on rebind;
//        subsidiary/base and rebinding snapshots catch both mutations.
// internal: alter arity, validation order, category, EOL publishing, or GC roots;
//           declaration, error payload, category, subsidiary and collection rows catch it.
// These are strategies, not a claim that production mutants were executed.
// Replay live GNU evidence offline, with exact outcomes and no divergence pins.
use romacs_vm::Vm;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn validate_declaration(spec: &Value) {
    let manifest: Value =
        serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/primitives.json"))).unwrap();
    let current = manifest
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["name"] == spec["name"])
        .expect("declaration name missing from current runtime manifest");
    assert_eq!(
        spec["arity"],
        serde_json::json!([current["min"], current["max"]]),
        "declaration arity stale"
    );
    assert_eq!(
        spec["effect"].as_str().unwrap(),
        current["effect"].as_str().unwrap().to_lowercase(),
        "declaration effect stale"
    );
    assert_eq!(spec["gnu_def"], current["gnu"], "declaration gnu stale");
    // The file, not its line: a line moves whenever code above the declaration does, and two
    // branches that each move it would fail on their merge though neither is wrong.
    let file = |v: &Value| v.as_str().map(|s| s.rsplit_once(':').map_or(s, |(f, _)| f).to_owned());
    assert_eq!(file(&spec["rust"]), file(&current["rust"]), "declaration rust stale");
}

fn replay(spec: &[u8], evidence: &str, count: usize) {
    let spec_value: Value = serde_json::from_slice(spec).unwrap();
    let ev: Value = serde_json::from_str(evidence).unwrap();
    assert_eq!(ev["protocol"], "primh-v1");
    assert_eq!(ev["primitive"], spec_value["name"]);
    assert_eq!(
        ev["gnu"]["revision"],
        "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea"
    );
    assert_eq!(
        ev["gnu_binary_sha256"],
        "a31ba7aa12a7e4bbdda6ab9e19dab8338340f81f8100c33ac3843d10e176c8d8"
    );
    assert_eq!(ev["spec_sha256"], hash(spec), "spec changed; re-record");
    for (key, bytes) in [
        (
            "generator_sha256",
            include_bytes!("../tools/primh/primh.py").as_slice(),
        ),
        (
            "driver_sha256",
            include_bytes!("../tools/primh/gnu_driver.el").as_slice(),
        ),
        (
            "domains_sha256",
            include_bytes!("../tools/primh/domains.json").as_slice(),
        ),
        (
            "replay_sha256",
            include_bytes!("../tools/primh/replay.rs.in").as_slice(),
        ),
    ] {
        assert_eq!(ev[key], hash(bytes), "{key} changed; re-record");
    }
    assert!(ev.get("pinned").is_none(), "no divergence pins permitted");
    let rows = ev["cases"].as_array().unwrap();
    assert_eq!(rows.len(), count, "every recorded case must replay");
    let ids: BTreeSet<_> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), rows.len(), "duplicate case IDs");
    let mut bad = Vec::new();
    for floor in [1, 4096] {
        for row in rows {
            let id = row["id"].as_str().unwrap();
            let src = row["source"].as_str().unwrap();
            let expected = row["expected"].as_str().unwrap();
            assert_ne!(expected, "GNU-TIMEOUT", "no skipped cases: {id}");
            let mut vm = Vm::new();
            vm.set_collection_floor(floor);
            let actual = match vm.eval_source(src) {
                Ok(v) => v
                    .display()
                    .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s))
                    .unwrap_or_else(|e| format!("VM-ERROR {e}")),
                Err(e) => format!("VM-ERROR {e}"),
            };
            if actual != expected {
                bad.push(format!(
                    "{id} (GC floor {floor}): {src}\n  GNU {expected}\n  VM2 {actual}"
                ));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} exact mismatches:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn check_coding_system_matches_live_gnu() {
    replay(
        include_bytes!("../tools/primh/eval-specs/check-coding-system.json"),
        include_str!("../tools/primh/evidence/check-coding-system.json"),
        15,
    );
}

#[test]
fn coding_system_aliases_matches_live_gnu() {
    replay(
        include_bytes!("../tools/primh/eval-specs/coding-system-aliases.json"),
        include_str!("../tools/primh/evidence/coding-system-aliases.json"),
        15,
    );
}

#[test]
fn coding_system_base_matches_live_gnu() {
    replay(
        include_bytes!("../tools/primh/eval-specs/coding-system-base.json"),
        include_str!("../tools/primh/evidence/coding-system-base.json"),
        14,
    );
}

#[test]
fn coding_system_eol_type_matches_live_gnu() {
    replay(
        include_bytes!("../tools/primh/eval-specs/coding-system-eol-type.json"),
        include_str!("../tools/primh/evidence/coding-system-eol-type.json"),
        14,
    );
}

#[test]
fn coding_system_p_matches_live_gnu() {
    replay(
        include_bytes!("../tools/primh/eval-specs/coding-system-p.json"),
        include_str!("../tools/primh/evidence/coding-system-p.json"),
        12,
    );
}

#[test]
fn coding_system_plist_matches_live_gnu() {
    replay(
        include_bytes!("../tools/primh/eval-specs/coding-system-plist.json"),
        include_str!("../tools/primh/evidence/coding-system-plist.json"),
        16,
    );
}

#[test]
fn coding_system_put_matches_live_gnu() {
    replay(
        include_bytes!("../tools/primh/eval-specs/coding-system-put.json"),
        include_str!("../tools/primh/evidence/coding-system-put.json"),
        34,
    );
}

#[test]
fn define_coding_system_alias_matches_live_gnu() {
    replay(
        include_bytes!("../tools/primh/eval-specs/define-coding-system-alias.json"),
        include_str!("../tools/primh/evidence/define-coding-system-alias.json"),
        10,
    );
}

#[test]
fn define_coding_system_internal_matches_live_gnu() {
    replay(
        include_bytes!("../tools/primh/eval-specs/define-coding-system-internal.json"),
        include_str!("../tools/primh/evidence/define-coding-system-internal.json"),
        65,
    );
}

#[test]
fn coding_registry_declarations_match_live_gnu() {
    validate_declaration(
        &serde_json::from_slice(include_bytes!(
            "../tools/primh/eval-specs/check-coding-system.json"
        ))
        .unwrap(),
    );
    validate_declaration(
        &serde_json::from_slice(include_bytes!(
            "../tools/primh/eval-specs/coding-system-aliases.json"
        ))
        .unwrap(),
    );
    validate_declaration(
        &serde_json::from_slice(include_bytes!(
            "../tools/primh/eval-specs/coding-system-base.json"
        ))
        .unwrap(),
    );
    validate_declaration(
        &serde_json::from_slice(include_bytes!(
            "../tools/primh/eval-specs/coding-system-eol-type.json"
        ))
        .unwrap(),
    );
    validate_declaration(
        &serde_json::from_slice(include_bytes!(
            "../tools/primh/eval-specs/coding-system-p.json"
        ))
        .unwrap(),
    );
    validate_declaration(
        &serde_json::from_slice(include_bytes!(
            "../tools/primh/eval-specs/coding-system-plist.json"
        ))
        .unwrap(),
    );
    validate_declaration(
        &serde_json::from_slice(include_bytes!(
            "../tools/primh/eval-specs/coding-system-put.json"
        ))
        .unwrap(),
    );
    validate_declaration(
        &serde_json::from_slice(include_bytes!(
            "../tools/primh/eval-specs/define-coding-system-alias.json"
        ))
        .unwrap(),
    );
    validate_declaration(
        &serde_json::from_slice(include_bytes!(
            "../tools/primh/eval-specs/define-coding-system-internal.json"
        ))
        .unwrap(),
    );
}
