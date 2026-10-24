use romacs_vm::Vm;

fn check(vm: &mut Vm, source: &str, expected: &str) {
    let actual = match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(error) => format!("VM error: {error}"),
    };
    assert_eq!(actual, expected, "{source}");
}

#[test]
fn obsolete_function_metadata_preserves_and_replaces_properties() {
    check(&mut Vm::new(), r#"(progn
      (put 'old 'unrelated '(keep))
      (make-obsolete 'old 'new "31")
      (setq previous (get 'old 'byte-obsolete-info))
      (list (make-obsolete 'old "use something else." "32")
            previous (get 'old 'byte-obsolete-info) (get 'old 'unrelated)))"#,
      r#"(old (new nil "31") ("use something else." nil "32") (keep))"#);
}

#[test]
fn obsolete_variable_metadata_keeps_optional_access_verbatim() {
    check(&mut Vm::new(), r#"(progn
      (put 'old 'unrelated 99)
      (make-obsolete-variable 'old 'new "31")
      (setq previous (get 'old 'byte-obsolete-variable))
      (list (make-obsolete-variable 'old nil "32" 'set)
            previous (get 'old 'byte-obsolete-variable) (get 'old 'unrelated)))"#,
      r#"(old (new nil "31") (nil set "32") 99)"#);
}

#[test]
fn obsolete_constant_and_non_symbol_errors_do_not_write_properties() {
    check(&mut Vm::new(), r#"(list
      (condition-case nil (make-obsolete nil 'x "32") (error 'constant))
      (condition-case nil (make-obsolete-variable t 'x "32") (error 'constant))
      (condition-case nil (make-obsolete 3 'x "32") (wrong-type-argument 'type))
      (condition-case nil (make-obsolete-variable 'old 'x) (wrong-number-of-arguments 'arity))
      (get nil 'byte-obsolete-info) (get t 'byte-obsolete-variable))"#,
      "(constant constant type arity nil nil)");
}

#[test]
fn obsolete_alias_macro_installs_alias_and_repeats_parameter_evaluation() {
    check(&mut Vm::new(), r#"(progn
      (setq old-count 0 new-count 0)
      (defun target (x) (+ x 7))
      (list
        (define-obsolete-function-alias
          (progn (setq old-count (+ old-count 1)) 'old)
          (progn (setq new-count (+ new-count 1)) 'target)
          "32" "legacy documentation")
        (old 5) old-count new-count (get 'old 'byte-obsolete-info)))"#,
      r#"(old 12 2 2 (target nil "32"))"#);
}

#[test]
fn advertised_convention_uses_resolved_function_identity_and_returns_signature() {
    check(&mut Vm::new(), r#"(progn
      (defun target (x) x)
      (defalias 'alias 'target)
      (list (set-advertised-calling-convention 'alias '(a &optional b) "32")
            (gethash (get 'target 'unused) advertised-signature-table 'missing)
            (hash-table-count advertised-signature-table)
            (hash-table-test advertised-signature-table)
            (hash-table-weakness advertised-signature-table)
            (get 'alias 'advertised-calling-convention)))"#,
      "((a &optional b) missing 1 eq key nil)");
    check(&mut Vm::new(), r#"(progn
      (set-advertised-calling-convention 'not-defined '(x) nil)
      (list (gethash nil advertised-signature-table)
            (set-advertised-calling-convention 17 '(z) nil)
            (gethash 17 advertised-signature-table)))"#,
      "((x) (z) (z))");
}

#[test]
fn advertised_convention_replaces_shared_alias_key_not_function_definition() {
    check(&mut Vm::new(), r#"(progn
      (defun target (x) (+ x 1))
      (defalias 'alias 'target)
      (set-advertised-calling-convention 'alias '(old) "31")
      (set-advertised-calling-convention 'target '(new) "32")
      (list (hash-table-count advertised-signature-table) (alias 9)))"#,
      "(1 10)");
}

#[test]
fn metadata_survives_counted_moving_collections_through_existing_owners() {
    let mut vm = Vm::new();
    vm.set_collection_floor(usize::MAX);
    check(&mut vm, r#"(progn
      (dotimes (i 80) (copy-sequence "dead"))
      (make-obsolete 'old (copy-sequence "replacement") (copy-sequence "32"))
      (make-obsolete-variable 'var (vector (copy-sequence "next")) "32" 'get)
      (set-advertised-calling-convention 17 (list (copy-sequence "argument")) "32")
      nil)"#, "nil");
    let before = vm.heap_stats().collections;
    vm.collect_garbage();
    assert!(vm.heap_stats().collections > before, "fixture must actually collect");
    vm.set_collection_floor(1);
    check(&mut vm, r#"(list (get 'old 'byte-obsolete-info)
      (get 'var 'byte-obsolete-variable) (gethash 17 advertised-signature-table))"#,
      r#"(("replacement" nil "32") (["next"] get "32") ("argument"))"#);
    assert!(vm.heap_stats().collections > before + 1, "evaluation must also collect");
}
