//! GNU's byte-op primitives are sealed (DIVERGENCES.md VM2-SEALED1): a call
//! of `car' is `Bcar', which never reads the function cell, so advice on
//! `car' is seen only by cell readers (`funcall' of a variable, `mapcar').
//! Pinned GNU 32.0.50, byte-compiled, answers the same case the same way.
use romacs_vm::Vm;
use std::path::Path;

fn bootstrapped(root: &Path) -> Vm {
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    vm.set_frame_geometry(80, 25, 1, 1).unwrap();
    let lisp = root.join("lisp/gnu");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('=')
            && !name.ends_with('~') && !name.ends_with(".orig") && !name.ends_with(".rej"))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    if names.iter().any(|n| n == "obsolete") { paths.push(lisp.join("obsolete")); }
    paths.push(root.join("lisp/romacs"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("lisp/romacs/romacs-loadup-bootstrap.el")).unwrap();
    vm
}

/// Cell reads go through `sp-call`: byte-opt folds `(funcall 'car x)`, and
/// `(funcall f x)` with F bound to a constant, into the byte code itself.
const CASE: &str = "(let ((adv (lambda (f x) (cons 'advised (funcall f x)))) (l '((1) (2))))
  (defvar sp-after nil)
  (defun sp-probe (x) (car x))
  (defun sp-plus (a b) (+ a b))
  (defun sp-call (fn &rest args) (apply fn args))
  (advice-add 'car :around adv)
  (prog1 (list (sp-probe '(1 2)) (car '(1 2)) (sp-call 'car '(1 2)) (mapcar #'car l)
               (and (advice-member-p adv 'car) t)
               (let ((orig (symbol-function '+)))
                 (fset '+ (lambda (a b) 'plus-redefined))
                 (unwind-protect (list (sp-plus 8 3) (sp-call '+ 8 3)) (fset '+ orig))))
    (advice-remove 'car adv)
    (setq sp-after (list (sp-call 'car '(1 2)) (and (advice-member-p adv 'car) t)))))";

#[test]
fn advice_on_a_sealed_primitive_matches_compiled_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&binary)
        .args(["-Q", "--batch", "--eval", "(setq lexical-binding t byte-compile-warnings nil native-comp-enable-subr-trampolines nil)",
            "--eval", &format!("(princ (prin1-to-string (funcall (byte-compile (lambda () {CASE})))))")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    assert_eq!(expected, "(1 1 (advised . 1) ((advised . 1) (advised . 2)) t (11 plus-redefined))");
    let root2 = root.clone();
    let actual = std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let mut vm = bootstrapped(&root2);
        let value = vm.eval_source(&format!("(prin1-to-string {CASE})")).and_then(|v| v.display())
            .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s));
        let after = vm.eval_source("(prin1-to-string sp-after)").and_then(|v| v.display())
            .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s));
        (value, after)
    }).unwrap().join().unwrap();
    assert_eq!(actual.0.as_deref(), Ok(expected.as_str()));
    // advice-remove restores the primitive for cell readers too.
    assert_eq!(actual.1.as_deref(), Ok("(1 nil)"));
}
