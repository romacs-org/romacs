//! A batch run with `--world-store' builds the runner's world once, confined
//! to what the bootstrap and Lisp startup read, and a later run restores it,
//! even when its test sits in another directory, as run.py extracts each
//! run's test tree afresh; the restored run's `load-path' has that
//! directory, so its target finds a library beside it by name.
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::{Duration, Instant},
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// A fresh directory for the test NAME alone, so tests run as threads of
/// one process do not share one.
fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR").map(PathBuf::from).expect("lane TMPDIR");
    let path = base.join(format!("world-restore-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn events(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout).lines().filter_map(|line| serde_json::from_str(line).ok()).collect()
}

/// The world event a run under DIR, a test directory of its own, reports,
/// and whether its target required the library beside it in DIR.
fn run(scratch: &Path, store: &Path, dir: &str) -> (Value, bool, Output) {
    run_in(&root(), &[], scratch, store, dir)
}

/// `run' with the tree at TREE and its PRELOADS after the bootstrap.
fn run_in(tree: &Path, preloads: &[PathBuf], scratch: &Path, store: &Path, dir: &str) -> (Value, bool, Output) {
    let dir = scratch.join(dir);
    fs::create_dir_all(&dir).unwrap();
    let target = dir.join("target.el");
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    fs::write(dir.join("wr-sibling.el"), format!("(defvar wr-sibling-run {name:?})\n(provide 'wr-sibling)\n")).unwrap();
    fs::write(&target, "(require 'wr-sibling)\n").unwrap();
    let harness = dir.join("harness.el");
    fs::write(&harness, "(defun ert-run-tests-batch (_selector) t)\n").unwrap();
    let reporter = dir.join("reporter.el");
    fs::write(&reporter, "(princ (format \"SIBLING %s\\n\" wr-sibling-run))\n0\n").unwrap();
    let plan = dir.join("plan.json");
    fs::write(&plan, json!({"file": target, "forms": []}).to_string()).unwrap();
    let lisp = tree.join("gnu-lisp");
    let mut command = Command::new(env!("CARGO_BIN_EXE_romacs-gnu-ert"));
    command.args(["--harness-kind", "real", "--harness"]).arg(&harness).arg("--plan").arg(&plan)
        .arg("--reporter").arg(&reporter)
        .arg("--preload").arg(tree.join("rlisp/romacs-loadup-bootstrap.el"));
    for preload in preloads {
        command.arg("--preload").arg(preload);
    }
    command.arg("--load-path").arg(&lisp);
    let mut subdirs: Vec<PathBuf> = fs::read_dir(&lisp).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
    subdirs.sort();
    for subdir in subdirs {
        command.arg("--load-path").arg(subdir);
    }
    command.arg("--load-path").arg(tree.join("rlisp")).arg("--load-path").arg(&dir).arg("--world-store").arg(store);
    let output = command.output().unwrap();
    let world = events(&output).into_iter().find(|e| e["event"] == "world").unwrap_or(Value::Null);
    let loaded = output.status.success() && String::from_utf8_lossy(&output.stdout).contains(&format!("SIBLING {name}\n"));
    (world, loaded, output)
}

#[test]
fn a_batch_world_store_build_restores_for_a_test_elsewhere() {
    let scratch = scratch("restores");
    let store = scratch.join("store");
    fs::create_dir_all(&store).unwrap();
    fs::set_permissions(&store, fs::Permissions::from_mode(0o700)).unwrap();
    // The first run starts the build and runs from source meanwhile.
    let (first, loaded, output) = run(&scratch, &store, "run-1");
    assert_eq!(first["mode"], "source", "{first}");
    assert!(loaded, "{}", String::from_utf8_lossy(&output.stdout));
    let published = |store: &Path| fs::read_dir(store).unwrap().filter_map(|e| e.ok())
        .any(|e| e.path().extension().is_some_and(|x| x == "json"));
    let failed = |store: &Path| fs::read_dir(store).unwrap().filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|x| x == "failed")).map(|e| fs::read_to_string(e.path()).unwrap_or_default());
    let start = Instant::now();
    while !published(&store) {
        assert_eq!(failed(&store), None, "the world build failed");
        assert!(start.elapsed() < Duration::from_secs(600), "no world published");
        std::thread::sleep(Duration::from_millis(200));
    }
    // A later run in another test directory restores it.
    let (second, loaded, output) = run(&scratch, &store, "run-2");
    assert_eq!(second["mode"], "restored", "{second}");
    assert!(loaded, "{}", String::from_utf8_lossy(&output.stdout));
    let _ = fs::remove_dir_all(&scratch);
}

/// A world whose preloads change `load-path' is not published: where a
/// run's own entries would go among the changes is unclear, so runs boot
/// from source, their own directory still on `load-path'.
#[test]
fn a_world_whose_preloads_change_load_path_is_not_published() {
    let scratch = scratch("pushing");
    let tree = scratch.join("tree");
    fs::create_dir_all(tree.join("rlisp")).unwrap();
    std::os::unix::fs::symlink(root().join("gnu-lisp"), tree.join("gnu-lisp")).unwrap();
    for entry in fs::read_dir(root().join("rlisp")).unwrap().filter_map(|e| e.ok()) {
        std::os::unix::fs::symlink(entry.path(), tree.join("rlisp").join(entry.file_name())).unwrap();
    }
    let push = tree.join("rlisp/wr-push.el");
    fs::write(&push, format!("(push {:?} load-path)\n", tree.join("gnu-lisp").display().to_string())).unwrap();
    let store = scratch.join("store");
    fs::create_dir_all(&store).unwrap();
    fs::set_permissions(&store, fs::Permissions::from_mode(0o700)).unwrap();
    let (first, loaded, output) = run_in(&tree, std::slice::from_ref(&push), &scratch, &store, "run-1");
    assert_eq!(first["mode"], "source", "{first}");
    assert!(loaded, "{}", String::from_utf8_lossy(&output.stdout));
    let start = Instant::now();
    let failed = loop {
        let found = fs::read_dir(&store).unwrap().filter_map(|e| e.ok())
            .find(|e| e.path().extension().is_some_and(|x| x == "failed")).map(|e| fs::read_to_string(e.path()).unwrap_or_default());
        if let Some(failed) = found { break failed; }
        assert!(start.elapsed() < Duration::from_secs(600), "no build outcome");
        assert!(!fs::read_dir(&store).unwrap().filter_map(|e| e.ok()).any(|e| e.path().extension().is_some_and(|x| x == "json")),
                "a world was published though its preloads changed load-path");
        std::thread::sleep(Duration::from_millis(200));
    };
    assert!(failed.contains("the preloads changed load-path"), "{failed}");
    let (second, loaded, output) = run_in(&tree, std::slice::from_ref(&push), &scratch, &store, "run-2");
    assert_eq!(second["mode"], "source", "{second}");
    assert!(loaded, "{}", String::from_utf8_lossy(&output.stdout));
    let _ = fs::remove_dir_all(&scratch);
}
