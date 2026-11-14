//! Indirect buffers' real callers on the full GNU Lisp chain, against
//! pinned GNU: shell-mode fontifies its input in comint's indirect buffer
//! when redisplay calls `jit-lock-function` (the error a Romacs shell window
//! showed was `(void-function make-indirect-buffer)`), and simple.el's
//! `clone-indirect-buffer` clones a narrowed Lisp buffer. Both sessions run
//! in one private HOME; the test compares their exit status and stdout.
#[path = "support/installed.rs"]
mod installed;
use std::process::Command;

const SCRIPT: &str = r#";;; -*- lexical-binding: t -*-
(require 'shell)
(defun probe-faces ()
  (let (runs (pos (point-min)))
    (while (< pos (point-max))
      (let ((next (next-single-property-change pos 'face nil (point-max))))
        (push (list pos next (get-text-property pos 'face)) runs)
        (setq pos next)))
    (nreverse runs)))
(let ((b (get-buffer-create "*shell-probe*")))
  (with-current-buffer b
    (shell-mode)
    (let ((inhibit-read-only t))
      (insert (propertize "$ " 'field 'output 'font-lock-face 'comint-highlight-prompt 'front-sticky t 'rear-nonsticky t))
      (insert "for f in *.el; do echo \"$f\" | grep -v 'x' # note\n")
      (insert (propertize "out\n" 'field 'output))
      (insert (propertize "$ " 'field 'output 'font-lock-face 'comint-highlight-prompt))
      (insert "if [ -n \"$HOME\" ]; then cd ~; fi"))
    ;; Redisplay's call: jit-lock fontifies from a position.
    (jit-lock-register #'font-lock-fontify-region)
    (put-text-property (point-min) (point-max) 'fontified nil)
    (jit-lock-function 3)
    (prin1 (list (buffer-name (buffer-base-buffer comint--indirect-buffer)) (probe-faces)))
    (terpri)
    (kill-buffer b)
    (prin1 (list (buffer-live-p b) (buffer-live-p comint--indirect-buffer)))
    (terpri)))
(let ((b (get-buffer-create "probe.el")))
  (with-current-buffer b
    (emacs-lisp-mode)
    (insert "(defun f (x)\n  \"Doc.\"\n  (+ x 1))\n")
    (narrow-to-region 15 22)
    (goto-char 17)
    (let ((c (clone-indirect-buffer nil nil)))
      (prin1 (list (buffer-name c) (buffer-name (buffer-base-buffer c))
                   (with-current-buffer c (list major-mode (point) (point-min) (point-max) (buffer-string)))
                   (progn (with-current-buffer c (widen) (goto-char (point-max)) (insert ";; end\n"))
                          (with-current-buffer b (widen) (buffer-string)))))
      (terpri))))
"#;

fn run(binary: &str, dir: &std::path::Path) -> (Option<i32>, String, String) {
    let output = Command::new(binary).args(["-Q", "--batch", "-l", "probe.el"]).current_dir(dir)
        .env("HOME", dir).env("XDG_CONFIG_HOME", dir.join("config")).env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_DATA_HOME", dir.join("data")).env("XDG_STATE_HOME", dir.join("state"))
        .output().unwrap();
    let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).into_owned();
    (output.status.code(), text(output.stdout), text(output.stderr))
}

#[test]
fn shell_input_fontification_and_clones_match_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let dir = std::env::temp_dir().join(format!("romacs-indirect-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("probe.el"), SCRIPT).unwrap();
    let expected = run(&gnu, &dir);
    let actual = run(installed::romacs(), &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(expected.0, Some(0), "GNU: {}", expected.2);
    assert_eq!((actual.0, &actual.1), (expected.0, &expected.1), "Romacs stderr: {}", actual.2);
}
