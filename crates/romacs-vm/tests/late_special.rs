//! A `let' inside a function made after its enclosing form was compiled
//! binds dynamically once the variable is special, as GNU's interpreter
//! decides at each `let': `with-eval-after-load' and `use-package :config'
//! wrap such a `defun' around a package's variable before the package's
//! `defvar' (VM2-LATE-SPECIAL1). Each form is a separate evaluation, as
//! `--eval' arguments are in GNU. A top-level function compiled before the
//! `defvar' still binds lexically, the limit VM2-LATE-SPECIAL1 records.
use romacs_vm::Vm;

const FORMS: &[&str] = &[
    "(defalias 'romacs-late-g (lambda () (list (if (boundp 'romacs-late-v) romacs-late-v 'unbound) (special-variable-p 'romacs-late-v))))",
    "(defalias 'romacs-late-outer (lambda () (defalias 'romacs-late-f (lambda () (let ((romacs-late-v 2)) (romacs-late-g))))))",
    "(defvar romacs-late-v 1)",
    "(romacs-late-outer)",
];
const PROBE: &str = "(list (romacs-late-f) (funcall (lambda (x) (let ((romacs-late-v x)) (romacs-late-g))) 4))";

#[test]
fn late_specials_bind_dynamically_as_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut args = vec!["-Q".to_string(), "--batch".into()];
    for form in FORMS {
        args.extend(["--eval".into(), form.to_string()]);
    }
    args.extend(["--eval".into(), format!("(prin1 {PROBE})")]);
    let output = std::process::Command::new(&binary).args(&args).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    for form in FORMS {
        vm.eval_source(form).unwrap();
    }
    assert_eq!(vm.eval_source(PROBE).unwrap().display().unwrap(), expected);
}
