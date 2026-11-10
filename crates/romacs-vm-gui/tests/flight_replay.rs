//! The flight recorder (romacs-vm src/flight.rs) end to end. A batch
//! session is recorded (`ROMACS_FLIGHT=record:LOG`), then replayed in a
//! fresh process from the same boot, the source chain here, with its
//! directory restored as it was (`ROMACS_FLIGHT=replay:LOG`). The replay
//! reports no divergence and prints what the recording printed: the clock,
//! entropy, a file it wrote and copied and that file's time, which the
//! kernel's clock stamped. With a byte of its input file changed, the replay's report
//! names the file and the step it was read in.
use std::path::Path;
use std::process::Command;

const SESSION: &str = r#"(setq create-lockfiles nil make-backup-files nil
      t-out (expand-file-name "work/out.txt") t-copy (expand-file-name "work/copy.txt"))
(find-file "work/in.txt")
(execute-kbd-macro (kbd "C-e x C-n y"))
(let ((text (buffer-string))) (with-temp-file t-out (insert text)))
(copy-file t-out t-copy t)
(princ (format "%S %S %S %S\n" (random 1000000) (float-time) (buffer-string)
               (file-attribute-modification-time (file-attributes t-out))))
"#;

/// The session's directory as every run starts from it: its input file,
/// it, and the directory it runs in, with one fixed time, since a stat of
/// any is logged; and restored in place, so each keeps its inode. What a
/// run made is removed.
fn restore(dir: &Path, input: &str) {
    use std::io::Write;
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    for entry in std::fs::read_dir(&work).unwrap().flatten() {
        if entry.file_name() != "in.txt" {
            std::fs::remove_file(entry.path()).unwrap();
        }
    }
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(work.join("in.txt")).unwrap();
    file.write_all(input.as_bytes()).unwrap();
    let fixed = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    for path in [work.join("in.txt"), work.clone(), dir.to_owned()] {
        std::fs::File::open(&path).unwrap().set_modified(fixed).unwrap();
    }
}

/// The session, recording or replaying (ROLE) the log in DIR: its exit
/// status, its standard output, and its report.
fn run(dir: &Path, role: &str) -> (Option<i32>, String, serde_json::Value) {
    let log = dir.join("logs/session.flight");
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(["--batch", "-l", &dir.join("session.el").display().to_string()]).current_dir(dir)
        .env_clear().env("PATH", "/usr/bin:/bin").env("LANG", "C.UTF-8").env("TZ", "UTC")
        .env("HOME", dir.join("home")).env("XDG_CONFIG_HOME", dir.join("home/config")).env("XDG_CACHE_HOME", dir.join("home/cache"))
        .env("XDG_DATA_HOME", dir.join("home/data")).env("XDG_STATE_HOME", dir.join("home/state"))
        .env("ROMACS_WORLD", "off").env("ROMACS_FLIGHT", format!("{role}:{}", log.display()))
        .output().unwrap();
    let report = std::fs::read_to_string(log.with_extension(format!("{role}.json"))).unwrap_or_default();
    (output.status.code(), String::from_utf8_lossy(&output.stdout).into_owned(),
     serde_json::from_str(&report).unwrap_or(serde_json::Value::Null))
}

#[test]
fn a_recorded_session_replays_and_a_changed_input_is_named() {
    let dir = std::env::temp_dir().join(format!("romacs-flight-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("logs")).unwrap();
    std::fs::create_dir_all(dir.join("home")).unwrap();
    std::fs::write(dir.join("session.el"), SESSION).unwrap();

    restore(&dir, "alpha\nbeta\n");
    let (status, recorded, report) = run(&dir, "record");
    assert_eq!(status, Some(0), "record: {recorded} {report}");
    assert!(report["steps"].as_u64().unwrap_or(0) > 4 && report["host_calls"].as_u64().unwrap_or(0) > 0, "{report}");

    restore(&dir, "alpha\nbeta\n");
    let (status, replayed, report) = run(&dir, "replay");
    assert_eq!(status, Some(0), "replay: {replayed} {report}");
    assert_eq!(report["divergence"], serde_json::Value::Null, "{report}");
    assert_eq!(replayed, recorded, "the replay printed what the recording printed");

    restore(&dir, "alpha\nBeta\n");
    let (status, _, report) = run(&dir, "replay");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(status, Some(0));
    let divergence = &report["divergence"];
    assert_eq!(divergence["kind"], "host", "{report}");
    assert!(divergence["found"].as_str().is_some_and(|found| found.ends_with("work/in.txt")), "{report}");
    assert!(divergence["after"].as_str().is_some_and(|after| after.ends_with("session.el:1")), "the step before find-file: {report}");
}
