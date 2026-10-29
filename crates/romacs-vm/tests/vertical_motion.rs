//! `vertical-motion' (indent.c Fvertical_motion) against pinned GNU in
//! batch, on its initial 80-column frame: continued lines, short, empty
//! and non-ASCII lines, truncation, counts that run past either end, and
//! the argument checks, and `line-pixel-height'. Batch moves by lines only,
//! as GNU's vmotion does.
use romacs_vm::Vm;

/// One expression per line, each in a fresh buffer.
const CASES: &str = r###"
(progn (insert "short\n" (make-string 25 ?x) "\nab\n\nlast") (goto-char 3) (list (vertical-motion 1) (point) (vertical-motion 1) (point) (vertical-motion '(2 . 1)) (point) (vertical-motion -2) (point) (vertical-motion '(1.6 . 3)) (point) (vertical-motion 10) (point) (vertical-motion -20) (point)))
(progn (insert "a\n" (make-string 200 ?y) "\nλόγος é ü\n" (make-string 79 ?z) "\n" (make-string 80 ?w) "\nend") (goto-char 1) (let (out) (dotimes (_ 9) (push (list (vertical-motion 1) (point)) out)) (dotimes (_ 9) (push (list (vertical-motion -1) (point)) out)) (nreverse out)))
(progn (insert (make-string 200 ?y) "\nb") (goto-char 120) (list (vertical-motion 0) (point) (progn (goto-char 120) (vertical-motion 1)) (point) (progn (goto-char 120) (vertical-motion -1)) (point) (progn (goto-char 170) (vertical-motion -2)) (point)))
(progn (insert (make-string 200 ?y) "\nb\n" (make-string 100 ?q)) (setq truncate-lines t) (goto-char 150) (list (vertical-motion 1) (point) (vertical-motion 1) (point) (vertical-motion -2) (point)))
(progn (insert "x") (list (condition-case e (vertical-motion 'a) (error e)) (condition-case e (vertical-motion '(a . 1)) (error e)) (condition-case e (vertical-motion 1 'w) (error e)) (condition-case e (vertical-motion 1 nil 'c) (error e)) (vertical-motion 0) (vertical-motion 1) (vertical-motion -1)))
(progn (insert "ab\n" (make-string 200 ?y)) (list (line-pixel-height) (progn (goto-char 150) (line-pixel-height)) (progn (goto-char (point-max)) (line-pixel-height))))
(progn (insert "\n\n\n") (goto-char (point-max)) (list (vertical-motion -1) (point) (vertical-motion -5) (point) (vertical-motion 5) (point)))
"###;

#[test]
fn vertical_motion_matches_pinned_gnu_in_batch() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    vm.set_frame_geometry(80, 25, 1, 1).unwrap();
    vm.install_lisp_prelude().unwrap();
    let mut failures = Vec::new();
    for case in CASES.lines().filter(|line| !line.is_empty()) {
        let source = format!("(progn (set-buffer (get-buffer-create \"vm\")) (kill-all-local-variables) (erase-buffer) (set-window-buffer nil (current-buffer)) {case})");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            failures.push(format!("{case}\n  GNU    {expected}\n  Romacs {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
