//! The parity ratchet, Lisp-census half: the GNU files that load under the
//! WARM (parity metric) and COLD (diagnostic) census are floors recorded in
//! parity-ratchet.json (tools/lib/parity_ratchet.py). The census binary is the one
//! Cargo just built for this package.
//!
//! Green only when the checker says PASS for exactly that unit AND reports
//! that the census measured files: an exit 0 with nothing measured is red.
use std::path::PathBuf;
use std::process::Command;

fn field(line: &str, name: &str) -> Option<usize> {
    let mut words = line.split_whitespace();
    while let Some(w) = words.next() {
        if w == name {
            return words.next().and_then(|v| v.parse().ok());
        }
    }
    None
}

fn ratchet(unit: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("python3")
        .arg(root.join("tools/lib/parity_ratchet.py"))
        .args(["check", "--accept-gains", "--unit", unit, "--census-bin", env!("CARGO_BIN_EXE_romacs-vm-census"), "--jobs", "8"])
        .current_dir(&root)
        .output()
        .expect("python3 must be runnable");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let verdict = format!("parity-ratchet: PASS {unit}:");
    let line = stdout.lines().find(|l| l.starts_with(&verdict));
    assert!(
        output.status.success() && line.is_some(),
        "parity ratchet {unit} is not PASS (exit {:?}). A FAIL names each lost \
         file; FAIL-WITH-INSTRUCTION means record the improvement:\n{stdout}\n{stderr}",
        output.status.code()
    );
    let line = line.unwrap();
    let denominator = field(line, "denominator").unwrap_or(0);
    let reported = field(line, "reported").unwrap_or(0);
    let floor = field(line, "floor").unwrap_or(0);
    assert!(
        denominator > 0 && reported > 0 && floor > 0,
        "parity ratchet {unit} printed PASS but measured nothing \
         (denominator {denominator}, reported {reported}, floor {floor}):\n{stdout}\n{stderr}"
    );
    require_evidence(unit, line, &stdout);
}

/// A PASS must stay auditable. nextest keeps nothing a passing test prints, so
/// the checker writes every verdict into the gate's run directory
/// (`ROMACS_GATE_EVIDENCE`, set by tools/gate); refuse to pass if this
/// unit's verdict line is not there. Outside the gate, print the verdict.
fn require_evidence(unit: &str, line: &str, stdout: &str) {
    for l in stdout.lines().filter(|l| l.starts_with("parity-ratchet:")) {
        println!("{l}");
    }
    if let Some(path) = std::env::var_os("ROMACS_GATE_EVIDENCE") {
        let kept = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            kept.lines().any(|l| l == line),
            "parity ratchet {unit} passed but left no verdict in {}: a pass \
             nobody can audit is not evidence:\n{stdout}",
            std::path::Path::new(&path).display()
        );
    }
}

#[test]
fn parity_ratchet_census_warm() {
    ratchet("census-warm");
}

#[test]
fn parity_ratchet_census_cold() {
    ratchet("census-cold");
}
