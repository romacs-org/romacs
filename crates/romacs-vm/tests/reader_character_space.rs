use romacs_vm::Vm;

#[test]
fn character_space_and_tab_end_before_adjacent_forms() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("'(seq (? ?-) (? ?. digit))", "(seq (32 45) (32 46 digit))"),
        ("(list ? 9 ?\t9)", "(32 9 9 9)"),
        ("(list (car (read-from-string \"? x\")) (cdr (read-from-string \"? x\")))", "(32 2)"),
    ] {
        assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), expected, "{source}");
    }
}
