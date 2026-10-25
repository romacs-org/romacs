//! Puts the GNU-suite runner's own tests (gnu-tests/tests/test_run.py) under
//! the gate.  Each class runs separately so a red names what broke.  The
//! adapter under test is the one Cargo just built for this package.
use std::path::PathBuf;
use std::process::Command;

fn python_class(class: &str) {
    python_contract("test_run.py", class);
}

fn python_contract(script: &str, class: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("python3")
        .arg(root.join("gnu-tests/tests").join(script))
        .arg("-v")
        .arg(class)
        .env("ROMACS_GNU_ERT", env!("CARGO_BIN_EXE_romacs-gnu-ert"))
        .current_dir(&root)
        .output()
        .expect("python3 must be runnable");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{class} failed (exit {:?}):\n{}\n{stderr}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout)
    );
    // unittest exits 0 for a class with no tests; require that tests RAN and
    // that none were skipped, so a renamed class cannot turn this green.
    let ran = stderr
        .lines()
        .find_map(|l| {
            l.strip_prefix("Ran ")
                .and_then(|r| r.split_whitespace().next())
        })
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    assert!(ran > 0, "{class}: no test ran:\n{stderr}");
    assert!(
        !stderr.contains("skipped"),
        "{class}: a test was skipped:\n{stderr}"
    );
}

#[test]
fn runner_classification_contract() {
    python_class("Classification");
}

#[test]
fn runner_exclusion_ledger_contract() {
    python_class("Ledger");
}

#[test]
fn runner_pristine_import_contract() {
    python_class("Pin");
}

#[test]
fn runner_end_to_end_contract() {
    python_class("EndToEnd");
}

#[test]
fn real_ert_parser_contract() {
    python_contract("test_ert_report.py", "Parser");
}

#[test]
fn real_ert_native_contract() {
    python_contract("test_ert_report.py", "Native");
}

#[test]
fn real_ert_romacs_contract() {
    python_contract("test_real_romacs.py", "RealRomacs");
}

#[test]
fn real_ert_batch_adapter_contract() {
    python_contract("test_ert_adapter.py", "BatchAdapter");
}

#[test]
fn real_ert_scoreboard_contract() {
    python_contract("test_scoreboard.py", "Scoreboard");
}

#[test]
fn subject_vacuity_classifier_contract() {
    python_contract("test_vacuity_classifier.py", "ClassifierTests");
    python_contract("test_vacuity_classifier.py", "BindingTests");
}

#[test]
fn run_local_evidence_contract() {
    python_contract("test_evidence_local.py", "EvidenceLocalTests");
    python_contract("test_evidence_local.py", "TreeEvidenceLocalTests");
}

#[test]
fn source_manifest_contract() {
    python_contract("test_audit_manifest.py", "LegacyRefusal");
    python_contract("test_audit_manifest.py", "ManifestTests");
    python_contract("test_audit_manifest.py", "FocusedImporterTests");
}

#[test]
fn gate_subject_preparation_contract() {
    python_contract("test_gate_vacuity.py", "GateVacuityTests");
}
