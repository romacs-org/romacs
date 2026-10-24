use romacs_vm::Vm;

fn check(source: &str, expected: &str, name: &str) {
    let mut vm = Vm::new();
    let actual = vm.eval_source(source).and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok(expected), "{name}: {source}");
}

#[test]
fn bolp_accessible_beginning_and_character_positions() {
    check("(with-temp-buffer (insert \"é🙂a\\nЖb\") (list (progn (goto-char 1) (bolp)) (progn (goto-char 3) (bolp)) (progn (goto-char 5) (bolp)) (progn (narrow-to-region 3 6) (goto-char 3) (bolp))))", "(t nil t t)", "BOLP_ACCESSIBLE_CHARACTER_BOUNDARY");
}

#[test]
fn eolp_accessible_end_and_character_positions() {
    check("(with-temp-buffer (insert \"é🙂a\\nЖb\") (list (progn (goto-char 4) (eolp)) (progn (goto-char 3) (eolp)) (progn (goto-char 7) (eolp)) (progn (narrow-to-region 2 3) (goto-char 3) (eolp))))", "(t nil t t)", "EOLP_ACCESSIBLE_CHARACTER_BOUNDARY");
}

#[test]
fn bobp_uses_accessible_not_absolute_beginning() {
    check("(with-temp-buffer (insert \"é🙂abc\") (list (progn (goto-char 1) (bobp)) (progn (goto-char 2) (bobp)) (progn (narrow-to-region 3 5) (goto-char 3) (bobp)) (progn (goto-char 4) (bobp))))", "(t nil t nil)", "BOBP_ACCESSIBLE_BEGINNING");
}

#[test]
fn eobp_uses_accessible_not_absolute_end() {
    check("(with-temp-buffer (insert \"é🙂abc\") (list (progn (goto-char 6) (eobp)) (progn (goto-char 2) (eobp)) (progn (narrow-to-region 2 4) (goto-char 4) (eobp)) (progn (goto-char 3) (eobp))))", "(t nil t nil)", "EOBP_ACCESSIBLE_END");
}

#[test]
fn empty_and_empty_restriction() {
    check("(with-temp-buffer (list (bolp) (eolp) (bobp) (eobp)))", "(t t t t)", "EMPTY_BUFFER");
    check("(with-temp-buffer (insert \"abc\") (narrow-to-region 2 2) (list (bolp) (eolp) (bobp) (eobp)))", "(t t t t)", "EMPTY_RESTRICTION");
}

#[test]
fn unibyte_predicates() {
    check("(with-temp-buffer (set-buffer-multibyte nil) (insert (unibyte-string 255 10 254)) (goto-char 3) (list (bolp) (eolp) (bobp) (eobp) (progn (narrow-to-region 2 3) (eolp)) (eobp)))", "(t nil nil nil t t)", "UNIBYTE_BOUNDARIES");
}

#[test]
fn callable_and_arity() {
    for name in ["bolp", "eolp", "bobp", "eobp"] {
        check(&format!("(with-temp-buffer (list (fboundp '{name}) (functionp #'{name}) (funcall #'{name})))"), "(t t t)", "CALLABLE");
        check(&format!("(condition-case c (apply #'{name} '(1)) (wrong-number-of-arguments (car c)))"), "wrong-number-of-arguments", "ZERO_ARITY");
    }
}
