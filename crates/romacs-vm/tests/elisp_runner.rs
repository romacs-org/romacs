//! The source-loaded Elisp test runner (`tools/test-elisp`) against the
//! romacs-vm host Cargo just built: `tools/tests/test_elisp_runner.py` needs a
//! built host, which check-derived's runner cannot assume, so this test owns
//! it (`ELSEWHERE` in `tools/lib/python_tests.py`).
use std::path::Path;
use std::process::Command;

#[test]
fn the_elisp_runner_reports_and_exits_as_its_tests_did() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/tests");
    let output = Command::new("python3")
        .args(["-B", "-m", "unittest", "-v", "test_elisp_runner"])
        .current_dir(&tests)
        .env("ROMACS_CORE_TEST_BINARY", env!("CARGO_BIN_EXE_romacs-vm"))
        .output()
        .expect("python3 must be runnable");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let ran = stderr.lines().find_map(|l| l.strip_prefix("Ran ").and_then(|r| r.split_whitespace().next()))
        .and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
    assert!(output.status.success() && ran > 0 && !stderr.contains("skipped"), "test_elisp_runner:\n{stderr}");
}
