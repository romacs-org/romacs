//! Static candidate ratchet guards run without building or probing an oracle.
use std::path::PathBuf;
use std::process::Command;

#[test]
fn candidate_ratchet_contract() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("python3")
        .arg(root.join("crates/romacs-gnu-tests/candidate_ratchet_contract.py"))
        .arg("-v")
        .current_dir(&root)
        .output()
        .expect("python3 must be runnable");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "candidate ratchet contract failed:\n{stdout}\n{stderr}"
    );
    let ran = stderr
        .lines()
        .find_map(|line| {
            line.strip_prefix("Ran ")
                .and_then(|s| s.split_whitespace().next())
        })
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    assert_eq!(ran, 10, "candidate contract test count changed:\n{stderr}");
    assert!(
        !stderr.contains(" ... skipped") && !stderr.contains("skipped="),
        "candidate contract skipped a test:\n{stderr}"
    );
}
