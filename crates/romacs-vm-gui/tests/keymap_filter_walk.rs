//! A menu item's `:filter' that changes the keymaps being walked, as
//! `set-transient-map' does when it pushes a map onto the composed
//! `overriding-terminal-local-map', against pinned GNU in batch: the key is
//! bound to the filter's result, as GNU's one-pass lookup finds it, in
//! `key-binding', `lookup-key' over the active maps and a keyboard macro.
#[path = "support/installed.rs"]
mod installed;
use std::process::Command;

fn batch(binary: &str, dir: &std::path::Path, form: &str) -> String {
    let mut command = Command::new(binary);
    command.args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {form}))")]).current_dir(dir).env("HOME", dir).env("TMPDIR", dir);
    let output = command.output().unwrap();
    assert!(output.status.success(), "{form}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_filter_that_pushes_a_map_keeps_its_result() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = installed::romacs();
    let dir = std::env::temp_dir().join(format!("romacs-keymap-filter-walk-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let form = r#"(progn
  (defvar fw-log nil)
  (defun fw-case (how)
              (let* ((fail (make-symbol "fw-fail")) (calls 0)
                     (other (make-sparse-keymap)) (map (make-sparse-keymap)))
                (fset fail (lambda () (interactive) (push 'fail-ran fw-log)))
                (define-key other "a" (lambda () (interactive) (push 'other-ran fw-log)))
                (define-key map "a" (list 'menu-item "" 'ignore
                                          :filter (lambda (_b) (setq calls (1+ calls))
                                                    (set-transient-map other t) fail)))
                (set-transient-map map t)
                (setq fw-log nil)
                (prog1 (list how
                             (pcase how
                               ('key-binding (let ((b (key-binding "a" t))) (and (symbolp b) (symbol-name b))))
                               ('lookup-key (let ((b (lookup-key (current-active-maps t) "a"))) (and (symbolp b) (symbol-name b))))
                               ('macro (execute-kbd-macro "a") (reverse fw-log)))
                             calls)
                  (setq overriding-terminal-local-map nil))))
  (list (fw-case 'key-binding) (fw-case 'lookup-key) (fw-case 'macro)))"#;
    let expected = batch(&gnu, &dir, form);
    assert_eq!(expected, r#"((key-binding "fw-fail" 1) (lookup-key "fw-fail" 1) (macro (fail-ran) 2))"#);
    assert_eq!(batch(romacs, &dir, form), expected);
    std::fs::remove_dir_all(&dir).unwrap();
}
