//! Every file of the Lisp tree compiles (`--compile-lisp-tree`, as an
//! installer runs it), but those `lisp_tree_compile_failures.txt` lists.
//! The installer and world build can load source when compilation fails.
//! This check compares the recorded diagnostics, reports recovered libraries,
//! and lets a trunk recorder remove their rows.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const BASELINE: &str = "crates/romacs-vm-gui/tests/lisp_tree_compile_failures.txt";
const KNOWN: &str = include_str!("lisp_tree_compile_failures.txt");

/// What starts the reason a unit refuses a constant it cannot keep
/// (romacs-vm's `compiled_lisp::unit::UNSERIALISABLE`).
const UNSERIALISABLE: &str = "a constant that cannot be serialised";

/// Compile processes at a time.
const JOBS: usize = 4;

/// The path and diagnostic on a baseline row.
fn entry(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    Some(line.split_once(": ").unwrap_or_else(|| panic!("{BASELINE}: expected PATH: diagnostic in {line:?}")))
}

fn known() -> BTreeMap<&'static str, &'static str> {
    KNOWN.lines().filter_map(entry).collect()
}

/// The compiler may put a source line before a serialization diagnostic.
fn diagnostic(reason: &str) -> &str {
    reason.strip_prefix("line ").and_then(|rest| rest.split_once(": "))
        .filter(|(line, _)| !line.is_empty() && line.bytes().all(|byte| byte.is_ascii_digit()))
        .map_or(reason, |(_, reason)| reason)
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

/// The failures in SUMMARY, including source-only files with a known error.
fn errors(summary: &serde_json::Value, tree: &Path, known: &BTreeMap<&str, &str>) -> BTreeMap<String, String> {
    ["errors", "source_only"].into_iter().flat_map(|kind| {
        summary[kind].as_array().unwrap().iter().filter_map(move |item| {
            let file = Path::new(item["file"].as_str().unwrap());
            let file = file.strip_prefix(tree).unwrap_or(file).to_string_lossy();
            let why = item["why"].as_str().unwrap();
            (kind == "errors" || known.contains_key(file.as_ref()) || why.contains(UNSERIALISABLE))
                .then(|| (file.into_owned(), why.to_owned()))
        })
    }).collect()
}

/// The new or changed compiler failures.
fn findings(errors: &BTreeMap<String, String>, known: &BTreeMap<&str, &str>) -> Vec<String> {
    errors.iter().filter_map(|(file, why)| match known.get(file.as_str()) {
        None => Some(format!("{file}: fails to compile: {why}; fix: make it compile")),
        Some(before) if *before != diagnostic(why) => {
            Some(format!("{file}: compiler failure changed from {before} to {why}; fix: make it compile"))
        }
        Some(_) => None,
    }).collect()
}

fn recording(tree: &Path) -> bool {
    if std::env::var_os("ROMACS_RECORD_LISP_COMPILE_FAILURES").as_deref() != Some("1".as_ref()) {
        return false;
    }
    if std::env::var_os("FORGE_TRUNK_JOB").as_deref() == Some("1".as_ref()) {
        return true;
    }
    let branch = Command::new("git").current_dir(tree).args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .env_remove("GIT_DIR").env_remove("GIT_WORK_TREE").env_remove("GIT_INDEX_FILE")
        .output().unwrap();
    assert!(branch.status.success() && branch.stdout == b"beta\n",
            "recording Lisp compile failures requires beta or FORGE_TRUNK_JOB=1");
    true
}

fn retained(baseline: &str, errors: &BTreeMap<String, String>) -> String {
    baseline.split_inclusive('\n').filter(|line| entry(line).is_none_or(|(file, _)| errors.contains_key(file))).collect()
}

fn record_fixes(path: &Path, baseline: &str, errors: &BTreeMap<String, String>) {
    let updated = retained(baseline, errors);
    if updated == baseline {
        return;
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), baseline, "{BASELINE} changed during the check");
    let draft = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&draft, updated).and_then(|()| std::fs::rename(&draft, path)).unwrap_or_else(|error| {
        let _ = std::fs::remove_file(draft);
        panic!("{}: {error}", path.display());
    });
}

#[test]
fn every_file_of_the_lisp_tree_compiles_but_the_known_failures() {
    let tree = tree();
    let record = recording(&tree);
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
    let known = known();
    let errors = errors(&summary, &tree, &known);
    let found = findings(&errors, &known);
    assert!(found.is_empty(), "{} of {} files:\n{}", found.len(), summary["files"], found.join("\n"));
    for file in known.keys().filter(|file| !errors.contains_key(**file)) {
        let state = if tree.join(file).is_file() { "compiles now" } else { "removed from the tree" };
        eprintln!("{file}: {state}");
    }
    if record {
        record_fixes(&tree.join(BASELINE), KNOWN, &errors);
    }
}

#[test]
fn a_known_library_compiling_again_passes() {
    let listed = BTreeMap::from([("lisp/a.el", "old reason"), ("lisp/b.el", "old reason")]);
    let errors = BTreeMap::from([("lisp/a.el".into(), "line 90: old reason".into())]);
    assert!(findings(&errors, &listed).is_empty());
}

#[test]
fn a_known_library_failing_differently_fails() {
    let listed = BTreeMap::from([("lisp/a.el", "old reason")]);
    let errors = BTreeMap::from([("lisp/a.el".into(), "new reason".into())]);
    assert!(!findings(&errors, &listed).is_empty());
}

#[test]
fn a_new_compiler_failure_fails() {
    let listed = BTreeMap::from([("lisp/a.el", "(error \"x\")")]);
    let errors = BTreeMap::from([("lisp/a.el".into(), "(error \"x\")".into()),
                                 ("lisp/b.el".into(), "(error \"x\")".into())]);
    assert_eq!(findings(&errors, &listed),
               ["lisp/b.el: fails to compile: (error \"x\"); fix: make it compile"]);
}

#[test]
fn an_existing_failure_switching_to_source_only_still_fails() {
    let listed = BTreeMap::from([("lisp/a.el", "old reason")]);
    let summary = serde_json::json!({"errors": [], "source_only": [
        {"file": "/tree/lisp/a.el", "why": "no-byte-compile"},
        {"file": "/tree/lisp/b.el", "why": "no-byte-compile"}
    ]});
    let measured = errors(&summary, Path::new("/tree"), &listed);
    assert_eq!(findings(&measured, &listed),
               ["lisp/a.el: compiler failure changed from old reason to no-byte-compile; fix: make it compile"]);
}

#[test]
fn trunk_recording_drops_only_repaired_rows() {
    let baseline = "# known failures\nlisp/a.el: old reason\nlisp/b.el: still broken\n";
    let mut measured = BTreeMap::from([("lisp/a.el".into(), "old reason".into()),
                                       ("lisp/b.el".into(), "still broken".into())]);
    assert_eq!(retained(baseline, &measured), baseline);
    measured.remove("lisp/a.el");
    assert_eq!(retained(baseline, &measured), "# known failures\nlisp/b.el: still broken\n");
}
