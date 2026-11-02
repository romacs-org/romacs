//! The mini harness evaluates a test file's top-level forms in the context
//! GNU's `load' gives them: `load-file-name', `load-true-file-name' and a
//! load history (`current-load-list') that starts with the file, which
//! `macroexp-file-name' reads, and so `ert-resource-directory' and
//! `ert-resource-file' (network-stream-tests.el's TLS server reads its key
//! and certificate through them; without the file it waited forever for a
//! server that could not start). Pinned GNU's own `load' of the same file
//! accepts the same check.
use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Command};

const CHECK: &str = r#"(if (and (stringp load-file-name)
         (equal load-true-file-name load-file-name)
         (equal current-load-list (list load-file-name))
         (string-match "target\\.el\\'" load-file-name))
    nil
  (error "load context %S %S %S" current-load-list load-true-file-name load-file-name))"#;

fn fixture() -> PathBuf {
    let base = std::env::var_os("TMPDIR").map(PathBuf::from).expect("lane TMPDIR");
    let path = base.join(format!("mini-load-context-{}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn mini_forms_see_the_load_context_gnu_load_gives() {
    let dir = fixture();
    let target = dir.join("target.el");
    fs::write(&target, format!(";;; -*- lexical-binding: t -*-\n{CHECK}\n")).unwrap();
    let harness = dir.join("fixture-harness.el");
    fs::write(&harness, "(defun romacs-gnu-tests--count () 0)\n").unwrap();
    let plan = dir.join("plan.json");
    fs::write(&plan, json!({"file": target, "forms": [{"ordinal": 1, "line": 2, "source": CHECK}]}).to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-gnu-ert"))
        .arg("--harness").arg(&harness).arg("--plan").arg(&plan)
        .output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let form: Vec<Value> = stdout.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["event"] == "form").collect();
    assert_eq!(form.len(), 1, "{stdout}");
    assert_eq!(form[0]["ok"], true, "{stdout}");

    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let gnu = Command::new(oracle).args(["-Q", "--batch", "-l"]).arg(&target).output().unwrap();
    assert!(gnu.status.success(), "{}", String::from_utf8_lossy(&gnu.stderr));
    let _ = fs::remove_dir_all(&dir);
}
