//! fixtures/transpose-regions-test.el in Romacs's `--batch` session and in
//! pinned GNU's: both run every test and pass it.
use std::process::Command;

fn run(binary: &str, dir: &std::path::Path) -> (Option<i32>, String) {
    let output = Command::new(binary)
        .args(["-Q", "--batch", "-l", concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/transpose-regions-test.el"),
               "-f", "ert-run-tests-batch-and-exit"])
        .current_dir(dir).env("HOME", dir).env("TMPDIR", dir).env("XDG_CACHE_HOME", dir.join("cache"))
        .env("ROMACS_WORLD", "off").output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr);
    let summary = text.lines().find(|line| line.starts_with("Ran ")).unwrap_or("no ERT summary").to_owned();
    (output.status.code(), summary.split(" (").next().unwrap().to_owned())
}

#[test]
fn transpose_regions_tests_pass_in_romacs_and_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let dir = std::env::temp_dir().join(format!("romacs-transpose-regions-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let expected = run(&gnu, &dir);
    let actual = run(env!("CARGO_BIN_EXE_romacs-vm-gui"), &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(expected, (Some(0), "Ran 5 tests, 5 results as expected, 0 unexpected".to_owned()));
    assert_eq!(actual, expected);
}
