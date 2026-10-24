use romacs_vm::Vm;

#[test]
fn string_to_number_gnu_prefix_radix_float_and_faults() {
    for (source, expected) in [
        ("(string-to-number \"42tail\")", "42"),
        ("(string-to-number \" \\t-42x\")", "-42"),
        ("(string-to-number \"\\n42\")", "0"),
        ("(string-to-number \"ff\" 16)", "255"),
        ("(string-to-number \"0xff\" 16)", "0"),
        ("(string-to-number \"101.1\" 2)", "5"),
        ("(string-to-number \".5tail\")", "0.5"),
        ("(string-to-number \"1.\")", "1"),
        ("(string-to-number \"1.e2\")", "100.0"),
        ("(string-to-number \"1e+\")", "1"),
        ("(string-to-number \"1.0e+INF\")", "1.0e+INF"),
        ("(string-to-number \"0.0e+NaN\")", "0.0e+NaN"),
        ("(string-to-number \"12\\00034\")", "12"),
        ("(string-to-number \"１２\")", "0"),
        ("(string-to-number \"-2305843009213693952\")", "-2305843009213693952"),
        ("(string-to-number \"2305843009213693951\")", "2305843009213693951"),
        ("(string-to-number \"-0.0\")", "-0.0"),
        ("(string-to-number \"1e9999\")", "1.0e+INF"),
        ("(string-to-number \"1e-9999\")", "0.0"),
        ("(string-to-number \"99999999999999999999999999999999.5\")", "1e+32"),
        ("(string-to-number \"1\" 1)", "(args-out-of-range 1)"),
        ("(string-to-number \"1\" 17)", "(args-out-of-range 17)"),
        ("(string-to-number \"1\" 2.0)", "(wrong-type-argument fixnump 2.0)"),
        ("(string-to-number nil 1)", "(wrong-type-argument stringp nil)"),
        ("(condition-case e (apply 'string-to-number nil) (wrong-number-of-arguments 'arity))", "arity"),
        ("(condition-case e (apply 'string-to-number '(\"1\" 10 nil)) (wrong-number-of-arguments 'arity))", "arity"),
    ] {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let value = vm.eval_source(&format!("(condition-case e {source} (error e))")).expect("condition wrapper");
        assert_eq!(value.display().unwrap(), expected, "{source}");
    }
}

#[test]
fn string_to_number_reports_absent_bignum_substrate() {
    for text in ["2305843009213693952", "-2305843009213693953", "18446744073709551616", "999999999999999999999999999999999999999"] {
        let mut vm = Vm::new();
        let value = vm.eval_source(&format!("(condition-case e (string-to-number \"{text}\") (error e))")).unwrap();
        assert_eq!(value.display().unwrap(), "(error \"string-to-number: bignum results are not supported\")", "{text}");
    }
}
