//! `romacs-vm-gui [-Q] --batch ARGS...` against pinned GNU's `emacs [-Q]
//! --batch ARGS...`: the command line packages spawn through
//! `invocation-name` (startup.el `command-line-1`'s `-L`, `-l`, `-f` and
//! `--eval`, `command-line-args-left`, `kill-emacs` and its hook, and the
//! batch report of an uncaught error). Each case runs both in the same
//! private HOME and directory and compares the exit status and stdout; an
//! error case compares the status and the message, stderr's last line,
//! since GNU's batch backtrace (on stdout) is VM2-ERRTEXT1.
use std::path::Path;
use std::process::Command;

const LIBRARY: &str = r#"(defun t-batch ()
  (princ (format "args=%S\n" command-line-args-left))
  (setq command-line-args-left nil)
  (kill-emacs 7))
(provide 't-batch)
"#;

struct Case { args: Vec<String>, stderr: bool }

fn cases(lib: &Path) -> Vec<Case> {
    let lib = lib.display().to_string();
    let case = |args: &[&str], stderr| Case { args: args.iter().map(|a| a.to_string()).collect(), stderr };
    vec![
        case(&["-Q", "--batch", "--eval",
               "(progn (add-hook 'kill-emacs-hook (lambda () (princ \"hook\\n\"))) (princ (format \"%S %S\\n\" noninteractive command-line-args-left)))",
               "-f", "ignore", "--eval", "(kill-emacs 3)"], false),
        case(&["-Q", "--batch", "-L", &lib, "-l", "t-batch", "-f", "t-batch", "one", "two"], false),
        case(&["-Q", "--batch", "-L", "lib", "--eval", "(progn (require 't-batch) (princ (featurep 't-batch)))"], false),
        case(&["--batch", "-Q", "--eval", "(error \"boom %d\" 1)"], true),
        case(&["-Q", "--batch"], false),
        // fileio.c's stream DEFUNs on the process's own streams: stdin
        // closed, stdout flushed and still written.
        case(&["-Q", "--batch", "--eval", "(progn (princ \"a\") (princ (list (set-binary-mode 'stdout t) (file--close-stream 'stdin) (unix-sync))))"], false),
        // files.el's save-buffer on a visit: the first save backs the
        // file up to bk~ and sets `buffer-backed-up'; the second keeps that
        // backup. The directory is under `temporary-file-directory', which
        // the default predicate exempts.
        case(&["-Q", "--batch", "--eval",
               "(let ((backup-enable-predicate #'always)) (with-temp-file \"bk\" (insert \"one\\n\")) (with-current-buffer (find-file-noselect \"bk\") (insert \"two\\n\") (save-buffer) (insert \"three\\n\") (save-buffer) (princ (list buffer-backed-up (local-variable-p 'buffer-backed-up) (with-temp-buffer (insert-file-contents \"bk~\") (buffer-string))))))"], false),
        // command-line's timestamps, which kill-emacs-hook functions read.
        case(&["-Q", "--batch", "--eval", "(princ (list (consp before-init-time) (time-less-p before-init-time after-init-time)))"], false),
    ]
}

fn run(binary: &str, dir: &Path, args: &[String]) -> (Option<i32>, String, String) {
    let output = Command::new(binary).args(args).current_dir(dir)
        .env("HOME", dir).env("XDG_CONFIG_HOME", dir.join("config")).env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_DATA_HOME", dir.join("data")).env("XDG_STATE_HOME", dir.join("state"))
        .output().unwrap();
    let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).into_owned();
    (output.status.code(), text(output.stdout), text(output.stderr))
}

#[test]
fn batch_command_line_matches_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = env!("CARGO_BIN_EXE_romacs-vm-gui");
    let dir = std::env::temp_dir().join(format!("romacs-batch-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("lib/t-batch.el"), LIBRARY).unwrap();
    // Each Romacs session runs the loadup chain first; run them side by side.
    let runs: Vec<_> = cases(&dir.join("lib")).into_iter().map(|case| {
        let (gnu, dir) = (gnu.clone(), dir.clone());
        std::thread::spawn(move || {
            let expected = run(&gnu, &dir, &case.args);
            let actual = run(romacs, &dir, &case.args);
            (case, expected, actual)
        })
    }).collect();
    let mut failures = Vec::new();
    for handle in runs {
        let (case, expected, actual) = handle.join().unwrap();
        let last = |stderr: &str| stderr.lines().last().unwrap_or("").to_owned();
        let shown = |(status, stdout, stderr): &(Option<i32>, String, String)|
            if case.stderr { (*status, String::new(), last(stderr)) } else { (*status, stdout.clone(), String::new()) };
        if shown(&expected) != shown(&actual) {
            failures.push(format!("{:?}\n  GNU    {:?}\n  Romacs {:?}\n  Romacs stderr {}", case.args, shown(&expected), shown(&actual), actual.2));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Startup from a HOME and directory whose parent holds more entries than a
/// listing bound: GNU asks no directory whether names fold case, so loadup
/// and startup's `abbreviate-file-name` answer as beside any other.
#[test]
fn batch_startup_beside_a_huge_directory_matches_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = env!("CARGO_BIN_EXE_romacs-vm-gui");
    let dir = std::env::temp_dir().join(format!("romacs-batch-huge-{}", std::process::id()));
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    for index in 0..4100 {
        std::fs::write(dir.join(format!("entry-{index}")), b"").unwrap();
    }
    let args: Vec<String> = ["-Q", "--batch", "--eval",
        "(princ (format \"%S\\n\" (list (file-name-case-insensitive-p default-directory) (abbreviate-file-name default-directory) (file-name-case-insensitive-p \"../entry-7\"))))"]
        .iter().map(|a| a.to_string()).collect();
    let expected = run(&gnu, &home, &args);
    let actual = run(romacs, &home, &args);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!((expected.0, expected.1.as_str()), (Some(0), "(nil \"~/\" nil)\n"), "GNU {expected:?}");
    assert_eq!((actual.0, actual.1.as_str()), (expected.0, expected.1.as_str()), "Romacs stderr {}", actual.2);
}
