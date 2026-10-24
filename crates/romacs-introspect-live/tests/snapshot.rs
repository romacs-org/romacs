//! The committed VM2 snapshot must match fresh function-cell enumeration.
//! Name additions and removals fail with an exact diff; VM2 arities are not
//! represented by this schema and were never checked by its VM2 half.
use romacs_introspect::Registry;

#[test]
fn snapshot_matches_live_registry() {
    let live = romacs_introspect_live::enumerate().expect("enumerate live VM2");
    let expected = serde_json::to_string_pretty(&live.to_json()).unwrap() + "\n";
    for path in [
        romacs_introspect_live::snapshot_path(),
        romacs_introspect_live::embedded_snapshot_path(),
    ] {
        assert_eq!(
            std::fs::read(&path).expect("snapshot bytes"),
            expected.as_bytes(),
            "{} is stale; regenerate both snapshots",
            path.display()
        );
    }
    let snapshot = romacs_introspect_live::snapshot().expect("read committed snapshot");
    let diff = snapshot.diff(&live);
    assert!(
        diff.is_empty(),
        "crates/romacs-introspect-live/data/registry.json is stale against VM2:\n{diff}\
         regenerate with: cargo run -p romacs-introspect-live -- --write"
    );
}

/// The same check with a missing binding must fail, rather than pass vacuously.
#[test]
fn a_snapshot_missing_one_primitive_is_reported() {
    let live = romacs_introspect_live::enumerate().expect("enumerate live VM2");
    let mut stale: Registry = live.clone();
    let dropped = stale.vm2.remove(0);
    let diff = stale.diff(&live);
    assert_eq!(diff.vm2_added, vec![dropped]);
    assert!(!diff.is_empty());
}

#[test]
fn a_snapshot_with_an_extra_primitive_is_reported() {
    let live = romacs_introspect_live::enumerate().expect("enumerate live VM2");
    let mut stale = live.clone();
    let extra = "surely-not-a-function-q7".to_owned();
    assert!(!stale.vm2.contains(&extra));
    stale.vm2.push(extra.clone());
    let diff = stale.diff(&live);
    assert_eq!(diff.vm2_removed, vec![extra]);
    assert!(!diff.is_empty());
}

#[test]
fn identical_inputs_enumerate_to_identical_bytes_twice() {
    let first = romacs_introspect_live::enumerate().expect("first enumeration");
    let mut second = romacs_introspect_live::enumerate().expect("second enumeration");
    second.provenance.generated_at_unix = first.provenance.generated_at_unix + 100;
    assert_eq!(
        serde_json::to_vec_pretty(&first.to_json()).unwrap(),
        serde_json::to_vec_pretty(&second.to_json()).unwrap()
    );
    assert_eq!(first.to_json()["schema"], 2);
    for field in ["generated_at_unix", "commit", "inputs_clean"] {
        assert!(first.to_json()["provenance"].get(field).is_none());
    }
}
