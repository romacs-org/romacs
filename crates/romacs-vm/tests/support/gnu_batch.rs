//! Pinned GNU's output for many `--eval` programs from one batch process.
//!
//! An oracle test that compares Romacs with pinned GNU case by case ran
//! `emacs -Q --batch --eval PROGRAM` once a case, hundreds of processes in
//! a row, each spending most of its time starting. Here one process reads
//! the programs from a file and evaluates each as `--eval` does (read, then
//! `eval` with lexical binding), writing a mark after its output, so each
//! program's standard output comes back as its own. A program that signals
//! has its error as its output, where its own process would have exited
//! with it. Each program sees the state the ones before it left: a case
//! whose program changes global state keeps a process of its own.
use std::process::Command;

/// What ends each program's output, no program printing it; and how the
/// driver writes it, as a Lisp string.
const MARK: &str = "\n\u{1f}romacs-gnu-batch\u{1f}\n";
const MARK_LISP: &str = r#""\n\^_romacs-gnu-batch\^_\n""#;

/// Pinned GNU's standard output for each of PROGRAMS, in order.
pub fn outputs(programs: &[String]) -> Vec<String> {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    // Each test module of the folded target holds a copy of this file, and
    // tests run on threads of one process: the thread names the directory,
    // as one thread makes one call at a time and removes its directory.
    let dir = std::env::temp_dir().join(format!("romacs-gnu-batch-{}-{:?}", std::process::id(), std::thread::current().id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("programs");
    assert!(programs.iter().all(|p| !p.contains('\0')), "a program holds a NUL, which separates them");
    std::fs::write(&file, programs.join("\0")).unwrap();
    let driver = format!(
        "(dolist (program (split-string (with-temp-buffer (let ((coding-system-for-read 'utf-8)) (insert-file-contents {file:?})) (buffer-string)) \"\\0\"))
           (condition-case err (eval (car (read-from-string program)) t)
             (error (princ (format \"GNU signalled %S\" err))))
           (princ {MARK_LISP}))",
        file = file.to_str().unwrap());
    let output = Command::new(&binary).args(["-Q", "--batch", "--eval", &driver]).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(output.status.success(), "pinned GNU's batch: {}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8(output.stdout).unwrap();
    let mut answers: Vec<String> = text.split(MARK).map(str::to_owned).collect();
    assert_eq!(answers.pop().as_deref(), Some(""), "pinned GNU's batch ended mid-program: {text}");
    assert_eq!(answers.len(), programs.len(), "pinned GNU answered {} of {} programs", answers.len(), programs.len());
    answers
}
