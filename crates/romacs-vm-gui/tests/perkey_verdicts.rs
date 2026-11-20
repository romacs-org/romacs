//! The per-key harnesses' pure parts (harness/test_perkey_live.py,
//! harness/test_perkey_ab.py): a phase's validation and counts, and the
//! A/B verdict a check reads from perkey_ab.py's exit status.
use std::path::Path;
use std::process::Command;

#[test]
fn perkey_harness_verdicts() {
    let output = Command::new("python3")
        .args(["-m", "unittest", "-v", "test_perkey_live", "test_perkey_ab"])
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("harness"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success() && !stderr.contains("skipped"), "PERKEY_VERDICTS: {stderr}");
}
