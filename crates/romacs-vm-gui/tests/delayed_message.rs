//! `funcall-with-delayed-message' and `with-delayed-message' against pinned
//! GNU in batch: FUNCTION's value, the type checks, and MESSAGE shown (batch
//! prints it to stderr) once FUNCTION has waited past TIMEOUT and not for a
//! call that ends first. Romacs shows it when FUNCTION waits, as a timer
//! runs (lisp/romacs-eval.el, romacs#245); these calls wait.
#[path = "support/installed.rs"]
mod installed;
use std::process::Command;

fn batch(binary: &str, dir: &std::path::Path, form: &str) -> (String, String) {
    let mut command = Command::new(binary);
    command.args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {form}))")]).current_dir(dir).env("HOME", dir).env("TMPDIR", dir);
    let output = command.output().unwrap();
    assert!(output.status.success(), "{form}: {}", String::from_utf8_lossy(&output.stderr));
    (String::from_utf8_lossy(&output.stdout).into_owned(),
     String::from_utf8_lossy(&output.stderr).lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n"))
}

#[test]
fn a_delayed_message_shows_as_gnu_shows_it() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = installed::romacs();
    let dir = std::env::temp_dir().join(format!("romacs-delayed-message-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let form = r#"(list (funcall-with-delayed-message 0.1 "slow-one" (lambda () (sleep-for 0.4) 'done))
                       (funcall-with-delayed-message 5 "never" (lambda () 42))
                       (condition-case e (funcall-with-delayed-message "x" "m" 'ignore) (error e))
                       (condition-case e (funcall-with-delayed-message 1 2 'ignore) (error e))
                       (with-delayed-message (0.1 "macro-slow") (sleep-for 0.3) 7)
                       ;; Saturated timeouts: infinity never, the rest at the first wait.
                       (funcall-with-delayed-message 1.0e+INF "inf" (lambda () (sleep-for 0.2) 1))
                       (funcall-with-delayed-message -1.0e+INF "ninf" (lambda () (sleep-for 0.2) 2))
                       (funcall-with-delayed-message 0.0e+NaN "nan" (lambda () (sleep-for 0.2) 3))
                       (funcall-with-delayed-message -1 "neg" (lambda () (sleep-for 0.2) 4))
                       (funcall-with-delayed-message 0 "zero" (lambda () (sleep-for 0.2) 5)))"#;
    let expected = batch(&gnu, &dir, form);
    assert_eq!(expected.1, "slow-one\nmacro-slow\nninf\nnan\nneg\nzero", "{expected:?}");
    assert_eq!(batch(romacs, &dir, form), expected);
    std::fs::remove_dir_all(&dir).unwrap();
}
