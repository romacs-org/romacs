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
