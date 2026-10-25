use romacs_vm::Vm;

fn probes() -> Vec<String> {
    let mut cases = Vec::new();
    for setup in [
        "",
        "(add-text-properties 0 1 '(p x) s) (add-text-properties 1 2 '(p x) s)",
        "(add-text-properties 1 2 '(p nil) s)",
    ] {
        for position in [0, 1, 2, 3] {
            for limit in ["nil", "2", "99", "-4"] {
                cases.push(format!(r#"(let ((s (copy-sequence "αβγ"))) {setup} (next-property-change {position} s {limit}))"#));
                cases.push(format!(r#"(let ((s (copy-sequence "αβγ"))) {setup} (next-single-char-property-change {position} 'p s {limit}))"#));
            }
        }
    }
    cases.extend([
        r#"(let ((s (unibyte-string 97 255 98))) (add-text-properties 1 2 '(p x) s) (list (next-property-change 0 s) (next-property-change 1 s) (next-single-char-property-change 0 'p s)))"#,
        r#"(let ((s (string 4194281 955 2097152))) (add-text-properties 1 2 '(p x) s) (list (next-property-change 0 s) (next-property-change 1 s) (next-single-char-property-change 0 'p s)))"#,
        r#"(with-temp-buffer (insert "αβγδ") (add-text-properties 1 5 '(p a)) (let ((o (make-overlay 2 4))) (overlay-put o 'p 'a) (list (next-single-char-property-change 1 'p) (progn (overlay-put o 'p 'b) (next-single-char-property-change 1 'p)) (next-single-char-property-change 2 'p) (next-single-char-property-change 4 'p) (next-property-change 1))))"#,
        r#"(with-temp-buffer (insert "αβγδ") (add-text-properties 1 5 '(p a)) (let ((o (make-overlay 2 4)) (top (make-overlay 3 4))) (overlay-put o 'p 'b) (overlay-put o 'priority 1) (overlay-put top 'p 'a) (overlay-put top 'priority 9) (list (next-single-char-property-change 1 'p) (next-single-char-property-change 2 'p) (next-single-char-property-change 3 'p))))"#,
        r#"(with-temp-buffer (insert "αβγδ") (add-text-properties 1 5 '(p a)) (let ((o (make-overlay 2 4))) (overlay-put o 'q 'other) (overlay-put o 'priority 9) (list (next-single-char-property-change 1 'p) (next-property-change 1))))"#,
        r#"(with-temp-buffer (insert "αβγδ") (put 'property-query-category 'p 'a) (add-text-properties 1 3 '(category property-query-category)) (add-text-properties 3 5 '(p a)) (list (next-property-change 1) (next-single-char-property-change 1 'p)))"#,
        r#"(with-temp-buffer (insert "αβγδ") (let ((v (vector 1)) (w (vector 1)) (o (make-overlay 2 4))) (add-text-properties 1 5 (list 'p v)) (overlay-put o 'p v) (list (next-single-char-property-change 1 'p) (progn (overlay-put o 'p w) (next-single-char-property-change 1 'p)))))"#,
        r#"(with-temp-buffer (insert "αβγδ") (add-text-properties 1 3 '(p a)) (narrow-to-region 2 4) (list (condition-case e (next-property-change 1) (error e)) (condition-case e (next-single-char-property-change 1 'p) (error e)) (next-property-change 2) (next-property-change 4) (next-single-char-property-change 4 'p) (next-single-char-property-change 2 'p nil 99) (next-single-char-property-change 2 'p nil 0) (condition-case e (next-single-char-property-change 9 'p) (error e))))"#,
        r#"(let ((origin (current-buffer)) (target (get-buffer-create "property-query-target"))) (with-current-buffer target (insert "αβγ") (add-text-properties 2 3 '(p a))) (list (next-property-change 1 target) (next-single-char-property-change 1 'p target) (eq origin (current-buffer)) (with-current-buffer target (point))))"#,
        r#"(with-temp-buffer (insert "αβγ") (add-text-properties 2 3 '(p a)) (list (next-property-change (copy-marker 1) nil (copy-marker 3)) (next-single-char-property-change (copy-marker 1) 'p nil (copy-marker 3)) (point)))"#,
        r#"(with-temp-buffer (insert "αβγ") (let ((s (copy-sequence "αβγ"))) (add-text-properties 1 2 '(p a) s) (list (next-property-change 0 s (copy-marker 3)) (next-single-char-property-change 0 'p s (copy-marker 3)))))"#,
        r#"(list (condition-case e (next-property-change 'bad "α" 2.5) (error e)) (condition-case e (next-single-char-property-change 'bad 'p nil 2.5) (error e)) (condition-case e (next-single-char-property-change 'bad 'p "α" 2.5) (error e)))"#,
        r#"(list (condition-case e (next-property-change 0 15) (error e)) (condition-case e (next-single-char-property-change 0 'p 15) (error e)) (condition-case e (next-property-change 0 "α" (make-marker)) (error e)))"#,
        r#"(list (next-property-change 0 "α" #x8000000000000000) (next-single-char-property-change 0 'p "α" #x8000000000000000) (with-temp-buffer (insert "α") (next-single-char-property-change 1 'p nil #x8000000000000000)))"#,
        r#"(list (condition-case e (next-property-change #x8000000000000000 "α") (error e)) (condition-case e (with-temp-buffer (next-single-char-property-change #x8000000000000000 'p)) (error e)))"#,
    ].into_iter().map(str::to_owned));
    cases
}

#[test]
fn property_boundaries_drive_pristine_printer_loops() {
    let mut vm = Vm::new();
    let source = r#"(let ((s (copy-sequence "αβγ"))) (add-text-properties 0 1 '(p x) s) (add-text-properties 1 2 '(p x) s) (list (next-property-change 0 s) (next-property-change 2 s) (next-single-char-property-change 0 'p s) (next-single-char-property-change 0 'q s) (next-property-change 3 s 99)))"#;
    assert_eq!(
        vm.eval_source(source).unwrap().display().unwrap(),
        "(2 nil 2 3 99)"
    );
}

#[test]
fn queries_match_live_gnu_and_preserve_effective_overlay_values() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let cases = probes();
    let count = cases.len();
    let mut failures = Vec::new();
    for expression in cases {
        let source = format!("(condition-case e {expression} (error (list 'signal e)))");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "GNU failed for {expression}: {}",
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
    eprintln!("GNU property query comparisons matched: {count}");
}
