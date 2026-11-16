//! The function a command error names (`command-error-function's SIGNAL,
//! the "scroll-up-command: " of "scroll-up-command: End of buffer")
//! against pinned GNU, in batch command loops: a Lisp half of a C
//! primitive (lisp/vm/window-scroll.el) is C in GNU, so the error names
//! the primitive's caller, not the half.
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
fn a_command_error_names_the_function_gnu_names() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = installed::romacs();
    let dir = std::env::temp_dir().join(format!("romacs-signaling-function-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let form = r#"(let (errors)
  (global-set-key [f9] 'exit-recursive-edit)
  (global-set-key [f5] 'sf-own)
  (fset 'sf-own (lambda () (interactive) (error "Own %s" 1)))
  (global-set-key [f6] (lambda () (interactive) (scroll-down)))
  (switch-to-buffer (get-buffer-create "sf"))
  (insert "a\nb\n")
  (goto-char (point-min))
  (let ((command-error-function
         (lambda (data _context signal) (push (list (if (symbolp signal) signal 'closure) data) errors))))
    (setq unread-command-events
          (mapcar (lambda (e) (cons t e)) (list ?\C-v ?\M-v 'f6 'f5 ?\M-> ?\C-f 'f9)))
    (recursive-edit))
  (nreverse errors))"#;
    let expected = batch(&gnu, &dir, form);
    assert!(expected.starts_with("((scroll-up-command (end-of-buffer)) (scroll-down-command"), "{expected}");
    assert_eq!(batch(romacs, &dir, form), expected);
    std::fs::remove_dir_all(&dir).unwrap();
}
