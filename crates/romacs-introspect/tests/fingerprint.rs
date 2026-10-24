//! Identity tests use tiny source fixtures, never mutate the live workspace.
use romacs_introspect::fingerprint::input_hash;
use std::path::Path;

fn put(root: &Path, path: &str, bytes: &str) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}
fn fixture(root: &Path, reverse: bool) {
    let mut entries = vec![
        (
            "Cargo.toml",
            "[workspace]\n[workspace.dependencies]\nshared = { path = 'crates/shared' }\n",
        ),
        ("Cargo.lock", "locked dependencies"),
        (
            "crates/romacs-introspect-live/Cargo.toml",
            "[package]\nname='live'\nversion='1.0.0'\n[dependencies]\nvm = { path = '../romacs-vm' }\n",
        ),
        (
            "crates/romacs-vm/Cargo.toml",
            "[package]\nname='vm'\nversion='1.0.0'\n[target.'cfg(unix)'.build-dependencies]\nshared.workspace = true\n",
        ),
        (
            "crates/shared/Cargo.toml",
            "[package]\nname='shared'\nversion='1.0.0'\n",
        ),
        ("crates/shared/src/lib.rs", "dependency source"),
        ("crates/romacs-vm/src/lib.rs", "VM source"),
        ("rlisp/vm/prelude.el", "(defun embedded () t)"),
        (
            "crates/romacs-vm/src/registry.json",
            "embedded JSON, not an output",
        ),
        ("crates/romacs-vm/build.rs", "build generator"),
        (
            "crates/romacs-introspect-live/src/main.rs",
            "generator source",
        ),
    ];
    if reverse {
        entries.reverse();
    }
    for (path, bytes) in entries {
        put(root, path, bytes);
    }
}

#[test]
fn identity_is_path_order_and_checkout_independent_and_outputs_are_not_inputs() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    fixture(a.path(), false);
    fixture(b.path(), true);
    let hash = input_hash(a.path()).unwrap();
    assert_eq!(hash.len(), 64);
    assert_eq!(hash, input_hash(b.path()).unwrap());
    for path in [
        ".git/HEAD",
        "crates/unrelated/src/lib.rs",
        "crates/romacs-introspect/data/registry.json",
        "crates/romacs-introspect-live/data/registry.json",
        "crates/romacs-vm/target/output",
        "crates/romacs-vm/README.md",
        "crates/romacs-vm/tests/new.rs",
        "crates/romacs-vm/evidence/ignored.log",
        "crates/romacs-vm/build-output.log",
        "crates/romacs-vm/src/lib.rs~",
        "crates/romacs-vm/src/#lib.rs#",
        "crates/romacs-vm/src/.lib.rs.swp",
    ] {
        put(a.path(), path, "not an input");
        assert_eq!(hash, input_hash(a.path()).unwrap(), "{path}");
    }
}

#[test]
fn identity_covers_source_bytes_paths_lisp_build_manifests_and_transitive_dependencies() {
    for path in [
        "Cargo.toml",
        "Cargo.lock",
        "crates/romacs-vm/src/lib.rs",
        "rlisp/vm/prelude.el",
        "crates/romacs-vm/src/registry.json",
        "crates/romacs-vm/build.rs",
        "crates/shared/src/lib.rs",
        "crates/romacs-introspect-live/src/main.rs",
    ] {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path(), false);
        let before = input_hash(dir.path()).unwrap();
        let p = dir.path().join(path);
        let mut bytes = std::fs::read(&p).unwrap();
        bytes.extend(b"\n# changed\n");
        std::fs::write(&p, bytes).unwrap();
        assert_ne!(before, input_hash(dir.path()).unwrap(), "{path}");
    }
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), false);
    let before = input_hash(dir.path()).unwrap();
    std::fs::rename(
        dir.path().join("rlisp/vm/prelude.el"),
        dir.path().join("rlisp/vm/renamed.el"),
    )
    .unwrap();
    assert_ne!(before, input_hash(dir.path()).unwrap());
    std::fs::remove_file(dir.path().join("crates/shared/Cargo.toml")).unwrap();
    assert!(
        input_hash(dir.path()).is_err(),
        "missing dependency must fail closed"
    );
}

