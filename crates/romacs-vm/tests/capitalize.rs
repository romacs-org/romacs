use romacs_vm::Vm;

#[test]
fn capitalize_native_contract() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(list (capitalize 4294967393) (capitalize 4294967361))", "(65 4294967361)"),
        (r#"(capitalize "a—b «ab» a×b ΑΣ»")"#, r#""A—B «Ab» A×B Ας»""#),
        (r#"(with-temp-buffer (set (make-local-variable 'case-symbols-as-words) t) (capitalize "foo_BAR"))"#, r#""Foo_bar""#),
        ("(with-temp-buffer (local-variable-if-set-p 'case-symbols-as-words))", "t"),
        (r#"(list (fboundp 'capitalize) (functionp #'capitalize) (capitalize "hELLO-world 123ABC foo_BAR don't") (funcall #'capitalize "xY"))"#, r#"(t t "Hello-World 123abc Foo_Bar Don'T" "Xy")"#),
        ("(list (capitalize 97) (capitalize 453) (capitalize 64257) (capitalize 134217825) (capitalize 536871009) (capitalize 9999999999))", "(65 453 64257 134217793 536871009 9999999999)"),
        (r#"(list (capitalize "ßABC ﬃXYZ ǆABC ΟΣ ΟΣΑ İX") (append (capitalize (unibyte-string 255 97 66)) nil) (multibyte-string-p (capitalize (string-to-multibyte "aBC"))))"#, r#"("Ssabc Ffixyz ǅabc Ος Οσα İx" (255 97 98) t)"#),
        (r#"(with-temp-buffer (set-syntax-table (copy-syntax-table)) (modify-syntax-entry 45 "w") (capitalize "a-B_C"))"#, r#""A-b_C""#),
        (r#"(let ((case-symbols-as-words t)) (capitalize "foo_BAR"))"#, r#""Foo_bar""#),
        ("(list (condition-case e (capitalize -1) (error e)) (condition-case e (capitalize nil) (error e)))", "((wrong-type-argument char-or-string-p -1) (wrong-type-argument char-or-string-p nil))"),
        (r#"(let* ((s (propertize (unibyte-string 97 66) 'face 'bold)) (r (capitalize s))) (list (eq s r) (append s nil) (append r nil) (get-text-property 0 'face r) (multibyte-string-p r)))"#, "(nil (97 66) (65 98) bold nil)"),
    ] {
        let actual = vm.eval_source(source).and_then(|v| v.display());
        assert_eq!(actual.as_deref(), Ok(expected), "CAPITALIZE_NATIVE_CONTRACT: {source}");
    }
    // Exact values for class-pinned substrate limitations (VM2-CASE1/CASE2).
    let boolean = vm.eval_source(r#"(with-temp-buffer (setq case-symbols-as-words 7) (list case-symbols-as-words (capitalize "foo_BAR")))"#).and_then(|v| v.display());
    assert_eq!(boolean.as_deref(), Ok(r#"(7 "Foo_bar")"#), "CAPITALIZE_BOOL_FORWARDING_DIVERGENCE");
    let custom = vm.eval_source(r#"(condition-case e (let ((table (copy-case-table (current-case-table)))) (set-char-table-range table 97 120) (set-case-table table) (capitalize "abc")) (error e))"#).and_then(|v| v.display());
    assert_eq!(custom.as_deref(), Ok("(void-function copy-case-table)"), "CAPITALIZE_CUSTOM_CASE_TABLE_DIVERGENCE");
}
