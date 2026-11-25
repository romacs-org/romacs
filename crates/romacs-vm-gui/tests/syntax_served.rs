//! electric-pair's balance check on an opener typed into a large Elisp
//! buffer, in the real `emacs-lisp-mode' with `elisp-mode-syntax-propertize'
//! (MEASURE.md F1, keys-el-default): the edit leaves the propertize
//! frontier at the edit, and electric-pair scans forward from each
//! enclosing opener in turn. The text the key leaves is pinned GNU's; and
//! once the first scans have propertized what they read, the later ones
//! fold over it, which `romacs-syntax-summaries-stats' counts.
#[path = "support/installed.rs"]
mod installed;
use std::process::Command;

const PROBE: &str = r#";;; -*- lexical-binding: t -*-
(require 'cl-lib)
(defun ss-stat (key &optional sub)
  (if (fboundp 'romacs-syntax-summaries-stats)
      (let ((v (plist-get (romacs-syntax-summaries-stats) key))) (or (if sub (cdr (assq sub v)) v) 0))
    0))
(defun ss-counts () (list (ss-stat :folds 'excursion) (ss-stat :folds 'exact) (ss-stat :skipped)))
(with-temp-buffer
  (dotimes (i 3500)
    (insert (format "(defun f%d (a b)\n  \"Doc (string) %d.\"\n  (let ((x ?\\()) (list a b x 'c)))\n\n" i i)))
  (emacs-lisp-mode)
  (electric-pair-mode 1)
  (syntax-ppss (point-max))
  (goto-char (point-min))
  (search-forward "(list a b x 'c" nil nil 3)
  (let ((before (ss-counts)) (done-before syntax-propertize--done))
    (let ((last-command-event ?\()) (self-insert-command 1))
    (prin1 (list 'value (point) (buffer-size) (< done-before (point-max))
                 (buffer-substring (- (point) 40) (+ (point) 40))))
    (terpri)
    (prin1 (cons 'folded (cl-mapcar #'- (ss-counts) before)))
    (terpri)))
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
fn electric_pair_scans_fold_what_the_first_ones_propertized() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let root = std::env::temp_dir().join(format!("romacs-syntax-served-{}", std::process::id()));
    let run_in = |binary: &str, name: &str| {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("probe.el"), PROBE).unwrap();
        run(binary, &dir)
    };
    let expected = run_in(&gnu, "gnu");
    let actual = run_in(installed::romacs(), "romacs");
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(expected.0, Some(0), "GNU: {}", expected.2);
    assert_eq!(actual.0, Some(0), "Romacs: {}", actual.2);
    let value = |out: &str| out.lines().find(|line| line.starts_with("(value")).map(str::to_owned);
    assert_eq!(value(&actual.1), value(&expected.1), "the key's result");
    let folded = actual.1.lines().find(|line| line.starts_with("(folded")).expect("a count line");
    let counts: Vec<i64> = folded.trim_matches(['(', ')']).split(' ').skip(1).map(|n| n.parse().unwrap()).collect();
    assert!(counts[0] >= 10 && counts[2] >= 100_000,
            "one key's scans folded (excursion exact skipped) {counts:?}: the later scans fold what the first propertized");
}
