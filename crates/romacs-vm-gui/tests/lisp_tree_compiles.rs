//! Every file of the Lisp tree compiles (`--compile-lisp-tree`, as an
//! installer runs it), but those `lisp_tree_compile_failures.txt` lists.
//! The installer and the world build let a file that fails to compile load
//! as source, so a failure is otherwise silent; this check is where one
//! fails. The list matches the tree both ways: a listed file that compiles
//! fails too, so the fix that makes it compile deletes its entry, and an
//! entry is added only by a change that shows it, with its reason, in the
//! diff. A listed file that fails for another reason is not caught.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The known failures, beside this file: `PATH: reason` a line.
const KNOWN: &str = include_str!("lisp_tree_compile_failures.txt");

/// Compile processes at a time.
const JOBS: usize = 4;

/// The known failures' paths, each with its reason.
fn known() -> BTreeMap<&'static str, &'static str> {
    KNOWN.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split_once(": ").unwrap_or_else(|| panic!("lisp_tree_compile_failures.txt: `{line}' is not `PATH: reason'")))
        .collect()
}

/// The repository's runtime tree, by its true name: the summary names
/// files by theirs.
fn tree() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// The compiled-Lisp store the check keeps across runs, under the target
/// directory: an artifact is keyed by its source's content and the compile
/// world's identity, and holds only while its observed inputs do, so a run
/// compiles what changed since the last and decides the rest without a
/// compile process (`--compile-lisp-tree`).
fn kept_store() -> PathBuf {
    use std::os::unix::fs::DirBuilderExt;
    let store = Path::new(env!("CARGO_TARGET_TMPDIR")).join("lisp-tree-store");
    match std::fs::DirBuilder::new().mode(0o700).create(&store) {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => panic!("{}: {e}", store.display()),
        _ => store,
    }
}

/// `--compile-lisp-tree` over TREE into STORE, in the fresh private HOME
/// under ROOT: its summary.
fn compile_tree(tree: &Path, root: &Path, store: &Path) -> serde_json::Value {
    let home = root.join("home");
    for dir in ["config", "cache", "data", "state", "tmp"] {
        std::fs::create_dir_all(home.join(dir)).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(["--compile-lisp-tree", "--jobs", &JOBS.to_string()])
        .current_dir(tree)
        .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", &home).env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache")).env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_STATE_HOME", home.join("state")).env("TMPDIR", home.join("tmp")).env("LANG", "C.UTF-8")
        .env("ROMACS_RUNTIME_ROOT", tree)
        .env("ROMACS_COMPILED_LISP", "on").env("ROMACS_COMPILED_LISP_STORE", store)
        .envs(std::env::var_os("LD_LIBRARY_PATH").map(|v| ("LD_LIBRARY_PATH", v)))
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Status 2: only files that failed to compile, which the summary lists.
    assert!(output.status.success() || output.status.code() == Some(2),
            "--compile-lisp-tree: {}\n{stdout}\n{}", output.status, String::from_utf8_lossy(&output.stderr));
    let line = stdout.lines().rev().find(|line| line.starts_with('{')).unwrap_or_else(|| panic!("no summary: {stdout}"));
    serde_json::from_str(line).unwrap()
}

/// What ERRORS (each failing file with why) breaks of KNOWN, one finding
/// a line: a failure not listed, and a listed file that compiled.
fn findings(errors: &BTreeMap<String, String>, known: &BTreeMap<&str, &str>) -> Vec<String> {
    let new = errors.iter().filter(|(file, _)| !known.contains_key(file.as_str()))
        .map(|(file, why)| format!("{file}: fails to compile: {why}; fix: make it compile"));
    let fixed = known.keys().filter(|file| !errors.contains_key(**file))
        .map(|file| format!("{file}: compiles now; fix: delete its line from lisp_tree_compile_failures.txt"));
    new.chain(fixed).collect()
}

#[test]
fn every_file_of_the_lisp_tree_compiles_but_the_known_failures() {
    let tree = tree();
    let root = std::env::temp_dir().join(format!("romacs-tree-compile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let summary = compile_tree(&tree, &root, &kept_store());
    let _ = std::fs::remove_dir_all(&root);
    let empty = |field: &str| summary[field].as_array().is_none_or(Vec::is_empty);
    assert!(empty("failed") && empty("lost"), "files without a usable artifact: {summary}");
    // Every file of the tree came to one end, and there were files.
    let count = |field: &str| summary[field].as_u64().or_else(|| summary[field].as_array().map(|a| a.len() as u64)).unwrap();
    let ends: u64 = ["compiled", "compiled_before", "source_only", "errors"].into_iter().map(count).sum();
    assert!(count("files") > 1000 && ends == count("files"), "the run did not compile the tree: {summary}");
    let errors: BTreeMap<String, String> = summary["errors"].as_array().unwrap().iter().map(|entry| {
        let file = Path::new(entry["file"].as_str().unwrap());
        let file = file.strip_prefix(&tree).unwrap_or(file).display().to_string();
        (file, entry["why"].as_str().unwrap_or_default().to_owned())
    }).collect();
    let found = findings(&errors, &known());
    assert!(found.is_empty(), "{} of {} files:\n{}", found.len(), summary["files"], found.join("\n"));
}

/// The findings name each failure the list does not, and each listed file
/// that compiled; every entry says why.
#[test]
fn findings_name_unlisted_failures_and_listed_files_that_compile() {
    let listed = BTreeMap::from([("lisp/a.el", "why"), ("lisp/b.el", "why")]);
    let errors = |files: &[&str]| files.iter().map(|f| ((*f).to_owned(), "(error \"x\")".to_owned())).collect();
    assert_eq!(findings(&errors(&["lisp/a.el", "lisp/b.el"]), &listed), Vec::<String>::new());
    assert_eq!(findings(&errors(&["lisp/a.el", "lisp/b.el", "lisp/c.el"]), &listed),
               ["lisp/c.el: fails to compile: (error \"x\"); fix: make it compile"]);
    assert_eq!(findings(&errors(&["lisp/a.el"]), &listed),
               ["lisp/b.el: compiles now; fix: delete its line from lisp_tree_compile_failures.txt"]);
    assert!(known().values().all(|why| !why.is_empty()), "every entry says why");
}
