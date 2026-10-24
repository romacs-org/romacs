use romacs_vm::Vm;

#[test]
fn translate_moves_all_character_registers_and_clamps_each_endpoint() {
    for (source, expected) in [
        ("(progn (set-match-data '(1 3 nil nil 4 8)) (list (match-data--translate 2) (match-data t)))", "(nil (3 5 nil nil 6 10))"),
        ("(progn (set-match-data '(1 3 nil nil 4 8)) (match-data--translate -5) (match-data t))", "(0 0 nil nil 0 3)"),
        ("(progn (set-match-data '(2 -1)) (match-data--translate 2) (match-data t))", "(4 1)"),
        ("(progn (string-match \"é\\\\(λ\\\\)\" \"xéλ\") (match-data--translate 3) (match-data t))", "(4 6 5 6)"),
        ("(progn (set-match-data nil) (list (match-data--translate 4) (match-data t)))", "(nil nil)"),
        ("(progn (set-match-data '(3 5)) (funcall #'match-data--translate 0) (match-data t))", "(3 5)"),
    ] {
        let mut vm = Vm::new();
        let actual = vm.eval_source(source).and_then(|v| v.display());
        assert_eq!(actual.as_deref(), Ok(expected), "TRANSLATE_REGISTERS: {source}");
    }
}

#[test]
fn translate_checks_fixnum_without_changing_existing_data() {
    let mut vm = Vm::new();
    let actual = vm.eval_source("(progn (set-match-data '(2 4)) (list (condition-case e (match-data--translate nil) (error e)) (match-data t)))").and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok("((wrong-type-argument fixnump nil) (2 4))"), "TRANSLATE_FIXNUM");
}

#[test]
fn translate_preserves_buffer_provenance_and_saved_markers() {
    let mut vm = Vm::new();
    let actual = vm.eval_source("(progn (erase-buffer) (insert \"aéλz\") (goto-char 2) (looking-at \"éλ\") (let ((saved (match-data))) (match-data--translate 1) (list (match-beginning 0) (match-end 0) (eq (car (cdr (cdr (match-data t)))) (current-buffer)) (marker-position (car saved)))))").and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok("(3 5 t 2)"), "TRANSLATE_BUFFER_PROVENANCE");
}
