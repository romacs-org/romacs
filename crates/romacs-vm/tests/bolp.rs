use romacs_vm::Vm;

#[test]
fn bolp_observes_newlines_empty_buffer_and_narrowing() {
    for (source, expected) in [
        ("(bolp)", "t"),
        ("(progn (insert \"αx\\ny\") (goto-char 2) (bolp))", "nil"),
        ("(progn (insert \"αx\\ny\") (goto-char 4) (bolp))", "t"),
        ("(progn (insert \"αx\\ny\") (goto-char 5) (bolp))", "nil"),
        ("(progn (insert \"abc\") (narrow-to-region 2 3) (goto-char 2) (bolp))", "t"),
        ("(progn (insert \"abc\") (narrow-to-region 2 3) (goto-char 3) (bolp))", "nil"),
        ("(progn (set-buffer-multibyte nil) (insert \"a\\nb\") (goto-char 3) (funcall #'bolp))", "t"),
    ] {
        let mut vm = Vm::new();
        assert_eq!(vm.eval_source(source).and_then(|v| v.display()), Ok(expected.into()), "bolp position contract: {source}");
    }
}
