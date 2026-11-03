//! A batch run with `--world-store' builds the runner's world once, confined
//! to what the bootstrap and Lisp startup read, and a later run restores it,
//! even when its test sits in another directory, as run.py extracts each
//! run's test tree afresh; the restored run still loads its target there.
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

fn scratch() -> PathBuf {
    let base = std::env::var_os("TMPDIR").map(PathBuf::from).expect("lane TMPDIR");
    let path = base.join(format!("world-restore-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn events(output: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout).lines().filter_map(|line| serde_json::from_str(line).ok()).collect()
}

/// The world event a run under DIR, a test directory of its own, reports,
/// and whether its target loaded from DIR.
fn run(scratch: &Path, store: &Path, dir: &str) -> (Value, bool, Output) {
    let dir = scratch.join(dir);
    fs::create_dir_all(&dir).unwrap();
    let target = dir.join("target.el");
    fs::write(&target, "(setq target-dir (file-name-directory load-file-name))\n").unwrap();
    let harness = dir.join("harness.el");
    fs::write(&harness, "(defun ert-run-tests-batch (_selector) t)\n").unwrap();
    let reporter = dir.join("reporter.el");
    fs::write(&reporter, "(princ (format \"TARGET-DIR %s\\n\" target-dir))\n0\n").unwrap();
    let plan = dir.join("plan.json");
    fs::write(&plan, json!({"file": target, "forms": []}).to_string()).unwrap();
    let lisp = root().join("gnu-lisp");
    let mut command = Command::new(env!("CARGO_BIN_EXE_romacs-gnu-ert"));
    command.args(["--harness-kind", "real", "--harness"]).arg(&harness).arg("--plan").arg(&plan)
        .arg("--reporter").arg(&reporter)
        .arg("--preload").arg(root().join("rlisp/romacs-loadup-bootstrap.el"))
        .arg("--load-path").arg(&lisp);
    let mut subdirs: Vec<PathBuf> = fs::read_dir(&lisp).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
    subdirs.sort();
    for subdir in subdirs {
        command.arg("--load-path").arg(subdir);
    }
    command.arg("--load-path").arg(root().join("rlisp")).arg("--load-path").arg(&dir).arg("--world-store").arg(store);
    let output = command.output().unwrap();
    let world = events(&output).into_iter().find(|e| e["event"] == "world").unwrap_or(Value::Null);
    let loaded = String::from_utf8_lossy(&output.stdout).contains(&format!("TARGET-DIR {}/", dir.display()));
    (world, loaded, output)
}

#[test]
fn a_batch_world_store_build_restores_for_a_test_elsewhere() {
    let scratch = scratch();
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
