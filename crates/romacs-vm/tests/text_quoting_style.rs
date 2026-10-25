use romacs_vm::Vm;

#[test]
fn explicit_style_capability_and_display_table_match_gnu() {
    for (source, expected) in [
        ("(let ((internal--text-quoting-flag t) (text-quoting-style nil)) (text-quoting-style))", "curve"),
        ("(let ((internal--text-quoting-flag nil) (text-quoting-style nil)) (text-quoting-style))", "grave"),
        ("(let ((internal--text-quoting-flag nil) (text-quoting-style 'straight)) (text-quoting-style))", "straight"),
        ("(let ((text-quoting-style 'grave)) (text-quoting-style))", "grave"),
        ("(let ((text-quoting-style 'invalid)) (text-quoting-style))", "curve"),
        ("(progn (put 'display-table 'char-table-extra-slots 18) (let ((text-quoting-style nil) (internal--text-quoting-flag t) (standard-display-table (make-char-table 'display-table))) (aset standard-display-table 8216 [96]) (text-quoting-style)))", "grave"),
    ] {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), expected, "{source}");
    }
}
