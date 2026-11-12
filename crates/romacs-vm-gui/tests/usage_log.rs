//! lisp/romacs/romacs-usage-log.el's ERT tests (fixtures/usage-log-test.el) in
//! Romacs's `--batch` session and in pinned GNU's: the library is GNU Lisp
//! on GNU's API, so both run every test and pass it.
use std::process::Command;

fn run(binary: &str, dir: &std::path::Path) -> (Option<i32>, String) {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let output = Command::new(binary)
        .args(["-Q", "--batch", "-l", &format!("{root}/lisp/romacs/romacs-usage-log.el"),
               "-l", concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/usage-log-test.el"),
               "-f", "ert-run-tests-batch-and-exit"])
        .current_dir(dir).env("HOME", dir).env("TMPDIR", dir).env("XDG_CACHE_HOME", dir.join("cache"))
        .env("ROMACS_WORLD", "off").env_remove("ROMACS_USAGE_LOG").output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr);
    let summary = text.lines().find(|line| line.starts_with("Ran ")).unwrap_or("no ERT summary").to_owned();
    // "Ran N tests, N results as expected, 0 unexpected (DATE, SECONDS)"
    (output.status.code(), summary.split(" (").next().unwrap().to_owned())
}

#[test]
fn usage_log_tests_pass_in_romacs_and_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let dir = std::env::temp_dir().join(format!("romacs-usage-log-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let expected = run(&gnu, &dir);
    let actual = run(env!("CARGO_BIN_EXE_romacs-vm-gui"), &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(expected, (Some(0), "Ran 11 tests, 11 results as expected, 0 unexpected".to_owned()));
    assert_eq!(actual, expected);
}
