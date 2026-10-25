use romacs_vm::Vm;

#[test]
fn replace_match_replays_pinned_gnu_values_offline() {
    let evidence: serde_json::Value = serde_json::from_str(include_str!("fixtures/replace_match_gnu.json")).unwrap();
    let cases = evidence["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 19, "pinned GNU evidence must not disappear");
    assert_eq!(cases.iter().filter(|case| case["supported"] == true).count(), 17);
    for case in cases {
        if case["supported"] == true {
            check(case["source"].as_str().unwrap(), case["gnu"].as_str().unwrap());
        }
    }
}

fn check(source: &str, expected: &str) {
    let mut vm = Vm::new();
    let actual = vm.eval_source(source).and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok(expected), "REPLACE_MATCH: {source}");
}

#[test]
fn replace_match_character_ranges_and_subexpressions() {
    check("(list (fboundp 'replace-match) (functionp #'replace-match))", "(t t)");
    check(r#"(progn (string-match "-mode" "Auto-mode") (replace-match "" nil nil "Auto-mode"))"#, r#""Auto""#);
    check(r#"(progn (string-match "é\\(λ\\)" "aéλz") (list (replace-match "X" t t "aéλz") (replace-match "Y" t t "aéλz" 1) (match-data t)))"#, r#"("aXz" "aéYz" (1 3 2 3))"#);
    check(r#"(progn (string-match "b" "abc") (funcall #'replace-match "" t t "abc"))"#, r#""ac""#);
}

#[test]
fn replace_match_backreferences_and_escapes() {
    check(r#"(progn (string-match "\\(a\\)b" "zab!") (replace-match "<\\&-\\1-\\9-\\\\-\\?>" t nil "zab!"))"#, r#""z<ab-a--\\-\\?>!""#);
    check(r#"(progn (string-match "\\(a\\)b" "zab!") (replace-match "\\&" t nil "zab!" 1))"#, r#""zab!""#);
    check(r#"(progn (string-match "a" "a") (condition-case e (replace-match "\\x" t nil "a") (error e)))"#, r#"(error "Invalid use of `\\' in replacement text")"#);
}

#[test]
fn replace_match_preserves_source_and_replacement_properties() {
    check(r#"(let ((s (copy-sequence "abc")) (r (copy-sequence "XY"))) (put-text-property 0 3 'face 'bold s) (put-text-property 0 2 'face 'italic r) (string-match "b" s) (let ((out (replace-match r t t s))) (list (substring-no-properties out) (get-text-property 0 'face out) (get-text-property 1 'face out) (get-text-property 3 'face out) (substring-no-properties s))))"#, r#"("aXYc" bold italic bold "abc")"#);
}

#[test]
fn replace_match_case_inference_and_initials() {
    for (text, replacement, expected) in [
        ("XX", "mIxEd woRD", "\"MIXED WORD\""),
        ("Xx", "mIxEd woRD", "\"MIxEd WoRD\""),
        ("X", "mIxEd woRD", "\"MIXED WORD\""),
        ("xX", "mIxEd woRD", "\"mIxEd woRD\""),
        ("XX", "ßéﬃ", "\"SSÉFFI\""),
        ("Xx", "ßéﬃ", "\"Sséﬃ\""),
        ("ÉΛ", "élan", "\"ÉLAN\""),
        ("Élan", "mIxEd", "\"MIxEd\""),
    ] {
        check(&format!("(progn (string-match {text:?} {text:?}) (replace-match {replacement:?} nil t {text:?}))"), expected);
    }
    check(r#"(progn (string-match "\\(a\\) XX" "a XX") (replace-match "\\1 mIxEd" nil nil "a XX"))"#, r#""a mIxEd""#);
    check(r#"(progn (string-match "a \\(XX\\)" "a XX") (replace-match "\\& mIxEd" nil nil "a XX" 1))"#, r#""a XX MIXED""#);
    check(r#"(list (upcase-initials "mIxEd woRD") (upcase-initials "ßé ﬃélan") (upcase-initials ?a))"#, r#"("MIxEd WoRD" "Ssé Ffiélan" 65)"#);
}

#[test]
fn replace_match_inference_obeys_case_tables() {
    check(r#"(progn (set-case-table (make-char-table 'case-table)) (string-match "XX" "XX") (replace-match "mixed" nil t "XX"))"#, r#""mixed""#);
    check(r#"(let ((table (copy-sequence (standard-case-table)))) (aset table ?X nil) (set-case-table table) (string-match "XX" "XX") (replace-match "mixed" nil t "XX"))"#, r#""mixed""#);
    check(r#"(let ((table (copy-sequence (standard-case-table)))) (aset table ?1 ?a) (set-case-table table) (string-match "11" "11") (replace-match "mixed" nil t "11"))"#, r#""MIXED""#);
    check(r#"(let* ((d (copy-sequence (standard-case-table))) (u (copy-sequence (char-table-extra-slot d 0)))) (aset u ?1 256) (set-char-table-extra-slot d 0 u) (set-case-table d) (string-match "Ab" "Ab") (list (aref (replace-match (unibyte-string ?1) nil t "Ab") 0) (aref (upcase (unibyte-string ?1)) 0)))"#, "(49 49)");
}

#[test]
fn replace_match_checks_types_ranges_and_arity() {
    check(r#"(progn (string-match "a" "a") (condition-case e (replace-match "x" t t "a" 30) (error (list (car e) (car (cdr e)) (car (cdr (cdr e)))))))"#, "(args-out-of-range 30 0)");
    check(r#"(progn (set-match-data '(0 1 nil nil)) (condition-case e (replace-match "x" t t "a" 1) (error e)))"#, r#"(error "replace-match subexpression does not exist" 1)"#);
    check(r#"(progn (string-match "a" "a") (list (multibyte-string-p (replace-match (string-to-multibyte "\\&") t nil (string-make-unibyte "a"))) (multibyte-string-p (replace-match (string-to-multibyte "b") t t "a"))))"#, "(nil t)");
    for (source, expected) in [
        (r#"(condition-case e (replace-match 3) (error e))"#, "(wrong-type-argument stringp 3)"),
        (r#"(condition-case e (replace-match "x" t t 3) (error e))"#, "(wrong-type-argument stringp 3)"),
        (r#"(progn (string-match "a" "a") (set-match-data nil) (condition-case e (replace-match "x" t t "a") (error e)))"#, r#"(error "replace-match subexpression does not exist" nil)"#),
        (r#"(progn (set-match-data '(0 1)) (condition-case e (replace-match "x" t t "a" 'bad) (error e)))"#, "(wrong-type-argument integerp bad)"),
        (r#"(progn (set-match-data '(0 4)) (condition-case e (replace-match "x" t t "a") (error e)))"#, "(args-out-of-range 0 4)"),
        (r#"(condition-case e (replace-match) (error (list (car e) (car (cdr (cdr e))))))"#, "(wrong-number-of-arguments 0)"),
        (r#"(condition-case e (replace-match "a" t t "b" nil 1) (error (list (car e) (car (cdr (cdr e))))))"#, "(wrong-number-of-arguments 6)"),
    ] { check(source, expected); }
}

#[test]
fn replace_match_refuses_unimplemented_buffer_replacement() {
    check(r#"(progn (set-match-data '(0 1)) (condition-case e (replace-match "x" t t) (error e)))"#, r#"(error "replace-match: buffer replacement is not implemented")"#);
    check(r#"(progn (set-match-data '(0 1)) (replace-match "x" nil t "a"))"#, r#""x""#);
    check(r#"(progn (set-match-data '(0 1)) (replace-match "x" nil t "A"))"#, r#""X""#);
}
