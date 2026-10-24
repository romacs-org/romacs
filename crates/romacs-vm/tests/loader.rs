use romacs_vm::{Fixed, Vm};
fn scratch(name: &str) -> std::path::PathBuf {
    let p = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(format!(
        ".hermes/cache/scratch/romacs/vm2/load-{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}
#[test]
fn nested_require_provide_and_error_recovery_are_real_file_effects() {
    let dir = scratch("require");
    std::fs::write(
        dir.join("dep.el"),
        "(setq dep-count (1+ dep-count)) (defun loaded (x) (+ x 2)) (provide 'dep)",
    )
    .unwrap();
    std::fs::write(
        dir.join("top.el"),
        "(require 'dep) (setq answer (loaded 40)) (provide 'top)",
    )
    .unwrap();
    let mut vm = Vm::new();
    vm.set_load_path(std::slice::from_ref(&dir)).unwrap();
    vm.set_collection_floor(1);
    vm.eval_source("(setq dep-count 0)").unwrap();
    let value = vm
        .eval_source("(let ((outer 9)) (require 'top) (+ outer answer))")
        .unwrap();
    assert_eq!(value.as_integer().unwrap(), 51);
    assert!(vm.eval_source("(eq (require 'top) 'top)").unwrap().is_t());
    assert_eq!(
        vm.eval_source("dep-count").unwrap().as_integer().unwrap(),
        1
    );
    assert_eq!(vm.eval_source("(featurep 'dep)").unwrap(), Fixed::T);
    assert_eq!(
        vm.eval_source("(require 'absent nil t)").unwrap(),
        Fixed::NIL
    );
    std::fs::write(dir.join("no-provide.el"), "(setq bad-load-effect 17)").unwrap();
    assert!(vm.eval_source("(require 'no-provide nil t)").is_err());
    assert_eq!(
        vm.eval_source("bad-load-effect")
            .unwrap()
            .as_integer()
            .unwrap(),
        17
    );
    assert_eq!(vm.eval_source("load-file-name").unwrap(), Fixed::NIL);
    assert_eq!(vm.eval_source("load-in-progress").unwrap(), Fixed::NIL);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn reentrant_load_rebases_live_frames_handlers_and_external_roots() {
    let dir = scratch("frames");
    // Force a much wider function than the suspended caller's register file.
    let args = (0..1000)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    std::fs::write(
        dir.join("wide.el"),
        format!("(+ {args}) (defun after-load () 42) (provide 'wide)"),
    )
    .unwrap();
    let mut vm = Vm::new();
    vm.set_load_path(std::slice::from_ref(&dir)).unwrap();
    let f=vm.compile_source("(defun nested-load (n) (if (= n 0) (condition-case e (progn (require 'wide) (after-load)) (error 99)) (+ 1 (nested-load (1- n))))) (nested-load 5)").unwrap();
    let value = vm.read_source("[λ 1.25]").unwrap().unwrap().0.root();
    assert_eq!(
        vm.execute(f)
            .unwrap()
            .as_integer()
            .unwrap(),
        47
    );
    vm.collect_garbage();
    assert_eq!(value.get(&vm).unwrap().display().unwrap(), "[λ 1.25]");
    std::fs::write(dir.join("broken.el"), "(car 1)").unwrap();
    assert_eq!(
        vm.eval_source("(condition-case e (load \"broken\") (error 71))")
            .unwrap()
            .as_integer()
            .unwrap(),
        71
    );
    assert_eq!(
        vm.eval_source("(after-load)")
            .unwrap()
            .as_integer()
            .unwrap(),
        42
    );
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn source_coding_is_explicit_and_invalid_utf8_is_not_replaced() {
    let dir = scratch("coding");
    std::fs::write(
        dir.join("latin.el"),
        b";;; -*- coding: latin-1; lexical-binding: t -*-\n(setq latin \"\xe9\")\n",
    )
    .unwrap();
    std::fs::write(dir.join("bad.el"), b"(setq bad \"\xff\")").unwrap();
    std::fs::write(dir.join("other.el"), ";;; -*- coding: shift_jis -*-\n1").unwrap();
    let mut vm = Vm::new();
    vm.load_file(&dir.join("latin.el")).unwrap();
    let v = vm.eval_source("latin").unwrap();
    assert_eq!(v.display().unwrap(), "\"é\"");
    assert!(
        vm.load_file(&dir.join("bad.el"))
            .unwrap_err()
            .contains("valid UTF-8")
    );
    assert!(
        vm.load_file(&dir.join("other.el"))
            .unwrap_err()
            .contains("unsupported source coding")
    );
    std::fs::remove_dir_all(dir).unwrap();
}
