use romacs_vm::Vm;
use std::path::Path;

fn bootstrapped(root: &Path) -> Vm {
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
    vm
}

#[test]
fn complete_gnu_ert_dependency_closure() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped(&root);
    assert!(vm.load_report()["steps"].as_array().unwrap().iter().all(|step| step["status"] == "ok"), "strict ERT bootstrap retained a failed load form");
    assert_eq!(vm.eval_source("(list (featurep 'ert) (fboundp 'ert-run-tests-batch))").unwrap().display().unwrap(), "(t t)");
    assert_eq!(vm.eval_source("(list (featurep 'mule) (fboundp 'sgml-html-meta-auto-coding-function))").unwrap().display().unwrap(), "(t t)");
    assert_eq!(vm.eval_source("(list (function-documentation (lambda () \"Private doc\" nil)) (condition-case e (length (lambda () \"Opaque\" nil)) (wrong-type-argument (car (cdr e)))))").unwrap().display().unwrap(), "(\"Private doc\" sequencep)");
    assert_eq!(vm.eval_source("(let ((function (lambda () \"Private source\" nil))) (mapcar (lambda (circle) (let ((print-circle circle)) (list (equal (prin1-to-string function) (with-output-to-string (cl-prin1 function))) (condition-case nil (progn (length function) nil) (wrong-type-argument t))))) '(nil t)))").unwrap().display().unwrap(), "((t t) (t t))", "OPAQUE-GNU-CL-PRINT");
    vm.eval_source("(require 'hfy-cmap)").unwrap();
    assert_eq!(vm.eval_source("(list emacs-major-version emacs-minor-version (with-temp-buffer (insert \"255 250 250\\t\\tsnow\\n248 248 255\\t\\tghost white\\n248 248 255\\t\\tGhostWhite\\n\") (hfy-cmap--parse-buffer (current-buffer))))").unwrap().display().unwrap(), "(32 0 ((\"GhostWhite\" 248 248 255) (\"ghost white\" 248 248 255) (\"snow\" 255 250 250)))");
}

// GNU's Unicode tables, with their byte-code functions in source
// (DIVERGENCES.md VM2-UNIDATA1), give pinned GNU's property values.
#[test]
fn unicode_character_properties_match_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped(&root);
    vm.eval_source("(load \"international/charprop.el\" nil t t)").unwrap();
    let source = "(prin1-to-string (list (mapcar (lambda (c) (mapcar (lambda (p) (get-char-code-property c p)) '(name general-category decomposition bidi-class old-name canonical-combining-class mirroring uppercase))) '(?a #xE9 #xAC01 #xD7A3 #x4E00 #x1F600 #x5D0 #x558 #xFB01 #x2160 #xE0100 #x300)) (mapcar (lambda (p) (char-code-property-description p (get-char-code-property #x5D0 p))) '(general-category bidi-class canonical-combining-class))))";
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(binary).args(["-Q", "--batch", "--eval", &format!("(princ {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let printed = vm.eval_source(source).unwrap().display().unwrap();
    let actual: String = serde_json::from_str(&printed).unwrap_or(printed);
    assert_eq!(actual, expected);
}

// coding.c's charset coding type, through mule-conf.el's definitions.
#[test]
fn charset_coding_systems_match_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped(&root);
    vm.eval_source("(progn (load \"language/european.el\" nil t t) (load \"language/chinese.el\" nil t t))").unwrap();
    let source = "(prin1-to-string (list (decode-coding-string \"\\341\" 'iso-8859-15) (decode-coding-string \"a\\244\\341\\r\\nb\" 'iso-8859-15-dos) (encode-coding-string \"a\\u00e1\\u20ac\\u0100\" 'iso-8859-15) (encode-coding-string \"\\u00e1\" 'iso-8859-2) (decode-coding-string \"\\x81 a\\xe9\" 'windows-1252) last-coding-system-used (encode-coding-string (string #x3fff81 ?a) 'latin-1) (decode-coding-string \"\\244\\242\" 'chinese-gbk) (encode-coding-string \"\\u4e00\" 'chinese-gbk) (decode-coding-string \"\\201\" 'chinese-gbk) (decode-coding-string \"a\" 'iso-8859-15) (decode-coding-string \"\\xe9\\xe9\" 'cp437)))";
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(binary).args(["-Q", "--batch", "--eval", &format!("(princ {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let printed = vm.eval_source(source).unwrap().display().unwrap();
    let actual: String = serde_json::from_str(&printed).unwrap_or(printed);
    assert_eq!(actual, expected);
}
