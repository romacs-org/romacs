//! What a load records about its definitions (LOADHIST_ATTACH), against
//! pinned GNU, so `symbol-file', `feature-file' and `unload-feature' read the
//! same `load-history'.
use romacs_vm::Vm;

const FILE: &str = "\
;;; -*- lexical-binding: t -*-
(defvar lhe-a 1 \"doc\")
(defvar lhe-b 2)
(defvar lhe-c)
(defconst lhe-d 3 \"doc\")
(defconst lhe-e (progn (defvar lhe-f 1) 2))
(defvaralias 'lhe-g 'lhe-a)
(defalias 'lhe-h #'car)
(autoload 'lhe-i \"nowhere\")
(defalias 'lhe-i #'car)
(autoload 'lhe-j \"nowhere\")
(provide 'lhe-k)
(require 'lhe-k)
(require 'lhe-k)
(provide 'lhe-k)
(setq lhe-risky (get 'lhe-d 'risky-local-variable)
      lhe-doc (get 'lhe-a 'variable-documentation)
      lhe-values (list lhe-a lhe-b (boundp 'lhe-c) lhe-d lhe-e lhe-f lhe-g))
";

/// Outside a load: an uninvolved `require' records nothing, the rest attach.
const OUTSIDE: &str = "(let ((current-load-list nil))
  (provide 'lhe-out)
  (require 'lhe-out)
  (let ((after-provide (copy-sequence current-load-list)))
    (setq current-load-list (list \"file\"))
    (require 'lhe-out)
    (list after-provide current-load-list)))";

#[test]
fn a_load_records_what_gnu_records() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let dir = std::env::temp_dir().join(format!("romacs-load-history-entries-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = std::fs::canonicalize(&dir).unwrap().join("rt-lhe.el");
    std::fs::write(&path, FILE).unwrap();
    let file = path.to_str().unwrap();
    let report = format!(
        "(prin1-to-string (list (cdr (assoc {file:?} load-history)) lhe-risky lhe-doc lhe-values {OUTSIDE}))");
    let output = std::process::Command::new(&binary)
        .args(["-Q", "--batch", "--eval", &format!("(progn (load {file:?} nil t) (princ {report}))")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&path).unwrap();
    let actual = vm.eval_source(&report).and_then(|v| v.display()).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(actual.trim_matches('"').replace("\\\"", "\""), expected);
}

/// Romacs loads source where GNU loads `.elc`: a `require` an expansion runs
/// is not the file's (loadhist-tests-file-requires), and one `eval-and-compile`
/// keeps is.
#[test]
fn a_compile_time_require_is_not_a_dependency() {
    let dir = std::env::temp_dir().join(format!("romacs-load-history-compile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = std::fs::canonicalize(&dir).unwrap().join("rt-lhe-compile.el");
    std::fs::write(&path, ";;; -*- lexical-binding: t -*-
(defmacro lhe-now (&rest body) (eval (cons 'progn body) t) nil)
(lhe-now (require 'lhe-dep-a))
(require 'lhe-dep-b)
").unwrap();
    let file = path.to_str().unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source("(progn (provide 'lhe-dep-a) (provide 'lhe-dep-b))").unwrap();
    vm.load_file(&path).unwrap();
    let requires = vm.eval_source(&format!(
        "(let ((out nil)) (dolist (x (cdr (assoc {file:?} load-history)) (nreverse out)) (if (eq (car-safe x) 'require) (setq out (cons (cdr x) out)))))"))
        .and_then(|v| v.display()).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(requires, "(lhe-dep-b)");
}
