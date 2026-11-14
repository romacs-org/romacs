use romacs_vm::Vm;

fn probes() -> Vec<String> {
    let mut cases = Vec::new();
    for source in [
        r#"(string-as-multibyte "abc")"#,
        r#"(copy-sequence "λ🙂z")"#,
        "(unibyte-string 97 255 98)",
        "(string 4194281 955 2097152)",
    ] {
        for properties in [
            "nil",
            "'(p nil)",
            "'(p 1 q 2)",
            "'(p 1 p 2)",
            "'(nil 1 t 2)",
            "'p",
            "'(p 1 . tail)",
            "'(p 1 q)",
        ] {
            for (start, end) in [(0, 3), (2, 0), (99, 99), (-1, 2)] {
                cases.push(format!("(let ((s {source})) (list (condition-case e (add-text-properties {start} {end} {properties} s) (error e)) (text-properties-at 0 s) (text-properties-at 1 s) (text-properties-at 2 s) (multibyte-string-p s) (vconcat s)))"));
            }
        }
    }
    for category in ["nil", "t"] {
        cases.push(format!("(progn (put {category} 'read-only 'locked) (with-temp-buffer (insert \"abc\") (add-text-properties 1 4 '(category {category})) (list (condition-case e (add-text-properties 1 4 '(p 9)) (error e)) (text-properties-at 1))))"));
        cases.push(format!("(progn (put {category} 'inhibit-read-only t) (with-temp-buffer (insert \"abc\") (add-text-properties 1 4 '(category {category})) (let ((buffer-read-only t)) (list (condition-case e (add-text-properties 1 4 '(p 9)) (error (car e))) (text-properties-at 1)))))"));
        cases.push(format!("(progn (put {category} 'read-only nil) (with-temp-buffer (insert \"abc\") (add-text-properties 1 4 '(category {category} alternate locked)) (let ((char-property-alias-alist '((read-only alternate)))) (list (condition-case e (add-text-properties 1 4 '(p 9)) (error e)) (text-properties-at 1)))))"));
    }
    cases.extend([
        r#"(let ((s (copy-sequence "abc"))) (condition-case e (add-text-properties #x8000000000000000 #x8000000000000000 (quote (p 1)) s) (error e)))"#,
        r#"(let ((s (copy-sequence "abc"))) (condition-case e (add-text-properties 0 #x8000000000000000 (quote (p 1)) s) (error e)))"#,
        r#"(let ((s (copy-sequence "abc"))) (condition-case e (add-text-properties #x8000000000000000 0 (quote (p 1)) s) (error e)))"#,
        r#"(let ((s (copy-sequence "abc"))) (condition-case e (add-text-properties #x8000000000000001 #x8000000000000001 (quote (p 1)) s) (error e)))"#,
        r#"(let ((s (copy-sequence "abc"))) (condition-case e (add-text-properties 0 #x8000000000000001 (quote (p 1)) s) (error e)))"#,
        r#"(let ((s (copy-sequence "abc"))) (condition-case e (add-text-properties #x8000000000000001 0 (quote (p 1)) s) (error e)))"#,
        r#"(let ((s (copy-sequence "abc"))) (condition-case e (add-text-properties (- #x8000000000000000) (- #x8000000000000000) (quote (p 1)) s) (error e)))"#,
        r#"(let ((s (copy-sequence "abc"))) (condition-case e (add-text-properties 0 (- #x8000000000000000) (quote (p 1)) s) (error e)))"#,
        r#"(let ((s (copy-sequence "abc"))) (condition-case e (add-text-properties (- #x8000000000000000) 0 (quote (p 1)) s) (error e)))"#,
        r#"(let ((s (copy-sequence "abc"))) (with-temp-buffer (insert "abc") (condition-case e (add-text-properties #x8000000000000000 #x8000000000000001 (quote (p 1)) s) (error e))))"#,
        r#"(let ((s (copy-sequence "abc"))) (with-temp-buffer (insert "abc") (condition-case e (add-text-properties #x8000000000000000 (- #x8000000000000000) (quote (p 1)) s) (error e))))"#,
        r#"(let ((s (copy-sequence "abc"))) (with-temp-buffer (insert "abc") (condition-case e (add-text-properties (- #x8000000000000000) (- #x8000000000000001) (quote (p 1)) s) (error e))))"#,
        r#"(let ((s (copy-sequence "abc"))) (with-temp-buffer (insert "abc") (condition-case e (add-text-properties #x8000000000000000 #x8000000000000001 (quote (p 1)) nil) (error e))))"#,
        r#"(let ((s (copy-sequence "abc"))) (with-temp-buffer (insert "abc") (condition-case e (add-text-properties #x8000000000000000 (- #x8000000000000000) (quote (p 1)) nil) (error e))))"#,
        r#"(let ((s (copy-sequence "abc"))) (with-temp-buffer (insert "abc") (condition-case e (add-text-properties (- #x8000000000000000) (- #x8000000000000001) (quote (p 1)) nil) (error e))))"#,
        r#"(with-temp-buffer (insert "abc") (add-text-properties #x8000000000000000 #x8000000000000001 '(p 1) (current-buffer)))"#,
        r#"(with-temp-buffer (insert "abc") (condition-case e (add-text-properties 1 #x8000000000000000 '(p 1) (current-buffer)) (error e)))"#,
        r#"(condition-case e (add-text-properties #x8000000000000000 'bad '(p 1) "abc") (error e))"#,
        r#"(condition-case e (add-text-properties 'bad #x8000000000000000 '(p 1) "abc") (error e))"#,
        r#"(condition-case e (add-text-properties #x8000000000000000 (make-marker) '(p 1) "abc") (error e))"#,
        r#"(condition-case e (add-text-properties #x8000000000000000 #x8000000000000001 '(p 1) 15) (error e))"#,
        r#"(add-text-properties #x8000000000000000 #x1fffffffffffffff '(p 1) "abc")"#,
        r#"(with-temp-buffer (insert "abc") (let ((m (copy-marker 1)) (end #x8000000000000000)) (condition-case e (add-text-properties m end '(p 1)) (error (list (car e) (eq (car (cdr e)) m) (eq (car (cdr (cdr e))) end))))))"#,
    ].into_iter().map(str::to_owned));
    cases.extend([
        r#"(list (boundp 'overriding-plist-environment) (special-variable-p 'overriding-plist-environment) overriding-plist-environment (let ((overriding-plist-environment '((c p 9)))) overriding-plist-environment) overriding-plist-environment)"#,
        r#"(with-temp-buffer (insert "abc") (add-text-properties 1 4 '(category c)) (let ((overriding-plist-environment '((c read-only locked)))) (list (condition-case e (add-text-properties 1 4 '(p 9)) (error e)) (text-properties-at 1))))"#,
        r#"(with-temp-buffer (insert "abc") (add-text-properties 1 4 '(category c)) (let ((overriding-plist-environment '((c inhibit-read-only t))) (buffer-read-only t)) (list (condition-case e (add-text-properties 1 4 '(p 9)) (error (car e))) (text-properties-at 1))))"#,
        r#"(progn (put 'c 'read-only 'locked) (with-temp-buffer (insert "abc") (add-text-properties 1 4 '(category c)) (let ((overriding-plist-environment '((c read-only nil)))) (list (condition-case e (add-text-properties 1 4 '(p 9)) (error e)) (text-properties-at 1)))))"#,
        r#"(with-temp-buffer (insert "abc") (add-text-properties 1 4 '(category c p 9)) (let ((overriding-plist-environment '((c read-only locked)))) (list (add-text-properties 1 4 '(p 9)) (text-properties-at 1))))"#,
        r#"(with-temp-buffer (insert "abc") (put 'c 'alternate t) (add-text-properties 1 4 '(category c read-only locked)) (let ((char-property-alias-alist '((inhibit-read-only alternate)))) (list (condition-case e (add-text-properties 1 4 '(p 8)) (error e)) (text-properties-at 1))))"#,
        r#"(with-temp-buffer (insert "abc") (put 'c 'alternate 'locked) (add-text-properties 1 4 '(category c)) (let ((char-property-alias-alist '((read-only alternate)))) (list (condition-case e (add-text-properties 1 4 '(p 8)) (error e)) (text-properties-at 1))))"#,
        r#"(with-temp-buffer (insert "abc") (add-text-properties 1 4 '(category 7)) (list (add-text-properties 1 4 '(p 8)) (text-properties-at 1)))"#,
        r#"(with-temp-buffer (insert "abc") (add-text-properties 1 4 '(category "c")) (list (add-text-properties 1 4 '(p 8)) (text-properties-at 1)))"#,
        r#"(with-temp-buffer (insert "abc") (add-text-properties 1 4 '(category [7])) (list (add-text-properties 1 4 '(p 8)) (text-properties-at 1)))"#,
        r#"(with-temp-buffer (insert "abc") (add-text-properties 1 4 '(read-only locked)) (let ((inhibit-read-only '(locked . tail))) (list (condition-case e (add-text-properties 1 4 '(p 8)) (error e)) (text-properties-at 1))))"#,
        r#"(with-temp-buffer (insert "abc") (add-text-properties 1 4 '(read-only locked)) (let ((inhibit-read-only '(other . tail))) (list (condition-case e (add-text-properties 1 4 '(p 8)) (error e)) (text-properties-at 1))))"#,
        r#"(with-temp-buffer (insert "abc") (let ((inhibit-read-only '(other . tail))) (list (add-text-properties 1 4 '(p 8)) (text-properties-at 1))))"#,
        r#"(with-temp-buffer (insert "abc") (add-text-properties 1 4 '(alternate locked)) (let ((char-property-alias-alist '((read-only alternate) . tail))) (list (condition-case e (add-text-properties 1 4 '(p 8)) (error e)) (text-properties-at 1))))"#,
        r#"(with-temp-buffer (insert "abc") (let ((char-property-alias-alist '(other . tail))) (list (condition-case e (add-text-properties 1 4 '(p 8)) (error e)) (text-properties-at 1))))"#,
        r#"(with-temp-buffer (insert "abc") (put 'c 'read-only 'locked) (add-text-properties 1 4 '(category c read-only nil)) (list (add-text-properties 1 4 '(p 8)) (text-properties-at 1)))"#,
        r#"(with-temp-buffer (insert "abc") (let ((default-text-properties '(read-only locked))) (list (condition-case e (add-text-properties 1 4 '(p 8)) (error e)) (text-properties-at 1))))"#,
        r#"(let ((target (get-buffer-create "property-killed"))) (kill-buffer target) (list (add-text-properties 99 99 '(p 1) target) (condition-case e (add-text-properties 1 2 '(p 1) target) (error e)) (condition-case e (add-text-properties 1 1 '(p 1 bad) target) (error e))))"#,
        r#"(let ((s (string-as-multibyte "abc")) (v (vector 1)) (w (vector 1))) (list (add-text-properties 0 3 (list 'p v) s) (add-text-properties 0 3 (list 'p v) s) (add-text-properties 0 3 (list 'p w) s) (eq w (get-text-property 1 'p s))))"#,
        r#"(let ((origin (current-buffer)) (target (get-buffer-create "property-killed"))) (kill-buffer target) (list (condition-case e (add-text-properties 1 1 nil target) (error (car e))) (eq origin (current-buffer))))"#,
        r#"(list (boundp 'inhibit-modification-hooks) (special-variable-p 'inhibit-modification-hooks) (let ((inhibit-modification-hooks 17)) inhibit-modification-hooks) inhibit-modification-hooks)"#,
        r#"(with-temp-buffer (insert "abc") (let ((inhibit-modification-hooks t) (before-change-functions (list (lambda (&rest args) (error "must stay inhibited"))))) (list (add-text-properties 1 4 '(p 7)) (text-properties-at 1) inhibit-modification-hooks)))"#,
        r#"(with-temp-buffer (insert "abc") (put-text-property 1 4 'modification-hooks (list (lambda (&rest args) (error "must stay inhibited")))) (let ((inhibit-modification-hooks t)) (list (add-text-properties 1 4 '(p 7)) (get-text-property 1 'p))))"#,
        r#"(with-temp-buffer (insert "abc") (put 'property-category 'read-only 'locked) (put-text-property 1 4 'category 'property-category) (list (condition-case e (add-text-properties 1 4 '(p 7)) (error e)) (let ((inhibit-read-only '(locked))) (add-text-properties 1 4 '(p 7))) (text-properties-at 1)))"#,
        r#"(with-temp-buffer (insert "abc") (put-text-property 1 2 'inhibit-read-only t) (let ((buffer-read-only t)) (list (condition-case e (add-text-properties 1 4 '(p 7)) (error (car e))) (get-text-property 1 'p) (get-text-property 2 'p))))"#,
        r#"(let ((s (copy-sequence "λ🙂z"))) (list (add-text-properties 0 2 '(p nil q 2) s) (add-text-properties 0 2 '(p nil q 2) s) (add-text-properties 1 3 '(p 1 p 2) s) (add-text-properties 1 3 '(p 1 p 2) s) (text-properties-at 0 s) (text-properties-at 1 s) (text-properties-at 2 s)))"#,
        r#"(with-temp-buffer (insert "λ🙂z") (let ((tick (buffer-modified-tick)) (chars (buffer-chars-modified-tick))) (list (add-text-properties 1 3 '(p nil q 2)) (add-text-properties 1 3 '(p nil q 2)) (- (buffer-modified-tick) tick) (= chars (buffer-chars-modified-tick)) (text-properties-at 1) (text-properties-at 2) (text-properties-at 3) (point))))"#,
        r#"(with-temp-buffer (insert "abc") (let ((m (copy-marker 2))) (list (add-text-properties m 4 '(p 7)) (marker-position m) (point) (text-properties-at 1) (text-properties-at 2))))"#,
        r#"(with-temp-buffer (insert "abcd") (narrow-to-region 2 4) (list (condition-case e (add-text-properties 1 3 '(p 7)) (error e)) (add-text-properties 2 4 '(p 7)) (text-properties-at 2) (text-properties-at 3)))"#,
        r#"(with-temp-buffer (insert "abc") (let ((buffer-read-only t)) (list (condition-case e (add-text-properties 1 4 '(p 7)) (error (car e))) (text-properties-at 1) (let ((inhibit-read-only t)) (add-text-properties 1 4 '(p 7))) (add-text-properties 1 4 '(p 7)))))"#,
        r#"(with-temp-buffer (insert "abc") (put-text-property 1 4 'read-only 'locked) (list (condition-case e (add-text-properties 1 4 '(read-only nil p 7)) (error e)) (get-text-property 1 'p) (let ((inhibit-read-only '(locked))) (add-text-properties 1 4 '(read-only nil p 7))) (text-properties-at 1)))"#,
        r#"(with-temp-buffer (insert "abc") (put-text-property 1 4 'read-only "locked") (list (condition-case e (add-text-properties 1 4 '(p 7)) (error e)) (text-properties-at 1)))"#,
        r#"(with-temp-buffer (insert "abc") (put-text-property 1 4 'inhibit-read-only t) (let ((buffer-read-only t)) (list (add-text-properties 1 4 '(p 7)) (text-properties-at 1))))"#,
        r#"(let ((origin (current-buffer)) (target (get-buffer-create "property-target"))) (with-current-buffer target (insert "λ🙂z")) (list (add-text-properties 1 3 '(p 7 q 8) target) (eq origin (current-buffer)) (with-current-buffer target (list (text-properties-at 1) (point)))))"#,
        r#"(let ((origin (current-buffer)) (target (get-buffer-create "property-target"))) (with-current-buffer target (insert "abc") (setq buffer-read-only t)) (list (condition-case e (add-text-properties 1 4 '(p 7) target) (error (car e))) (eq origin (current-buffer)) (with-current-buffer target (get-text-property 1 'p))))"#,
        r#"(with-temp-buffer (insert "abc") (let ((s (copy-sequence "xyz"))) (list (add-text-properties (copy-marker 1) (copy-marker 3) '(p 1) s) (text-properties-at 0 s) (text-properties-at 1 s) (text-properties-at 2 s))))"#,
        r#"(list (add-text-properties 'bad 'bad nil 15) (condition-case e (add-text-properties 0 0 '(p 1) 15) (error e)) (condition-case e (add-text-properties (make-marker) 1 '(p 1) "a") (error e)))"#,
        r#"(let ((s (copy-sequence "abc"))) (put-text-property 0 1 'p 1 s) (list (condition-case e (add-text-properties 0 3 '(q 2 bad) s) (error e)) (text-properties-at 0 s) (text-properties-at 1 s)))"#,
    ].into_iter().map(str::to_owned));
    cases
}

