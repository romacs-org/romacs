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
/// A condition that escapes a required file reaches the requiring file's
/// top level with its data, as GNU prints `(CONDITION . DATA)'; the loader
/// once reported only `unhandled signal CONDITION'.
#[test]
fn a_condition_crossing_require_keeps_its_data() {
    let dir = scratch("crossing");
    std::fs::write(dir.join("signals.el"), "(signal 'wrong-type-argument (list 'stringp 42)) (provide 'signals)").unwrap();
    std::fs::write(dir.join("errs.el"), "(error \"Boom %d\" 7) (provide 'errs)").unwrap();
    std::fs::write(dir.join("top-signal.el"), "(require 'signals)").unwrap();
    std::fs::write(dir.join("top-error.el"), "(require 'errs)").unwrap();
    let mut vm = Vm::new();
    vm.set_load_path(std::slice::from_ref(&dir)).unwrap();
    let error = vm.load_file(&dir.join("top-signal.el")).unwrap_err();
    assert!(error.ends_with("(wrong-type-argument stringp 42)"), "{error}");
    let error = vm.load_file(&dir.join("top-error.el")).unwrap_err();
    assert!(error.ends_with("(error \"Boom 7\")"), "{error}");
    assert_eq!(vm.eval_source("(signal 'args-out-of-range '(\"ab\" 5))").unwrap_err(), "(args-out-of-range \"ab\" 5)");
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
    // GNU decode_coding_utf_8 (pinned GNU's answers): a byte that starts no
    // character is a raw-byte character; a 4-byte sequence may name a
    // character beyond Unicode, which a file without a cookie still detects
    // as UTF-8 (quick-cns.el).
    std::fs::write(dir.join("raw.el"), b";; -*- coding: utf-8 -*-\n(setq raw \"\xe6\xb1\x89 \xa3\xa0\")\n").unwrap();
    std::fs::write(dir.join("wide.el"), b"(setq wide \"\xf4\x94\x80\x99\")\n").unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&dir.join("raw.el")).unwrap();
    vm.load_file(&dir.join("wide.el")).unwrap();
    assert_eq!(vm.eval_source("(list (multibyte-string-p raw) (mapcar #'identity raw) (mapcar #'identity wide))").unwrap().display().unwrap(),
               "(t (27721 32 4194211 4194208) (1130521))");
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

/// D2 for symbols: a raw byte in a symbol read from explicit UTF-8 source is
/// a raw-byte character of its name, as in a string; the symbol interns,
/// prints and compares as GNU's. A genuine U+FDD0 (VM2's internal key
/// escape, symkey.rs) is an ordinary name character. Pinned GNU 32.0.50.
#[test]
fn raw_bytes_in_source_symbols_keep_their_characters() {
    let dir = scratch("raw-symbols");
    std::fs::write(dir.join("s2.el"), b";; -*- coding: utf-8; lexical-binding: t -*-\n(defvar s2-sym (quote raw\xff))\n(defvar s2-list (list (quote raw\xff) \"r\xff\" (quote \xef\xb7\x90x)))\n").unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&dir.join("s2.el")).unwrap();
    let form = "(list (append (symbol-name s2-sym) nil) (eq s2-sym (intern (string ?r ?a ?w #x3fffff))) (eq s2-sym (car s2-list))
      (eq s2-sym (intern-soft (string ?r ?a ?w #x3fffff))) (append (prin1-to-string s2-sym) nil) (append (prin1-to-string s2-list) nil)
      (append (format \"%s\" s2-sym) nil) (append (symbol-name (nth 2 s2-list)) nil) (eq (nth 2 s2-list) (intern (string #xfdd0 ?x)))
      (append (symbol-name (make-symbol (string ?a #x3fffff))) nil) (eq (make-symbol \"raw\") (intern \"raw\")) (multibyte-string-p (symbol-name s2-sym)))";
    assert_eq!(vm.eval_source(form).unwrap().display().unwrap(),
        "((114 97 119 4194303) t t t (114 97 119 4194303) (40 114 97 119 4194303 32 34 114 92 51 55 55 34 32 64976 120 41) (114 97 119 4194303) (64976 120) t (97 4194303) nil t)");
    std::fs::remove_dir_all(dir).unwrap();
}
