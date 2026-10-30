//! buffer.c's per-buffer variables and buffer-local built-ins on the full
//! GNU Lisp chain, against pinned GNU. Every DEFVAR_PER_BUFFER exists with
//! GNU's default, local-if-set or always-local flag, permanence and
//! predicate; and the callers the daily journey found refused or void run
//! as GNU's do: eshell's `(setq-local max-lisp-eval-depth ...)`,
//! compilation and grep buffers' local `overlay-arrow-string`, save's
//! `buffer-backed-up` and ibuffer's `buffer-display-time`. Both sessions run
//! in one private HOME and directory; the test compares exit status and
//! stdout.
use std::process::Command;

const SCRIPT: &str = r#";;; -*- lexical-binding: t -*-
;; Every DEFVAR_PER_BUFFER of buffer.c: bound, default, local-if-set,
;; local in a new buffer, permanent-local, and what each value stores.
(defvar probe-per-buffer
  '(
  abbrev-mode auto-fill-function bidi-display-reordering bidi-paragraph-direction 
  bidi-paragraph-separate-re bidi-paragraph-start-re buffer-auto-save-file-format 
  buffer-auto-save-file-name buffer-backed-up buffer-display-count buffer-display-table 
  buffer-display-time buffer-file-coding-system buffer-file-format buffer-file-name 
  buffer-file-truename buffer-invisibility-spec buffer-read-only buffer-saved-size buffer-undo-list 
  cache-long-scans ctl-arrow cursor-in-non-selected-windows cursor-type default-directory 
  enable-multibyte-characters fill-column fringe-cursor-alist fringe-indicator-alist 
  fringes-outside-margins header-line-format horizontal-scroll-bar indicate-buffer-boundaries 
  indicate-empty-lines left-fringe-width left-margin left-margin-width line-spacing 
  local-abbrev-table local-minor-modes major-mode mark-active mode-line-format mode-name 
  overwrite-mode point-before-scroll right-fringe-width right-margin-width scroll-bar-height 
  scroll-bar-width scroll-down-aggressively scroll-up-aggressively selective-display 
  selective-display-ellipses tab-line-format tab-width text-conversion-style truncate-lines 
  vertical-scroll-bar word-wrap ))
(defun probe-show (v) (cond ((or (null v) (eq v t) (numberp v) (symbolp v) (stringp v)) v) ((consp v) (list 'cons (length v))) (t (type-of v))))
(dolist (v probe-per-buffer)
  (let ((rec (list v)))
    (push (boundp v) rec)
    (push (condition-case e (probe-show (default-value v)) (error (list 'error (car e)))) rec)
    (push (with-temp-buffer (list (local-variable-if-set-p v) (local-variable-p v) (and (assq v (buffer-local-variables)) t))) rec)
    (push (get v 'permanent-local) rec)
    (dolist (val (list 'probe-symbol "str" 1.5 -1 0 0.25 nil t '(a . b)))
      (push (with-temp-buffer
              (condition-case e
                  (progn (set (make-local-variable v) val)
                         (list (probe-show (symbol-value v)) (local-variable-p v)
                               (progn (kill-all-local-variables) (local-variable-p v))))
                (error (list 'error (car e) (cdr e)))))
            rec))
    (prin1 (nreverse rec)) (terpri)))
(defun probe (tag f) (prin1 (cons tag (condition-case e (funcall f) (error (list 'error e))))) (terpri))
(probe 'save (lambda ()
  (let ((file (expand-file-name "saved.txt" default-directory)) (make-backup-files t))
    (with-temp-file file (insert "one\n"))
    (with-current-buffer (find-file-noselect file)
      (goto-char (point-max)) (insert "two\n")
      (prog1 (list buffer-backed-up (local-variable-p 'buffer-backed-up)
                   (progn (setq buffer-backed-up t) (backup-buffer))
                   (let ((make-backup-files nil)) (save-buffer) (buffer-modified-p))
                   (file-exists-p (concat file "~"))
                   (with-temp-buffer (insert-file-contents file) (buffer-string)))
        (kill-buffer))))))
(probe 'local-int-restore (lambda ()
  (with-temp-buffer
    (make-local-variable 'undo-limit) (setq undo-limit 5)
    (list (condition-case e (let ((undo-limit 7)) (set-buffer-local-toplevel-value 'undo-limit 'bad) 'body) (error e))
          undo-limit (default-value 'undo-limit)))))
(probe 'eshell (lambda ()
  (require 'eshell)
  (setq eshell-directory-name (expand-file-name "eshell/" default-directory))
  (with-current-buffer (eshell)
    (list major-mode (local-variable-p 'max-lisp-eval-depth) max-lisp-eval-depth (default-value 'max-lisp-eval-depth)))))
(probe 'compile (lambda ()
  (require 'compile)
  (with-current-buffer (get-buffer-create "*compilation*")
    (compilation-mode)
    (list major-mode (local-variable-p 'overlay-arrow-string) overlay-arrow-string (default-value 'overlay-arrow-string)))))
(probe 'grep (lambda ()
  (require 'grep)
  (with-current-buffer (get-buffer-create "*grep*")
    (grep-mode)
    (list major-mode (local-variable-p 'overlay-arrow-string) overlay-arrow-string))))
(probe 'ibuffer (lambda ()
  (require 'ibuffer)
  (get-buffer-create "ibuf-probe")
  (with-current-buffer (get-buffer-create "*Ibuffer*")
    (ibuffer-mode)
    (ibuffer-update nil t)
    (list major-mode (and (string-match-p "ibuf-probe" (buffer-string)) t)
          (buffer-local-value 'buffer-display-time (get-buffer "ibuf-probe"))))))
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
fn per_buffer_variables_and_local_builtins_match_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let root = std::env::temp_dir().join(format!("romacs-buffer-locals-{}", std::process::id()));
    let run_in = |binary: &str, name: &str| {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("probe.el"), SCRIPT).unwrap();
        run(binary, &dir)
    };
    let expected = run_in(&gnu, "gnu");
    let actual = run_in(env!("CARGO_BIN_EXE_romacs-vm-gui"), "romacs");
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(expected.0, Some(0), "GNU: {}", expected.2);
    let lines = |text: &str| text.lines().map(str::to_owned).collect::<Vec<_>>();
    let (want, got) = (lines(&expected.1), lines(&actual.1));
    let differing: Vec<String> = want.iter().zip(&got).filter(|(a, b)| a != b)
        .map(|(a, b)| format!("  GNU    {a}\n  Romacs {b}")).collect();
    assert!(actual.0 == expected.0 && want.len() == got.len() && differing.is_empty(),
        "status {:?} vs {:?}, {} vs {} lines\n{}\nRomacs stderr: {}", actual.0, expected.0, got.len(), want.len(), differing.join("\n"), actual.2);
}
