use romacs_vm::Vm;

fn evaluate(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).unwrap().display().unwrap()
}

#[test]
fn family_setter_returns_transformed_copy_without_writing_configuration() {
    let mut vm = Vm::new();
    let actual = evaluate(&mut vm, r#"
      (let ((face-alternative-font-family-alist 'sentinel)
            (input (list (list "AbC" "DeF"))))
        (condition-case e
            (let ((result (internal-set-alternative-font-family-alist input)))
              (list result (eq input result) (eq (car input) (car result))
                    input face-alternative-font-family-alist))
          (error e)))"#);
    assert_eq!(actual, "(((AbC DeF)) nil nil ((\"AbC\" \"DeF\")) sentinel)",
               "FAMILY_COPIES_INTERNS_AND_PRESERVES_CONFIG");
}

#[test]
fn family_validation_errors_and_copy_precedence() {
    let mut vm = Vm::new();
    for (input, expected) in [
        ("42", "(wrong-type-argument listp 42)"),
        ("'(42)", "(wrong-type-argument listp 42)"),
        ("'((foo))", "(wrong-type-argument stringp foo)"),
        ("'((nil))", "(wrong-type-argument stringp nil)"),
        ("'((t))", "(wrong-type-argument stringp t)"),
        ("'(([1]))", "(wrong-type-argument stringp [1])"),
        ("'((42 . inner-tail))", "(wrong-type-argument listp inner-tail)"),
        ("'((42) . outer-tail)", "(wrong-type-argument listp outer-tail)"),
        ("'((42) (\"A\" . later-tail))", "(wrong-type-argument stringp 42)"),
    ] {
        assert_eq!(evaluate(&mut vm, &format!("(condition-case e (internal-set-alternative-font-family-alist {input}) (error e))")), expected, "FAMILY_VALIDATION {input}");
    }
}

#[test]
fn family_cycles_signal_original_spine_before_conversion() {
    let mut vm = Vm::new();
    assert_eq!(evaluate(&mut vm, r#"
      (let* ((outer (list (list 42))) (inner (list 42)))
        (setcdr outer outer)
        (setcdr inner inner)
        (list
          (condition-case e (internal-set-alternative-font-family-alist outer)
            (error (list (car e) (eq (car (cdr e)) outer))))
          (condition-case e (internal-set-alternative-font-family-alist (list inner))
            (error (list (car e) (eq (car (cdr e)) inner))))))"#),
      "((circular-list t) (circular-list t))", "FAMILY_CYCLE_DATA_ORIGINAL_SPINE");
}

#[test]
fn family_interning_respects_copy_boundaries_and_partial_side_effects() {
    let mut vm = Vm::new();
    assert_eq!(evaluate(&mut vm, r#"
      (let ((obarray (obarray-make)))
        (list
          (condition-case e
              (internal-set-alternative-font-family-alist '(("outer-not-interned") . tail))
            (error e))
          (intern-soft "outer-not-interned")
          (condition-case e
              (internal-set-alternative-font-family-alist '(("inner-not-interned" . tail)))
            (error e))
          (intern-soft "inner-not-interned")
          (condition-case e
              (internal-set-alternative-font-family-alist '(("earlier-row") ("earlier-member" 42)))
            (error e))
          (not (null (intern-soft "earlier-row")))
          (not (null (intern-soft "earlier-member")))))"#),
      "((wrong-type-argument listp tail) nil (wrong-type-argument listp tail) nil (wrong-type-argument stringp 42) t t)",
      "FAMILY_VALIDATION_BEFORE_INTERNING_AND_PARTIAL_EFFECTS");
}

#[test]
fn family_uses_current_obarray_preserves_case_duplicates_and_empty_rows() {
    let mut vm = Vm::new();
    assert_eq!(evaluate(&mut vm, r#"
      (let ((ordinary (intern "FontCase")) (obarray (obarray-make)))
        (let* ((input '(nil ("FontCase" "FontCase" "nil" "t") nil))
               (result (internal-set-alternative-font-family-alist input)))
          (list result (eq ordinary (car (car (cdr result))))
            (eq (intern-soft "FontCase") (car (car (cdr result))))
            (eq (car (car (cdr result))) (car (cdr (car (cdr result)))))
            input)))"#),
      "((nil (FontCase FontCase nil t) nil) nil t t (nil (\"FontCase\" \"FontCase\" \"nil\" \"t\") nil))",
      "FAMILY_CURRENT_OBARRAY_NOT_GLOBAL_CASE_OR_NORMALIZATION");
    assert_eq!(vm.font_alternative_state().unwrap().family_alist(), &[vec![], vec!["FontCase".to_owned(), "FontCase".to_owned(), "nil".to_owned(), "t".to_owned()], vec![]]);
}

#[test]
fn family_success_always_invalidates_and_failure_preserves_publication() {
    let mut vm = Vm::new();
    assert_eq!(vm.font_alternative_state().unwrap().revision(), 0);
    for expected_revision in 1..=3 {
        vm.frame_snapshot(400, 300).unwrap();
        assert!(vm.published_window_count() > 0);
        let version = vm.display_version();
        evaluate(&mut vm, "(internal-set-alternative-font-family-alist '((\"AbC\" \"DeF\")))");
        assert_eq!(vm.font_alternative_state().unwrap().revision(), expected_revision, "FAMILY_EVERY_SUCCESS_BUMPS_REVISION");
        assert_eq!(vm.published_window_count(), 0, "FAMILY_EVERY_SUCCESS_CLEARS_PREVIOUS");
        assert_eq!(vm.display_version(), version + 1, "FAMILY_EVERY_SUCCESS_BUMPS_DISPLAY_VERSION");
        let snapshot = vm.frame_snapshot(400, 300).unwrap();
        assert_eq!(snapshot.font_alternatives, vm.font_alternative_state().unwrap(), "FAMILY_SNAPSHOT_PUBLISHES_STATE");
        let before = vm.font_alternative_state().unwrap();
        let count = vm.published_window_count();
        let version = vm.display_version();
        assert_eq!(evaluate(&mut vm, "(condition-case e (internal-set-alternative-font-family-alist '((\"partial\" 42))) (error e))"), "(wrong-type-argument stringp 42)");
        assert_eq!(vm.font_alternative_state().unwrap(), before, "FAMILY_FAILURE_STATE_ATOMIC");
        assert_eq!(vm.published_window_count(), count, "FAMILY_FAILURE_CACHE_ATOMIC");
        assert_eq!(vm.display_version(), version, "FAMILY_FAILURE_VERSION_ATOMIC");
    }
    evaluate(&mut vm, "(internal-set-alternative-font-family-alist nil)");
    assert!(vm.font_alternative_state().unwrap().family_alist().is_empty());
    assert_eq!(vm.font_alternative_state().unwrap().revision(), 4);
}

#[test]
fn family_raw_root_is_return_identity_traced_and_live_for_snapshot() {
    let mut vm = Vm::new();
    let returned = vm.eval_source(r#"(setq family-result
      (let ((obarray (obarray-make)))
        (internal-set-alternative-font-family-alist '(("PrivateFont")))))"#).unwrap().root();
    assert!(returned.get(&vm).unwrap().eq(vm.font_alternative_family_root()), "FAMILY_RAW_ROOT_EXACT_RETURN_IDENTITY");
    drop(returned);
    assert_eq!(vm.font_alternative_family_root().display().unwrap(), "((PrivateFont))");
    vm.collect_garbage();
    assert_eq!(vm.font_alternative_family_root().display().unwrap(), "((PrivateFont))", "FAMILY_PRIVATE_OBARRAY_SYMBOL_ROOT_SURVIVES_GC");
    vm.frame_snapshot(400, 300).unwrap();
    assert!(vm.published_window_count() > 0);
    let version = vm.display_version();
    let revision = vm.font_alternative_state().unwrap().revision();
    // Native root alone must keep the mutated list alive after this call.
    evaluate(&mut vm, "(progn (setcar (car family-result) 'ChangedFont) (setcdr family-result (list (list 'AddedFont))) (setq family-result nil))");
    vm.collect_garbage();
    let mutated = vm.font_alternative_state().unwrap();
    assert_eq!(mutated.revision(), revision + 1, "FAMILY_RAW_MUTATION_BUMPS_REVISION");
    assert_eq!(vm.display_version(), version + 1, "FAMILY_RAW_MUTATION_BUMPS_VERSION");
    assert_eq!(vm.published_window_count(), 0, "FAMILY_RAW_MUTATION_CLEARS_PREVIOUS");
    let snapshot = vm.frame_snapshot(400, 300).unwrap();
    assert_eq!(snapshot.font_alternatives.family_alist(), &[vec!["ChangedFont".to_owned()], vec!["AddedFont".to_owned()]], "FAMILY_RETURN_MUTATION_REACHES_SNAPSHOT_AFTER_GC");
    assert_eq!(vm.font_alternative_family_root().display().unwrap(), "((ChangedFont) (AddedFont))");
    let state = vm.font_alternative_state().unwrap();
    assert_eq!(snapshot.font_alternatives, state);
    assert_eq!(vm.font_alternative_state().unwrap(), state, "FAMILY_READ_WITHOUT_MUTATION_STABLE");
}

#[test]
fn family_native_root_alone_traces_private_obarray_symbols() {
    let mut vm = Vm::new();
    vm.eval_source(r#"(let ((obarray (obarray-make)))
      (internal-set-alternative-font-family-alist '(("OnlyNativeRoot" "nil" "t"))))"#).unwrap();
    vm.collect_garbage();
    vm.collect_garbage();
    assert_eq!(vm.font_alternative_family_root().display().unwrap(), "((OnlyNativeRoot nil t))");
    assert_eq!(vm.frame_snapshot(400, 300).unwrap().font_alternatives.family_alist(), &[vec!["OnlyNativeRoot".to_owned(), "nil".to_owned(), "t".to_owned()]], "FAMILY_NATIVE_ROOT_ALONE_PRESERVES_PRIVATE_SYMBOL_NAMES");
}

#[test]
fn family_later_malformed_mutation_is_rejected_not_silently_dropped() {
    let mut vm = Vm::new();
    for mutation in [
        "(setcar (car family-result) 42)",
        "(setcdr (car family-result) 'dotted)",
        "(setcdr family-result 'dotted)",
        "(setcdr family-result family-result)",
        "(setcdr (car family-result) (car family-result))",
    ] {
        evaluate(&mut vm, "(setq family-result (internal-set-alternative-font-family-alist '((\"Valid\"))))");
        vm.eval_source(mutation).unwrap();
        assert!(vm.font_alternative_state().is_err(), "FAMILY_MALFORMED_MUTATION_STATE {mutation}");
        assert!(vm.frame_snapshot(400, 300).is_err(), "FAMILY_MALFORMED_MUTATION_SNAPSHOT {mutation}");
    }
}

#[test]
fn registry_setter_matches_pinned_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = Vm::new();
    for input in [
        "nil", "42", "'(42)", "'((\"ISO8859-1\" \"Iso10646-1\") (\"gb2312.1980\" \"GBK\"))",
        "'((\"A\" ?B 66))", "'((foo))", "'((\"A\" . tail))", "'((\"A\") . tail)",
        "'((\"A\") 42 (sym))", "(list (list \"ÉCOLE\" \"Λ\"))",
    ] {
        let source = format!(r#"(prin1-to-string (condition-case e
            (let* ((input {input}) (result (internal-set-alternative-font-registry-alist input)))
              (list result (and result (eq input result)) (and (consp result) (consp (car result)) (eq (car input) (car result))) input))
          (error e)))"#);
        let output = std::process::Command::new(&oracle).args(["-Q", "--batch", "--eval", &format!("(princ {source})")]).output().unwrap();
        assert!(output.status.success(), "{input}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual: String = serde_json::from_str(&evaluate(&mut vm, &source)).unwrap();
        assert_eq!(actual, expected, "REGISTRY {input}");
    }
}
