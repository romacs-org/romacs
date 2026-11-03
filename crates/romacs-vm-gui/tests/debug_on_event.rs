//! `debug-on-event' (keyboard.c handle_user_signal): SIGUSR2 sent to a
//! session whose evaluator is busy enters the debugger at once, as C-g
//! quits there: in a loop that reads no input, inside a long regexp search
//! (the matcher's quit probe) and in a process wait (`accept-process-output'
//! with nothing to read); it is not left as an event for an input read that
//! never comes.
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Run BODY after `debug-on-event' and a debugger that prints its
/// arguments and exits; SIGUSR2 once BODY has begun. The debugger's line.
fn debugger_line(name: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("romacs-debug-on-event-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!(r#"(setq debug-on-event 'sigusr2)
(setq debugger (lambda (&rest args) (princ (format "DEBUGGER %S\n" args)) (kill-emacs 0)))
(with-temp-file (expand-file-name "busy" (getenv "MARK")) (insert "busy"))
{body}
(princ "ENDED\n")
"#);
    std::fs::write(dir.join("busy.el"), source).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(["-Q", "--batch", "-l"]).arg(dir.join("busy.el")).current_dir(&dir)
        .env("HOME", &dir).env("MARK", &dir).env("ROMACS_WORLD", "off")
        .stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let start = Instant::now();
    while !dir.join("busy").exists() {
        assert!(start.elapsed() < Duration::from_secs(60), "{name}: the session never began");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(Command::new("kill").args(["-USR2", &child.id().to_string()]).status().unwrap().success());
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() { break status; }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("{name}: SIGUSR2 did not enter the debugger");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take().unwrap(), &mut out).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(status.success(), "{name}: {status}: {out}");
    out.trim().to_owned()
}

#[test]
fn sigusr2_breaks_a_busy_loop_into_the_debugger() {
    assert_eq!(debugger_line("loop", "(let ((n 0)) (while t (setq n (1+ n))))"), "DEBUGGER (error (quit))");
}

#[test]
fn sigusr2_breaks_a_backtracking_search_into_the_debugger() {
    // Minutes of backtracking (the backreference defeats the matcher's
    // memo): every way to split the a's fails at the b.
    assert_eq!(debugger_line("regexp", r#"(string-match "^\\(\\(a\\)\\2?\\)*$" (concat (make-string 48 ?a) "b"))"#),
               "DEBUGGER (error (quit))");
}

#[test]
fn sigusr2_breaks_a_process_wait_into_the_debugger() {
    assert_eq!(debugger_line("process", r#"(accept-process-output (start-process "quiet" nil "sleep" "60") 60)"#),
               "DEBUGGER (error (quit))");
}
