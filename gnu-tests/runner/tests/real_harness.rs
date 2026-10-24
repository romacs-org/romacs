//! Plumbing fixtures are deliberately NOT evidence that GNU ERT can load.
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let base = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .expect("lane TMPDIR");
        let path = base.join(format!(
            "real-harness-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn put(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, text).unwrap();
        path
    }
    fn run(&self, harness: &str, target: &str, reporter: &str) -> Output {
        let h = self.put("fixture-harness.el", harness);
        let t = self.put("target.el", target);
        let r = self.put("reporter.el", reporter);
        let plan = self.put("plan.json", &json!({"file":t,"forms":[{"ordinal":1,"source":"(error \"PLAN-FORMS-MUST-NOT-RUN\")"}]}).to_string());
        Command::new(env!("CARGO_BIN_EXE_romacs-gnu-ert"))
            .args(["--harness-kind", "real", "--harness"])
            .arg(h)
            .arg("--plan")
            .arg(plan)
            .arg("--reporter")
            .arg(r)
            .output()
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}
fn events(output: &Output) -> Vec<Value> {
    stdout(output)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
const HARNESS: &str = r#"
(defun romacs-gnu-tests--count () (error "MINI-ACCESSOR-MUST-NOT-RUN"))
(defun ert-run-tests-batch (selector)
  (if (and target-loaded (null load-in-progress))
      (progn (message "NATIVE-FIXTURE-SUMMARY") 17)
    (error "TARGET-LOAD-SEMANTICS")))
"#;
#[test]
fn real_mode_loads_target_and_runs_reporter_without_mini_accessors() {
    let f = Fixture::new();
    let output = f.run(
        HARNESS,
        "(setq target-loaded (and load-in-progress load-file-name))",
        "(setq fixture-stats (let ((load-in-progress nil)) (ert-run-tests-batch t)))\n(if (= fixture-stats 17) 0 1)",
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "REAL-PLUMBING: real mode must load target and reporter, never mini accessors: {} {:?}",
        stdout(&output),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout(&output).contains("NATIVE-FIXTURE-SUMMARY"),
        "REAL-OUTPUT: VM message output must reach stdout"
    );
    assert!(
        !stdout(&output).contains("MINI-ACCESSOR"),
        "REAL-ISOLATION: mini accessor was invoked"
    );
}
#[test]
fn real_mode_stops_on_target_load_error() {
    let f = Fixture::new();
    let output = f.run(
        HARNESS,
        "(error \"TARGET-LOAD-STOP\")\n(message \"AFTER-FAILED-FORM\")",
        "(message \"REPORTER-MUST-NOT-RUN\")\n0",
    );
    assert_eq!(
        output.status.code(),
        Some(4),
        "REAL-LOAD-STOP: target load error must stop before reporter: {}",
        stdout(&output)
    );
    let rows = events(&output);
    assert!(
        rows.iter().any(|v| v["phase"] == "test-load"
            && v["error"]
                .as_str()
                .is_some_and(|e| e.contains("TARGET-LOAD-STOP"))),
        "REAL-LOAD-ERROR: missing original target load diagnostic: {}",
        stdout(&output)
    );
    assert!(
        !stdout(&output).contains("REPORTER-MUST-NOT-RUN")
            && !stdout(&output).contains("AFTER-FAILED-FORM"),
        "REAL-LOAD-ORDER: continued after failed load"
    );
}
#[test]
fn real_mode_preserves_reporter_failure_status() {
    let f = Fixture::new();
    let output = f.run(HARNESS, "(setq target-loaded t)", "1");
    assert_eq!(
        output.status.code(),
        Some(1),
        "REAL-STATUS: unexpected native results must exit 1: {}",
        stdout(&output)
    );
}

#[test]
fn ordinary_load_defers_registration_body_until_execution() {
    let f = Fixture::new();
    let harness = include_str!("../../harness/romacs-mini-ert.el");
    let definition = "(defmacro timing-explode () (error \"BODY-MACRO-EXPANDED\"))";
    let registration = "(ert-deftest timing-deferred () (timing-explode))";
    // This ordinary-load control uses the miniature harness, not GNU ERT.
    let ordinary = f.run(
        harness,
        &format!("{definition}\n{registration}"),
        r#"
(if (not (= (romacs-gnu-tests--count) 1)) (error "NOT-REGISTERED"))
(message "REGISTERED-BEFORE-RUN")
(romacs-gnu-tests--run 0)
(if (eq (car romacs-gnu-tests--last) 'errored)
    (progn (message "FAILED-ONLY-AT-RUN: %S" romacs-gnu-tests--last) 0)
  (error "BODY-DID-NOT-FAIL"))
"#,
    );
    assert_eq!(
        ordinary.status.code(),
        Some(0),
        "ORDINARY-LOAD-TIMING: {}",
        stdout(&ordinary)
    );
    assert!(stdout(&ordinary).contains("REGISTERED-BEFORE-RUN"));
    assert!(stdout(&ordinary).contains("FAILED-ONLY-AT-RUN:"));
    assert!(stdout(&ordinary).contains("BODY-MACRO-EXPANDED"));
}
