//! Symbol shorthands (lread.c oblookup_considering_shorthand): the reader
//! renames a symbol whose name starts with a SHORTHAND of
//! `read-symbol-shorthands' to its LONGHAND, the first matching pair
//! winning; `#_' and names made only of `^*+-/<=>_|' are exempt. `load'
//! binds the variable for each source file to what
//! `hack-read-symbol-shorthands-function' finds in it (mule.el
//! load-with-code-conversion), so a file's shorthands end with the file.
use romacs_vm::Vm;

fn gnu(args: &[&str]) -> String {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(oracle).args(args).stdin(std::process::Stdio::null()).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn reader_applies_shorthands_as_gnu_does() {
    let expression = r#"(let ((read-symbol-shorthands '(("s-" . "string-") bad ("s-x" . "never-") ("q" . 1) ("é-" . "accent-"))))
  (list (read "(s-trim #_s-trim s- s-- -- s s\\-y é-z x-s- s-x quux)") (car (read-from-string "s-a"))))"#;
    let gnu = gnu(&["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {expression}))")]);
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let mine = vm.eval_source(&format!("(prin1-to-string {expression})")).unwrap().display().unwrap();
    assert_eq!(mine, format!("{gnu:?}"));
}

const FIXTURE: &str = ";;; shf.el --- -*- lexical-binding: t -*-
(defvar shf-x 1)
(defun shf-get () shf-x)
(setq shorthand-fixture-result
      (list (shf-get) (boundp 'shorthand-fixture-x) (boundp 'shf-x) '#_shf-raw
            (fboundp 'shorthand-fixture-get) 'shf-- read-symbol-shorthands))
;; Local Variables:
;; read-symbol-shorthands: ((\"shf-\" . \"shorthand-fixture-\"))
;; End:
";

#[test]
fn load_binds_each_files_shorthands() {
    let dir = std::env::temp_dir().join(format!("read-shorthands-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("shf.el");
    std::fs::write(&file, FIXTURE).unwrap();
    let after = "(prin1-to-string (list shorthand-fixture-result read-symbol-shorthands (boundp 'shf-x)))";
    let gnu = gnu(&["-Q", "--batch", "-l", file.to_str().unwrap(), "--eval", &format!("(princ {after})")]);
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    // GNU's hack-read-symbol-shorthands parses the Local Variables with
    // files.el; a bare VM has none, so a reader of the same section stands
    // in for it here (the loadup regime runs GNU's own).
    vm.eval_source("(setq hack-read-symbol-shorthands-function (lambda () (goto-char (point-min)) (and (search-forward \"read-symbol-shorthands: \" nil t) (read (current-buffer)))))").unwrap();
    vm.load_file(&file).unwrap();
    let mine = vm.eval_source(after).unwrap().display().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(mine, format!("{gnu:?}"));
}

/// Before mule.el defines `hack-read-symbol-shorthands-function' (early
/// loadup), a file that names the variable loads with no shorthands.
#[test]
fn load_without_the_hook_function_reads_names_as_written() {
    let dir = std::env::temp_dir().join(format!("read-shorthands-early-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("shf.el");
    std::fs::write(&file, FIXTURE).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&file).unwrap();
    let mine = vm.eval_source("(list (boundp 'shf-x) (fboundp 'shf-get) (boundp 'shorthand-fixture-x))").unwrap().display().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(mine, "(t t nil)");
}
