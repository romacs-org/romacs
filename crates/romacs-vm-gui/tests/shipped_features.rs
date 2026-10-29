//! What a shipped romacs-vm-gui is built with: its normal dependency graph
//! (`cargo build`), not the test graph. romacs-vm's test hooks
//! (`world-test-hooks`, which read ROMACS_WORLD_TEST_PAUSE_LOCK) are only
//! compiled under that feature, and only romacs-vm's own tests enable it.
//! A test build that also builds those tests may unify the feature into
//! this crate's test binaries, so the graph is what is checked, not a
//! binary.

#[test]
fn a_default_build_has_no_world_test_hooks() {
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = std::process::Command::new(cargo)
        .args(["tree", "--offline", "--locked", "-p", "romacs-vm-gui", "-e", "features,normal", "-i", "romacs-vm", "--manifest-path"])
        .arg(&workspace).output().unwrap();
    let tree = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(tree.contains("romacs-vm feature \"default\""), "{tree}");
    assert!(!tree.contains("world-test-hooks"), "the shipped graph enables the test hooks:\n{tree}");
}
