//! The syntax scanner over real files in their real modes, against pinned
//! GNU: chained and whole `parse-partial-sexp' states, comment and string
//! stops, and `scan-lists', `scan-sexps' and `forward-comment' both ways
//! from positions STRIDE apart (ROMACS_SYNTAX_CORPUS_STRIDE, default 89; 1
//! is every position). Both sessions read the same files from this tree,
//! each in a private HOME, and the test compares their output line by line.
#[path = "support/installed.rs"]
mod installed;
use std::process::Command;

const PROBE: &str = r#";;; -*- lexical-binding: t -*-
(defun sc-out (&rest x) (prin1 x) (terpri))
(defun sc-try (f) (condition-case e (funcall f) (error (list 'error e))))
(defun sc-probe (file mode stride)
  (with-temp-buffer
    (insert-file-contents file)
    (funcall mode)
    (let ((name (file-name-nondirectory file)) (end (point-max)))
      (let ((state nil) (p (point-min)))
        (while (< p end)
          (let ((next (min end (+ p stride))))
            (setq state (parse-partial-sexp p next nil nil state))
            (sc-out name 'chain next (point) state)
            (setq p next))))
      (let ((p (point-min)))
        (while (< p end)
          (sc-out name 'whole p (parse-partial-sexp (point-min) p))
          (setq p (+ p (* 61 stride)))))
      (dolist (stop '(t syntax-table))
        (goto-char (point-min))
        (let ((s nil) (n 0))
          (while (and (< (point) end) (< n 3000))
            (setq s (parse-partial-sexp (point) end nil nil s stop))
            (sc-out name 'stop stop (point) s)
            (setq n (1+ n)))))
      (let ((p (point-min)))
        (while (<= p end)
          (sc-out name 'motion p
                  (sc-try (lambda () (scan-lists p 1 0))) (sc-try (lambda () (scan-lists p -1 0)))
                  (sc-try (lambda () (scan-lists p 1 1))) (sc-try (lambda () (scan-lists p -1 1)))
                  (sc-try (lambda () (scan-sexps p 1))) (sc-try (lambda () (scan-sexps p -1)))
                  (save-excursion (goto-char p) (list (forward-comment 1) (point)))
                  (save-excursion (goto-char p) (list (forward-comment -1) (point))))
          (setq p (+ p stride)))))))
(let ((stride (string-to-number (getenv "SC_STRIDE"))))
  (dolist (spec (read (getenv "SC_FILES")))
    (sc-probe (car spec) (cadr spec) stride)))
"#;

/// Elisp with strings, character literals and `syntax-propertize' at work;
/// C with both comment styles and the preprocessor.
const FILES: [(&str, &str); 3] = [
    ("lisp/subr.el", "emacs-lisp-mode"),
    ("lisp/emacs-lisp/rx.el", "emacs-lisp-mode"),
    ("test/gnu/test/manual/etags/c-src/etags.c", "c-mode"),
];

fn run(binary: &str, dir: &std::path::Path, files: &str, stride: &str) -> (Option<i32>, String, String) {
    let output = Command::new(binary).args(["-Q", "--batch", "-l", "probe.el"]).current_dir(dir)
        .env("HOME", dir).env("XDG_CONFIG_HOME", dir.join("config")).env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_DATA_HOME", dir.join("data")).env("XDG_STATE_HOME", dir.join("state"))
        .env("SC_FILES", files).env("SC_STRIDE", stride)
        .output().unwrap();
    let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).into_owned();
    (output.status.code(), text(output.stdout), text(output.stderr))
}

#[test]
fn syntax_scans_of_corpus_files_match_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let stride = std::env::var("ROMACS_SYNTAX_CORPUS_STRIDE").unwrap_or_else(|_| "89".into());
    let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let files = FILES.iter().map(|(file, mode)| format!("({:?} {mode})", tree.join(file).to_str().unwrap()))
        .collect::<Vec<_>>().join(" ");
    let files = format!("({files})");
    let root = std::env::temp_dir().join(format!("romacs-syntax-corpus-{}", std::process::id()));
    let run_in = |binary: &str, name: &str| {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("probe.el"), PROBE).unwrap();
        run(binary, &dir, &files, &stride)
    };
    let expected = run_in(&gnu, "gnu");
    let actual = run_in(installed::romacs(), "romacs");
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(expected.0, Some(0), "GNU: {}", expected.2);
    assert_eq!(actual.0, Some(0), "Romacs: {}", actual.2);
    let (gnu_lines, romacs_lines): (Vec<_>, Vec<_>) = (expected.1.lines().collect(), actual.1.lines().collect());
    let differ: Vec<String> = gnu_lines.iter().zip(&romacs_lines).filter(|(g, r)| g != r).take(10)
        .map(|(g, r)| format!("  GNU    {g}\n  Romacs {r}")).collect();
    assert!(differ.is_empty() && gnu_lines.len() == romacs_lines.len(),
            "{} GNU lines, {} Romacs lines; first differences:\n{}", gnu_lines.len(), romacs_lines.len(), differ.join("\n"));
}