#[test]
fn bulk_merge_retains_presence_order_and_changed_result() {
    let mut vm = Vm::new();
    let source = r#"(let ((s (copy-sequence "λ🙂z"))) (list (add-text-properties 0 2 '(p nil q 2) s) (add-text-properties 0 2 '(p nil q 2) s) (add-text-properties 1 3 '(p 1 p 2) s) (add-text-properties 1 3 '(p 1 p 2) s) (text-properties-at 0 s) (text-properties-at 1 s) (text-properties-at 2 s)))"#;
    assert_eq!(
        vm.eval_source(source).unwrap().display().unwrap(),
        "(t nil t t (q 2 p nil) (q 2 p 2) (p 2))"
    );
}

#[test]
fn bulk_properties_match_live_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    let cases = probes();
    let count = cases.len();
    for expression in cases {
        let source = format!("(condition-case e {expression} (error (list 'signal e)))");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "GNU probe failed for {expression}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            failures.push(format!("{expression}\nGNU {expected}\nVM {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    eprintln!("GNU property comparisons matched: {count}");
}

#[test]
fn property_values_survive_collection_and_invalid_plists_are_atomic() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let source = r#"(let* ((value (vector (lambda () 7))) (s (copy-sequence "λ🙂z")))
      (add-text-properties 0 2 (list 'p value 'q nil) s)
      (garbage-collect)
      (list (eq value (get-text-property 1 'p s))
        (funcall (aref (get-text-property 0 'p s) 0))
        (condition-case e (add-text-properties 0 3 '(new 1 bad) s) (error (car e)))
        (get-text-property 0 'new s) (get-text-property 2 'p s)))"#;
    assert_eq!(
        vm.eval_source(source).unwrap().display().unwrap(),
        "(t 7 error nil nil)"
    );
}

#[test]
fn change_hooks_run_and_unsupported_keys_refuse_without_mutation() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    // GNU (lexical): the hook runs once; inhibited, it does not run again.
    let source = r#"(with-temp-buffer (insert "abc")
      (let* ((calls 0) (before-change-functions (list (lambda (&rest args) (setq calls (1+ calls))))))
        (list (condition-case e (add-text-properties 1 4 '(p 7)) (error e))
          (get-text-property 1 'p) calls
          (let ((inhibit-modification-hooks t)) (add-text-properties 1 4 '(p 8)))
          calls (get-text-property 1 'p)
          (condition-case e (add-text-properties 1 4 (list 'p 9 7 9)) (error e))
          (get-text-property 1 'p) (point))))"#;
    assert_eq!(
        vm.eval_source(source).unwrap().display().unwrap(),
        "(t 7 1 t 1 8 (error \"text property keys other than symbols are not implemented\") 8 4)"
    );
}

#[test]
fn bignum_positions_use_native_sign_clamping_before_empty_range() {
    let mut vm = Vm::new();
    let source = r#"(list
      (add-text-properties #x8000000000000000 #x8000000000000001 '(p 1) "abc")
      (add-text-properties (- #x8000000000000000) (- #x8000000000000001) '(p 1) "abc")
      (condition-case e (add-text-properties 0 #x8000000000000000 '(p 1) "abc") (error e)))"#;
    assert_eq!(
        vm.eval_source(source).unwrap().display().unwrap(),
        "(nil nil (args-out-of-range 0 9223372036854775808))"
    );
}
