//! Gate-visible validation: JSON data does not get checked by the Rust compiler.
use std::path::Path;
use std::process::Command;

#[test]
fn every_derived_artifact_is_parseable_and_conflict_free() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("python3")
        .arg(root.join("tools/derived_artifacts.py"))
        .arg("validate")
        .arg("--root")
        .arg(&root)
        .output()
        .expect("run derived JSON validator (not a compiler proxy)");
    assert!(
        output.status.success(),
        "derived artifact validation must reject conflict markers and unparsable JSON:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
