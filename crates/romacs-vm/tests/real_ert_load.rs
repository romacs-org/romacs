use romacs_vm::Vm;
use std::path::Path;

#[test]
fn complete_gnu_ert_dependency_closure() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    let paths = ["gnu-lisp", "gnu-lisp/emacs-lisp", "gnu-lisp/progmodes",
        "gnu-lisp/international", "gnu-lisp/textmodes", "rlisp"].map(|name| root.join(name));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    for name in ["rlisp/romacs-gnu-ert-bootstrap.el", "gnu-lisp/help-fns.el", "rlisp/romacs-cl-print.el", "gnu-lisp/emacs-lisp/ert.el"] {
        if let Err(error) = vm.load_file(&root.join(name)).map(|_| ()) {
            let report = vm.load_report();
            let failed = report["steps"].as_array().unwrap().iter()
                .filter(|step| step["status"] != "ok").collect::<Vec<_>>();
            panic!("{name}: {error}\n{failed:?}");
        }
    }
    assert!(vm.load_report()["steps"].as_array().unwrap().iter().all(|step| step["status"] == "ok"), "strict ERT bootstrap retained a failed load form");
    assert_eq!(vm.eval_source("(list (featurep 'ert) (fboundp 'ert-run-tests-batch))").unwrap().display().unwrap(), "(t t)");
    assert_eq!(vm.eval_source("(list (featurep 'mule) (fboundp 'sgml-html-meta-auto-coding-function))").unwrap().display().unwrap(), "(t t)");
    assert_eq!(vm.eval_source("(list (function-documentation (lambda () \"Private doc\" nil)) (condition-case e (length (lambda () \"Opaque\" nil)) (wrong-type-argument (car (cdr e)))))").unwrap().display().unwrap(), "(\"Private doc\" sequencep)");
    assert_eq!(vm.eval_source("(let ((function (lambda () \"Private source\" nil))) (mapcar (lambda (circle) (let ((print-circle circle)) (list (equal (prin1-to-string function) (with-output-to-string (cl-prin1 function))) (condition-case nil (progn (length function) nil) (wrong-type-argument t))))) '(nil t)))").unwrap().display().unwrap(), "((t t) (t t))", "OPAQUE-GNU-CL-PRINT");
    vm.eval_source("(require 'hfy-cmap)").unwrap();
    assert_eq!(vm.eval_source("(list emacs-major-version emacs-minor-version (with-temp-buffer (insert \"255 250 250\\t\\tsnow\\n248 248 255\\t\\tghost white\\n248 248 255\\t\\tGhostWhite\\n\") (hfy-cmap--parse-buffer (current-buffer))))").unwrap().display().unwrap(), "(32 0 ((\"GhostWhite\" 248 248 255) (\"ghost white\" 248 248 255) (\"snow\" 255 250 250)))");
}
