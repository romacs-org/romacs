//! The parity ratchet, GNU-suite half: every GNU test file's passing deftests
//! are floors recorded in parity-ratchet.json (tools/lib/parity_ratchet.py). Each
//! shard fits in one batch of eight files under the gate's per-test timeout.
//! The adapter is the one Cargo just built.
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
        .arg(root.join("tools/lib/parity_ratchet.py"))
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

const GNU_ERT: &str = env!("CARGO_BIN_EXE_romacs-gnu-ert");

#[test]
fn parity_ratchet_gnu_suite_shard_1_of_71() {
    ratchet("gnu-shard:1/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_2_of_71() {
    ratchet("gnu-shard:2/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_3_of_71() {
    ratchet("gnu-shard:3/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_4_of_71() {
    ratchet("gnu-shard:4/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_5_of_71() {
    ratchet("gnu-shard:5/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_6_of_71() {
    ratchet("gnu-shard:6/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_7_of_71() {
    ratchet("gnu-shard:7/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_8_of_71() {
    ratchet("gnu-shard:8/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_9_of_71() {
    ratchet("gnu-shard:9/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_10_of_71() {
    ratchet("gnu-shard:10/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_11_of_71() {
    ratchet("gnu-shard:11/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_12_of_71() {
    ratchet("gnu-shard:12/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_13_of_71() {
    ratchet("gnu-shard:13/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_14_of_71() {
    ratchet("gnu-shard:14/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_15_of_71() {
    ratchet("gnu-shard:15/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_16_of_71() {
    ratchet("gnu-shard:16/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_17_of_71() {
    ratchet("gnu-shard:17/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_18_of_71() {
    ratchet("gnu-shard:18/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_19_of_71() {
    ratchet("gnu-shard:19/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_20_of_71() {
    ratchet("gnu-shard:20/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_21_of_71() {
    ratchet("gnu-shard:21/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_22_of_71() {
    ratchet("gnu-shard:22/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_23_of_71() {
    ratchet("gnu-shard:23/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_24_of_71() {
    ratchet("gnu-shard:24/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_25_of_71() {
    ratchet("gnu-shard:25/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_26_of_71() {
    ratchet("gnu-shard:26/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_27_of_71() {
    ratchet("gnu-shard:27/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_28_of_71() {
    ratchet("gnu-shard:28/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_29_of_71() {
    ratchet("gnu-shard:29/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_30_of_71() {
    ratchet("gnu-shard:30/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_31_of_71() {
    ratchet("gnu-shard:31/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_32_of_71() {
    ratchet("gnu-shard:32/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_33_of_71() {
    ratchet("gnu-shard:33/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_34_of_71() {
    ratchet("gnu-shard:34/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_35_of_71() {
    ratchet("gnu-shard:35/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_36_of_71() {
    ratchet("gnu-shard:36/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_37_of_71() {
    ratchet("gnu-shard:37/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_38_of_71() {
    ratchet("gnu-shard:38/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_39_of_71() {
    ratchet("gnu-shard:39/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_40_of_71() {
    ratchet("gnu-shard:40/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_41_of_71() {
    ratchet("gnu-shard:41/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_42_of_71() {
    ratchet("gnu-shard:42/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_43_of_71() {
    ratchet("gnu-shard:43/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_44_of_71() {
    ratchet("gnu-shard:44/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_45_of_71() {
    ratchet("gnu-shard:45/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_46_of_71() {
    ratchet("gnu-shard:46/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_47_of_71() {
    ratchet("gnu-shard:47/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_48_of_71() {
    ratchet("gnu-shard:48/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_49_of_71() {
    ratchet("gnu-shard:49/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_50_of_71() {
    ratchet("gnu-shard:50/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_51_of_71() {
    ratchet("gnu-shard:51/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_52_of_71() {
    ratchet("gnu-shard:52/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_53_of_71() {
    ratchet("gnu-shard:53/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_54_of_71() {
    ratchet("gnu-shard:54/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_55_of_71() {
    ratchet("gnu-shard:55/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_56_of_71() {
    ratchet("gnu-shard:56/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_57_of_71() {
    ratchet("gnu-shard:57/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_58_of_71() {
    ratchet("gnu-shard:58/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_59_of_71() {
    ratchet("gnu-shard:59/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_60_of_71() {
    ratchet("gnu-shard:60/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_61_of_71() {
    ratchet("gnu-shard:61/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_62_of_71() {
    ratchet("gnu-shard:62/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_63_of_71() {
    ratchet("gnu-shard:63/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_64_of_71() {
    ratchet("gnu-shard:64/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_65_of_71() {
    ratchet("gnu-shard:65/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_66_of_71() {
    ratchet("gnu-shard:66/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_67_of_71() {
    ratchet("gnu-shard:67/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_68_of_71() {
    ratchet("gnu-shard:68/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_69_of_71() {
    ratchet("gnu-shard:69/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_70_of_71() {
    ratchet("gnu-shard:70/71", "--gnu-ert", GNU_ERT);
}

#[test]
fn parity_ratchet_gnu_suite_shard_71_of_71() {
    ratchet("gnu-shard:71/71", "--gnu-ert", GNU_ERT);
}

/// The checker's own logic (floors, lowering records, zero-run refusals).
#[test]
fn parity_ratchet_checker_contract() {
    let root = root();
    let output = Command::new("python3")
        .arg(root.join("tools/tests/parity_ratchet_test.py"))
        .arg("-v")
        .env("ROMACS_GNU_ERT", GNU_ERT)
        .current_dir(&root)
        .output()
        .expect("python3 must be runnable");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "tools/tests/parity_ratchet_test.py failed (exit {:?}):\n{}\n{stderr}",
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
