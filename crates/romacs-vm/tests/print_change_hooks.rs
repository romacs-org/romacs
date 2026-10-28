//! Printing into a buffer or at a marker runs the insertion's change hooks
//! as GNU print_finish does: insert_1_both prepares the text (before-change
//! hooks), signal_after_change follows even for empty output, and point is
//! at the insertion while they run. Compared against pinned GNU.
use romacs_vm::Vm;
use std::path::Path;
#[path = "support/gnu_ert_vm.rs"]
mod gnu_ert_vm;

const SETUP: &str = r##"(set-buffer (get-buffer-create "*print-hooks*")) (erase-buffer) (insert "abc") (goto-char 2) (setq log nil) (make-local-variable 'after-change-functions) (make-local-variable 'before-change-functions) (setq after-change-functions (list (lambda (b e l) (setq log (cons (list 'a b e l (point) (buffer-substring b e)) log))))) (setq before-change-functions (list (lambda (b e) (setq log (cons (list 'b b e (point)) log)))))"##;

const CASES: &str = r##"
(list (prin1 'xy (current-buffer)) (buffer-string) (point) (reverse log))
(list (princ "" (current-buffer)) (buffer-string) (point) (reverse log))
(let ((m (copy-marker 3))) (list (prin1 '(1 "q") m) (buffer-string) (point) (marker-position m) (reverse log)))
(let ((m (copy-marker 1))) (list (princ "ZZ" m) (buffer-string) (point) (marker-position m) (reverse log)))
(list (print 7 (current-buffer)) (terpri (current-buffer)) (write-char ?λ (current-buffer)) (buffer-string) (reverse log))
(list (princ (propertize "pp" 'face 'bold) (current-buffer)) (text-properties-at 2) (reverse log))
(let ((inhibit-modification-hooks t)) (list (princ "n" (current-buffer)) (buffer-string) (reverse log)))
(progn (setq after-change-functions nil) (list (princ "b" (current-buffer)) (buffer-string) (reverse log)))
(let ((other (current-buffer))) (set-buffer (get-buffer-create "*other*")) (list (princ "o" other) (buffer-name) (with-current-buffer other (buffer-string)) (reverse log)))
(progn (setq after-change-functions (list (lambda (&rest _) (error "boom")))) (list (condition-case e (princ "e" (current-buffer)) (error e)) (buffer-string) after-change-functions))
(progn (setq before-change-functions (list (lambda (&rest _) (error "hook-must-run")))) (list (condition-case e (princ "x" (current-buffer)) (error e)) (buffer-string) (point)))
(let ((m (copy-marker 3))) (setq buffer-read-only t) (list (condition-case e (princ "x" m) (error (car e))) (point) (buffer-string)))
"##;

fn oracle(expression: &str) -> String {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let source = format!("(progn {SETUP} (prin1 (let ((print-escape-newlines t)) (prin1-to-string {expression}))))");
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &source])
        .env("LANG", "C.UTF-8").output().unwrap();
    assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn romacs(vm: &mut Vm, expression: &str) -> String {
    let source = format!("(progn (defvar log nil) {SETUP} (let ((print-escape-newlines t)) (prin1-to-string {expression})))");
    match vm.eval_source(&source).and_then(|v| v.display()) {
        Ok(text) => text,
        Err(error) => format!("VM error: {error}"),
    }
}

#[test]
fn printing_runs_change_hooks_as_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let failures: Vec<String> = CASES.lines().filter(|line| !line.is_empty())
        .filter_map(|expression| {
            let (gnu, ours) = (oracle(expression), romacs(&mut vm, expression));
            (gnu != ours).then(|| format!("{expression}\n  gnu:    {gnu}\n  romacs: {ours}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