#[test]
fn ignored_evidence_logs_and_git_metadata_match_a_git_free_export() {
    let checkout = tempfile::tempdir().unwrap();
    let export = tempfile::tempdir().unwrap();
    fixture(checkout.path(), false);
    fixture(export.path(), false);
    put(checkout.path(), ".gitignore", "*.log\n");
    put(
        checkout.path(),
        "crates/romacs-vm/evidence/ignored.log",
        "local evidence log",
    );
    let init = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(checkout.path())
        .status()
        .unwrap();
    assert!(init.success());
    let ignored = std::process::Command::new("git")
        .args([
            "check-ignore",
            "--quiet",
            "crates/romacs-vm/evidence/ignored.log",
        ])
        .current_dir(checkout.path())
        .status()
        .unwrap();
    assert!(ignored.success());
    assert_eq!(
        input_hash(checkout.path()).unwrap(),
        input_hash(export.path()).unwrap()
    );
    put(checkout.path(), ".gitignore", "different-ignore-history\n");
    assert_eq!(
        input_hash(checkout.path()).unwrap(),
        input_hash(export.path()).unwrap()
    );
}

#[test]
fn new_untracked_primitive_source_changes_identity() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), false);
    let before = input_hash(dir.path()).unwrap();
    put(
        dir.path(),
        "crates/romacs-vm/src/prims/new.rs",
        "new primitive",
    );
    assert_ne!(before, input_hash(dir.path()).unwrap());
}

#[test]
fn all_local_workspace_patches_are_inputs_including_renamed_and_non_crates_io() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), false);
    put(
        dir.path(),
        "Cargo.toml",
        "[workspace]\n[workspace.dependencies]\nshared = { path = 'crates/shared' }\n[patch.crates-io]\nalias = { package = 'actual', path = 'local/one' }\n[patch.'https://example.org/registry']\nunused = { path = 'local/two' }\n",
    );
    for name in ["one", "two"] {
        put(
            dir.path(),
            &format!("local/{name}/Cargo.toml"),
            &format!(
                "[package]\nname='{name}'\nversion='1.0.0'\n[dependencies]\nshared = {{ path = '../../crates/shared' }}\n"
            ),
        );
        put(
            dir.path(),
            &format!("local/{name}/src/lib.rs"),
            "patched source",
        );
    }
    let before = input_hash(dir.path()).unwrap();
    for name in ["one", "two"] {
        put(
            dir.path(),
            &format!("local/{name}/src/lib.rs"),
            "changed patch",
        );
        assert_ne!(before, input_hash(dir.path()).unwrap());
        put(
            dir.path(),
            &format!("local/{name}/src/lib.rs"),
            "patched source",
        );
        assert_eq!(before, input_hash(dir.path()).unwrap());
    }
}

#[test]
fn a_missing_lockfile_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), false);
    std::fs::remove_file(dir.path().join("Cargo.lock")).unwrap();
    assert!(input_hash(dir.path()).is_err());
}

#[cfg(unix)]
#[test]
fn dangling_editor_lock_does_not_change_identity() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), false);
    let before = input_hash(dir.path()).unwrap();
    std::os::unix::fs::symlink(
        "missing-target",
        dir.path().join("crates/romacs-vm/src/.#x.rs"),
    )
    .unwrap();
    assert_eq!(before, input_hash(dir.path()).unwrap());
}

#[cfg(unix)]
#[test]
fn symlinked_source_fails_closed_instead_of_escaping_or_looping() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), false);
    std::os::unix::fs::symlink(".", dir.path().join("crates/romacs-vm/src/loop")).unwrap();
    assert!(input_hash(dir.path()).is_err());
}
