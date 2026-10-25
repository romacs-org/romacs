use romacs_vm::Vm;

#[test]
fn syntax_descriptors_share_flags_matching_and_live_identity() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    for (source, expected) in [
        (r#"(list (string-to-syntax ".") (string-to-syntax "(β 1234bpnc") (string-to-syntax "@ ignored"))"#, "((1) (16711684 . 946) nil)"),
        (r#"(let* ((table (copy-syntax-table)) (entry (string-to-syntax "w"))) (modify-syntax-entry 65 "w" table) (garbage-collect) (list (eq entry (string-to-syntax "w")) (eq entry (aref table 65)) (eq entry (aref (standard-syntax-table) 65))))"#, "(t t t)"),
        (r#"(condition-case e (string-to-syntax 1) (error e))"#, "(wrong-type-argument stringp 1)"),
        (r#"(condition-case e (string-to-syntax "x") (error e))"#, r#"(error "Invalid syntax description letter: x")"#),
        (r#"(list (string-to-syntax "w\0p") (string-to-syntax "w \0p"))"#, "((2) (2))"),
    ] {
        assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), expected, "{source}");
    }
}
