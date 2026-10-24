//! The parity ratchet, GNU-suite half: every GNU test file's passing deftests
//! are floors recorded in parity-ratchet.json (tools/parity_ratchet.py). Four
//! shards so each stays far under the gate's per-test timeout and a red names
//! the files it covers. The adapter is the one Cargo just built.
//!
//! A shard is green only when the checker says PASS for exactly that unit AND
//! reports that romacs ran tests: an exit 0 with nothing measured is red.
use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `parity-ratchet: PASS <unit>: measured N floor N units U denominator D reported R seconds S`
fn field(line: &str, name: &str) -> Option<usize> {
    let mut words = line.split_whitespace();
    while let Some(w) = words.next() {
        if w == name {
            return words.next().and_then(|v| v.parse().ok());
        }
    }
    None
}

fn ratchet(unit: &str, flag: &str, binary: &str) {
    let root = root();
    let output = Command::new("python3")
        .arg(root.join("tools/parity_ratchet.py"))
        .args(["check", "--accept-gains", "--unit", unit, flag, binary, "--jobs", "8"])
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
         member; FAIL-WITH-INSTRUCTION means record the improvement:\n{stdout}\n{stderr}",
        output.status.code()
    );
    let line = line.unwrap();
    let units = field(line, "units").unwrap_or(0);
    let denominator = field(line, "denominator").unwrap_or(0);
    let reported = field(line, "reported").unwrap_or(0);
    assert!(
        units > 0 && denominator > 0 && reported > 0,
        "parity ratchet {unit} printed PASS but measured nothing \
         (units {units}, denominator {denominator}, reported {reported}):\n{stdout}\n{stderr}"
    );
    require_evidence(unit, line, &stdout);
}

/// A PASS must stay auditable. nextest keeps nothing a passing test prints, so
/// the checker writes every verdict into the gate's run directory
/// (`ROMACS_GATE_EVIDENCE`, set by scripts/gate); refuse to pass if this
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

const GNU_ERT: &str = env!("CARGO_BIN_EXE_romacs-gnu-ert");

#[test]
fn parity_ratchet_gnu_suite_shard_1_of_4() {
    ratchet("gnu-shard:1/4", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_2_of_4() {
    ratchet("gnu-shard:2/4", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_3_of_4() {
    ratchet("gnu-shard:3/4", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_4_of_4() {
    ratchet("gnu-shard:4/4", "--gnu-ert", GNU_ERT);
}

/// The checker's own logic (floors, lowering records, zero-run refusals).
#[test]
fn parity_ratchet_checker_contract() {
    let root = root();
    let output = Command::new("python3")
        .arg(root.join("tools/parity_ratchet_test.py"))
        .arg("-v")
        .env("ROMACS_GNU_ERT", GNU_ERT)
        .current_dir(&root)
        .output()
        .expect("python3 must be runnable");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "tools/parity_ratchet_test.py failed (exit {:?}):\n{}\n{stderr}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout)
    );
    let ran = stderr
        .lines()
        .find_map(|l| l.strip_prefix("Ran ").and_then(|r| r.split_whitespace().next()))
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    assert!(ran > 0, "no checker test ran:\n{stderr}");
    // unittest marks a skipped test "... skipped '<reason>'" and counts it in
    // the summary as "skipped=N"; test NAMES may contain the word.
    assert!(
        !stderr.contains(" ... skipped") && !stderr.contains("skipped="),
        "a checker test was skipped:\n{stderr}"
    );
}
