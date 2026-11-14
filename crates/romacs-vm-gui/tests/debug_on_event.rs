//! `debug-on-event' (keyboard.c handle_user_signal): SIGUSR2 sent to a
//! session whose evaluator is busy enters the debugger at once, as C-g
//! quits there: in a loop that reads no input, inside a long regexp search
//! (the matcher's quit probe) and in a process wait (`accept-process-output'
//! with nothing to read); it is not left as an event for an input read that
//! never comes. Unlike C-g it quits under `inhibit-quit' too, which the
//! handler clears.
#[path = "support/installed.rs"]
mod installed;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Run BODY after `debug-on-event' and a debugger that prints its
/// arguments and exits; SIGUSR2 once BODY has begun. The debugger's line.
fn debugger_line(name: &str, body: &str) -> String { debugger_line_after(name, "", body) }

/// `debugger_line' with SETUP run before the session counts as begun.
fn debugger_line_after(name: &str, setup: &str, body: &str) -> String {
    let dir = std::env::temp_dir().join(format!("romacs-debug-on-event-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = format!(r#"(setq debug-on-event 'sigusr2)
(setq debugger (lambda (&rest args) (princ (format "DEBUGGER %S\n" args)) (kill-emacs 0)))
{setup}
(with-temp-file (expand-file-name "busy" (getenv "MARK")) (insert "busy"))
{body}
(princ "ENDED\n")
"#);
    std::fs::write(dir.join("busy.el"), source).unwrap();
    let mut child = Command::new(installed::romacs())
        .args(["-Q", "--batch", "-l"]).arg(dir.join("busy.el")).current_dir(&dir)
        .env("HOME", &dir).env("MARK", &dir)
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
fn sigusr2_breaks_a_loop_and_a_search_under_inhibit_quit() {
    assert_eq!(debugger_line("inhibited-loop", "(let ((inhibit-quit t) (n 0)) (while t (setq n (1+ n))))"),
               "DEBUGGER (error (quit))");
    assert_eq!(debugger_line("inhibited-regexp",
                             r#"(let ((inhibit-quit t)) (string-match "^\\(\\(a\\)\\2?\\)*$" (concat (make-string 48 ?a) "b")))"#),
               "DEBUGGER (error (quit))");
}

#[test]
fn sigusr2_breaks_a_long_motion_into_the_debugger() {
    // Nothing polls in Lisp here: the signal lands inside the one call.
    assert_eq!(debugger_line_after("motion", r#"(switch-to-buffer (generate-new-buffer "big"))
(insert (make-string 100000000 ?\n))
(goto-char 1)"#, "(vertical-motion 100000000)"), "DEBUGGER (error (quit))");
}

#[test]
fn sigusr2_breaks_text_that_shows_as_nothing() {
    // One element's generation walks the whole buffer, loading nothing.
    assert_eq!(debugger_line_after("hidden", r#"(require 'disp-table)
(switch-to-buffer (generate-new-buffer "big"))
(insert (make-string 300000000 ?x))
(setq buffer-display-table (make-display-table))
(aset buffer-display-table ?x [])
(goto-char 1)"#, "(vertical-motion 1)"), "DEBUGGER (error (quit))");
}

#[test]
fn sigusr2_breaks_a_huge_replacement_string() {
    assert_eq!(debugger_line_after("replacement", r#"(switch-to-buffer (generate-new-buffer "big"))
(insert "ab")
(put-text-property 1 2 'display (make-string 30000000 ?y))
(goto-char 1)"#, "(vertical-motion 1)"), "DEBUGGER (error (quit))");
}

#[test]
fn sigusr2_breaks_a_process_wait_into_the_debugger() {
    assert_eq!(debugger_line("process", r#"(accept-process-output (start-process "quiet" nil "sleep" "60") 60)"#),
               "DEBUGGER (error (quit))");
}
