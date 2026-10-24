use romacs_vm::Vm;

#[test]
fn native_overlay_arrow_variable_preserves_identity_and_dynamic_binding() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    for (source, expected) in [
        ("overlay-arrow-variable-list", "(overlay-arrow-position)"),
        ("(progn (setq saved-arrows overlay-arrow-variable-list) (setcar overlay-arrow-variable-list 'changed) (eq saved-arrows overlay-arrow-variable-list))", "t"),
        ("(let ((overlay-arrow-variable-list '(local))) overlay-arrow-variable-list)", "(local)"),
        ("overlay-arrow-variable-list", "(changed)"),
        ("frame-internal-parameters", "(undeleted cloned-from frame-id name parent-id window-id)"),
    ] {
        assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), expected, "{source}");
    }
}

#[test]
fn replace_match_nochange_uses_matched_text_and_syntax() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(progn (string-match \"-Minor\" \"Next-Error-Minor\") (replace-match \" minor\" nil t \"Next-Error-Minor\"))", "\"Next-Error minor\""),
        ("(progn (string-match \"abc\" \"abc\") (replace-match \"XYZ\" nil t \"abc\"))", "\"XYZ\""),
        ("(progn (string-match \"[0-9]+\" \"123\") (replace-match \"Mixed\" nil t \"123\"))", "\"Mixed\""),
        ("(progn (string-match \"\" \"abc\") (replace-match \"Mixed\" nil t \"abc\"))", "\"Mixedabc\""),
        ("(progn (set-syntax-table (copy-syntax-table)) (modify-syntax-entry 65 \".\") (string-match \"Abc\" \"Abc\") (replace-match \"mixed\" nil t \"Abc\"))", "\"mixed\""),
    ] {
        assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), expected, "{source}");
    }
    vm.eval_source("(set-syntax-table (standard-syntax-table))").unwrap();
    for text in ["ABC", "Abc", "X"] {
        let source = format!("(progn (string-match {text:?} {text:?}) (condition-case e (replace-match \"mixed\" nil t {text:?}) (error e)))");
        assert_eq!(vm.eval_source(&source).unwrap().display().unwrap(), "(error \"replace-match: case-table conversion is not implemented\")");
    }
    vm.eval_source("(set-syntax-table (copy-syntax-table)) (modify-syntax-entry 10 \"w\")").unwrap();
    let source = "(progn (string-match \"a\" \"a\") (condition-case e (replace-match \"x\" nil t \"a\") (error e)))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(error \"replace-match: case-table conversion is not implemented\")");
}
