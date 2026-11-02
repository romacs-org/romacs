//! The batch scaling probes (harness/scaling/probes.el, run by
//! harness/scaling_live.py) against this build: every subsystem probe's
//! empirical exponent, its work at K*N against its work at N in fresh
//! sessions, within its budget, or reported as known, ticketed debt. A
//! per-item derivation over a growing collection fails here, with the
//! probe and its exponent, instead of surfacing as a slow session. The
//! harness's pure parts (exponents, verdicts, overrides) run first.
use std::path::Path;
use std::process::Command;

#[test]
fn scaling_harness_verdicts() {
    let output = Command::new("python3")
        .args(["-m", "unittest", "-v", "test_scaling_live"])
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("harness"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success() && !stderr.contains("skipped"), "SCALING_VERDICTS: {stderr}");
}

#[test]
fn batch_probes_scale_within_their_budgets() {
    let out = std::env::temp_dir().join(format!("romacs-scaling-{}", std::process::id()));
    let output = Command::new("python3")
        .arg("-B")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("harness/scaling_live.py"))
        .args(["--binary", env!("CARGO_BIN_EXE_romacs-vm-gui"), "--mode", "batch", "--output"])
        .arg(&out)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(out.join("home"));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success() && stdout.contains("PASS in"),
        "SCALING_PROBES (report in {}):\n{stdout}{}", out.display(), String::from_utf8_lossy(&output.stderr));
    let _ = std::fs::remove_dir_all(&out);
}
