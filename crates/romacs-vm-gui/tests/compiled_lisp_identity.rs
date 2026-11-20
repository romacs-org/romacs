//! A compile process keys what it compiles by the compile world it
//! restored (src/batch.rs `compile_lisp_digest`): asked for diagnostics, its
//! identity lookup says it took the restored manifest, and that manifest's
//! digest, the one published in the compile store it restored from.
use std::path::Path;
use std::process::Command;

/// A `--compile-lisp FILE` run over STORE's compile world, as a session's
/// compile process runs (its HOME and TMPDIR the store's), with the runtime
/// tree TREE and diagnostics on: what it said on standard error.
fn compile(store: &Path, tree: &Path, file: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .arg("--world-store").arg(store.join("worlds")).arg("--compile-lisp").arg(file)
        .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", store.join("home")).env("TMPDIR", store.join("tmp"))
        .env("LANG", "C.UTF-8").env("ROMACS_RUNTIME_ROOT", tree).env("ROMACS_COMPILED_LISP", "on")
        .env("ROMACS_COMPILED_LISP_STORE", store).env("ROMACS_DIAGNOSTICS", "1")
        .envs(std::env::var_os("LD_LIBRARY_PATH").map(|v| ("LD_LIBRARY_PATH", v)))
        .output().unwrap();
    let said = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{said}");
    said
}

#[test]
fn a_compile_process_says_it_keys_by_the_world_it_restored() {
    use std::os::unix::fs::DirBuilderExt;
    let root = std::env::temp_dir().join(format!("romacs-compile-identity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let store = root.join("store");
    for dir in ["home", "tmp", "worlds"] {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(store.join(dir)).unwrap();
    }
    std::fs::create_dir_all(root.join("lib")).unwrap();
    // The tree the child is given, by its true name: what the store names its files relative to.
    let tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let lib = |name: &str| {
        let file = root.join("lib").join(format!("{name}.el"));
        std::fs::write(&file, format!(";;; -*- lexical-binding: t -*-\n(defun {name} () 1)\n(provide '{name})\n")).unwrap();
        file
    };
    // The first compile builds the compile world and restores it; the second
    // finds it made, as a compile of a tree's or a session's does.
    compile(&store, &tree, &lib("t-ident-a"));
    let said = compile(&store, &tree, &lib("t-ident-b"));
    let event = said.lines().filter_map(|line| line.strip_prefix("romacs-world: "))
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| event.get("compile_identity").is_some())
        .unwrap_or_else(|| panic!("the compile process said nothing of its identity: {said}"));
    assert_eq!(event["compile_identity"], "restored", "{said}");
    let manifests: Vec<_> = std::fs::read_dir(store.join("worlds")).unwrap().filter_map(Result::ok).map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json") && p.file_stem().is_some_and(|s| s.len() == 64))
        .collect();
    assert_eq!(manifests.len(), 1, "one compile world: {manifests:?}");
    let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&manifests[0]).unwrap()).unwrap();
    assert_eq!(event["lisp"], serde_json::json!(romacs_vm::world::store::lisp_digest(&manifest, &[tree])), "{event}");
    let _ = std::fs::remove_dir_all(&root);
}
