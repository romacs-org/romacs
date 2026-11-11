//! The recording oracle is the pin, or nothing is recorded (R228).
//!
//! Every primh / bulk / varh evidence file carries the GNU header its recording
//! printed, including `revision` = the oracle binary's own
//! `emacs-repository-version`. Until this guard the recorder was
//! `/usr/bin/emacs`, built at b28750b822dc, 320 GNU commits older than the pin
//! `parity-reference.toml` declares, while the evidence's `gnu_sources` hashed
//! the pin's C files: every "recorded against live GNU" number named the wrong
//! reference.
//!
//! Two refusals, both inside the gate:
//!
//! * `evidence_is_recorded_against_the_pin`: every evidence file names the pin.
//!   There is no grandfathering: the 44 pre-pin files this guard first froze
//!   in a legacy ledger, and vm2/minibuf's 2, were re-recorded at the pin with
//!   every case unchanged (`tools/primh/oracle-drift/rerecord.py`), and the
//!   ledger was retired with them.
//! * `the_recorder_refuses_*`: `tools/lib/gnu_oracle.py`, through which
//!   `tools/primh/record.py` chooses its GNU, refuses an unset, relative or
//!   non-pin oracle, judged by what the binary itself reports.
use std::path::PathBuf;
use std::process::{Command, Output};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn primh() -> PathBuf {
    repo().join("crates/romacs-vm/tools/primh")
}

/// `mirror_commit` from parity-reference.toml: the one declared GNU pin.
fn pin() -> String {
    let text = std::fs::read_to_string(repo().join("parity-reference.toml")).unwrap();
    let line = text
        .lines()
        .find(|l| l.starts_with("mirror_commit"))
        .expect("parity-reference.toml declares mirror_commit");
    let commit = line.split('"').nth(1).unwrap().to_owned();
    assert_eq!(
        commit.len(),
        40,
        "mirror_commit is a full revision: {commit}"
    );
    commit
}

/// Every committed GNU recording primh, bulk and varh produce, by path under primh/.
fn recordings() -> Vec<String> {
    let mut out = Vec::new();
    for (dir, suffix) in [
        ("evidence", ".json"),
        ("bulk", ".corpus.json"),
        ("../varh/evidence", ".json"),
    ] {
        for entry in std::fs::read_dir(primh().join(dir)).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            if name.ends_with(suffix) {
                out.push(format!("{dir}/{name}"));
            }
        }
    }
    out.sort();
    out
}

fn revision(rel: &str) -> String {
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(primh().join(rel)).unwrap()).unwrap();
    v["gnu"]["revision"].as_str().unwrap_or("").to_owned()
}

/// Recordings that do not name the pin, for the tree as it is.
fn violations(pin: &str) -> Vec<String> {
    recordings()
        .into_iter()
        .filter_map(|rel| {
            let rev = revision(&rel);
            (rev != pin).then(|| format!("{rel}: recorded against {rev:?}, not the pin {pin}"))
        })
        .collect()
}

#[test]
fn evidence_is_recorded_against_the_pin() {
    let pin = pin();
    let bad = violations(&pin);
    assert!(
        bad.is_empty(),
        "{} recording(s) not from the pinned GNU oracle (record with tools/primh/record.py):\n{}",
        bad.len(),
        bad.join("\n")
    );
    eprintln!(
        "{} recordings, every one from the pin {}",
        recordings().len(),
        &pin[..12]
    );
}

// ---- the recorder's refusal, against fake oracles that report a revision ----

struct Fake(PathBuf);

impl Fake {
    fn new(name: &str, revision: &str) -> Fake {
        let dir = std::env::temp_dir().join(format!("gnu-oracle-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("emacs");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\nprintf '{{\"revision\":\"{revision}\",\"version\":\"32.0.50\"}}'\n"
            ),
        )
        .unwrap();
        let mut perm = std::fs::metadata(&bin).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut perm, 0o755);
        std::fs::set_permissions(&bin, perm).unwrap();
        Fake(bin)
    }
}

fn oracle(value: Option<&str>) -> Output {
    let mut cmd = Command::new("python3");
    cmd.arg(repo().join("tools/lib/gnu_oracle.py"))
        .arg("check")
        .env_remove("ROMACS_GNU_ORACLE");
    if let Some(v) = value {
        cmd.env("ROMACS_GNU_ORACLE", v);
    }
    cmd.output().expect("python3 runs tools/lib/gnu_oracle.py")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn the_recorder_refuses_an_oracle_that_reports_another_revision() {
    let old = Fake::new("old", "b28750b822dc45ffdceda3ea44a3c8b93f4e5a6b");
    let out = oracle(Some(old.0.to_str().unwrap()));
    assert!(
        !out.status.success() && text(&out).contains("REFUSED") && text(&out).contains(&pin()),
        "a non-pin oracle must be refused, naming the pin:\n{}",
        text(&out)
    );
}

#[test]
fn the_recorder_accepts_an_oracle_that_reports_the_pin() {
    let good = Fake::new("pin", &pin());
    let out = oracle(Some(good.0.to_str().unwrap()));
    assert!(
        out.status.success(),
        "the pin must be accepted:\n{}",
        text(&out)
    );
    assert!(
        text(&out).contains(&format!("\"revision\": \"{}\"", pin())),
        "{}",
        text(&out)
    );
}

/// A REAL GNU, not a fake: the identity probe must actually run in GNU -Q
/// --batch, and the verdict must follow the revision the binary reports.
/// (The fakes above print a canned reply and cannot catch a probe GNU itself
/// rejects, which is how a first draft refused every binary for the wrong
/// reason.) The gate exports its GNU as ROMACS_FORCE_ORACLE_PATH; today that is
/// the b28750b822dc system build, which must be refused BY REVISION.
#[test]
fn a_real_gnu_is_judged_by_the_revision_it_reports() {
    let gnu = std::env::var("ROMACS_FORCE_ORACLE_PATH").unwrap_or_else(|_| "/usr/bin/emacs".into());
    let reported = Command::new(&gnu)
        .args([
            "-Q",
            "--batch",
            "--eval",
            "(princ emacs-repository-version)",
        ])
        .output()
        .unwrap_or_else(|e| panic!("a real GNU at {gnu} must run: {e}"));
    let reported = String::from_utf8(reported.stdout).unwrap();
    let out = oracle(Some(&gnu));
    if reported == pin() {
        assert!(
            out.status.success(),
            "{gnu} reports the pin and must be accepted:\n{}",
            text(&out)
        );
    } else {
        assert!(
            !out.status.success()
                && text(&out).contains(&format!("reports emacs-repository-version '{reported}'")),
            "{gnu} reports {reported:?}; it must be refused for that revision:\n{}",
            text(&out)
        );
    }
}

#[test]
fn the_recorder_refuses_without_an_explicit_absolute_oracle() {
    // Unset: no default and no PATH lookup, even though PATH has an emacs.
    let unset = oracle(None);
    assert!(
        !unset.status.success() && text(&unset).contains("ROMACS_GNU_ORACLE is unset"),
        "an unset oracle must be refused:\n{}",
        text(&unset)
    );
    let relative = oracle(Some("emacs"));
    assert!(
        !relative.status.success() && text(&relative).contains("not an absolute path"),
        "a PATH-relative oracle must be refused:\n{}",
        text(&relative)
    );
}
