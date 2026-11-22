//! The launch markers (`romacs_vm::host::LAUNCH_MARKERS`) are a process's
//! own: its Lisp does not see them, and no child it starts inherits them,
//! whichever way it starts one (`call-process`, `make-process` on a pipe or
//! a pty, the terminal's shell). A variable beside them passes through, so
//! each child's environment is read.
#[path = "support/installed.rs"]
mod installed;
#[path = "../../romacs-vm/tests/support/own_test.rs"]
mod own_test;

use romacs_vm::TerminalHost;
use romacs_vm_gui::terminal_service::Service;
use std::time::{Duration, Instant};

/// Markers that change nothing in a `--batch` session that builds no world,
/// and the variable that must pass.
const MARKERS: [&str; 2] = ["ROMACS_WORLD_LOCK_FD", "ROMACS_COMPILE_PARENT"];
const PROBE: &str = "ROMACS_MARKER_PROBE=passes";

/// Each child's environment, after a line naming how it was started.
const CHILDREN: &str = r#"(let ((read (lambda (type)
                (let* ((out "")
                       (p (make-process :name "env" :command '("/usr/bin/env") :connection-type type
                                        :filter (lambda (_ s) (setq out (concat out s))))))
                  (while (process-live-p p) (accept-process-output p 0.1))
                  (accept-process-output nil 0.1)
                  out))))
  (princ (concat "getenv " (prin1-to-string (list (getenv "ROMACS_WORLD_LOCK_FD") (getenv "ROMACS_COMPILE_PARENT"))) "\n"
                 "== call-process\n" (with-temp-buffer (call-process "/usr/bin/env" nil t) (buffer-string))
                 "== pipe\n" (funcall read 'pipe)
                 "== pty\n" (funcall read 'pty))))"#;

/// The set markers' names and the probe, as `env` lines.
fn with_markers(command: &mut std::process::Command) -> &mut std::process::Command {
    let (probe, value) = PROBE.split_once('=').unwrap();
    command.env(MARKERS[0], "999").env(MARKERS[1], std::process::id().to_string()).env(probe, value)
}

/// Whether the child environment TEXT has the probe and no marker.
/// A pty's lines end in a carriage return too.
fn clean(what: &str, text: &str) {
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    assert!(lines.contains(&PROBE), "{what}: no {PROBE}, so its environment was not read: {text}");
    for marker in MARKERS {
        assert!(!lines.iter().any(|l| l.starts_with(&format!("{marker}="))), "{what} inherited {marker}: {text}");
    }
}

#[test]
fn lisp_and_its_children_see_no_launch_marker() {
    let home = std::env::temp_dir().join(format!("romacs-launch-markers-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let output = with_markers(&mut std::process::Command::new(installed::romacs()))
        .args(["--batch", "--eval", CHILDREN]).env("HOME", &home).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.starts_with("getenv (nil nil)\n"), "Lisp sees a launch marker: {text}");
    let parts: Vec<&str> = text.split("== ").collect();
    assert_eq!(parts.len(), 4, "{text}");
    for part in &parts[1..] {
        let (what, env) = part.split_once('\n').unwrap();
        clean(what, env);
    }
}

/// Set in the test binary run again, with the markers, for the terminal's
/// shell: the host process is then this one.
const IN_CHILD: &str = "ROMACS_MARKER_TERMINAL_CHILD";

#[test]
fn the_terminal_shell_inherits_no_launch_marker() {
    if std::env::var_os(IN_CHILD).is_none() {
        let test = own_test::name(module_path!(), "the_terminal_shell_inherits_no_launch_marker");
        let output = with_markers(&mut std::process::Command::new(std::env::current_exe().unwrap()))
            .args(["--exact", &test, "--nocapture", "--test-threads", "1"]).env(IN_CHILD, "1").output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
        // A name that selected nothing would pass with no test run.
        assert!(stdout.contains("test result: ok. 1 passed"), "the child ran no test: {stdout}");
        return;
    }
    let mut service = Service::new();
    let id = service.create("/bin/sh", "/", 200, 60).unwrap();
    service.send(id, b"stty -echo; env | grep ^ROMACS_; exit\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let snapshot = loop {
        let snapshot = service.snapshot(id).unwrap();
        if !snapshot.running {
            break snapshot;
        }
        assert!(Instant::now() < deadline, "PTY timeout: {snapshot:?}");
        std::thread::sleep(Duration::from_millis(10));
    };
    clean("the terminal's shell", &snapshot.text);
}
