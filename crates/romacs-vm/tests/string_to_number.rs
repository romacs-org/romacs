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
        ("(string-to-number \"2305843009213693952\")", "2305843009213693952"),
        ("(string-to-number \" -99999999999999999999x\")", "-99999999999999999999"),
        ("(string-to-number \"ffffffffffffffffffffff\" 16)", "309485009821345068724781055"),
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

/// An integer past the fixnum range is a bignum in any base, as GNU's
/// make_bignum_str makes it.
#[test]
fn string_to_number_bignums_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let source = r#"(mapcar (lambda (c) (let ((n (apply #'string-to-number c))) (list n (bignump n) (fixnump n) (= n (car (read-from-string (format "%d" n))))))) '(("2305843009213693952") ("-2305843009213693953") ("2305843009213693951") ("-2305843009213693952") ("18446744073709551616") ("999999999999999999999999999999999999999") (" +99999999999999999999x") ("\t-99999999999999999999.") ("99999999999999999999e") ("99999999999999999999e5") ("ffffffffffffffffffffff" 16) ("-1111111111111111111111111111111111111111111111111111111111111111111" 2) ("zzzz" 16) ("fffffffffffffffff" 16) ("7777777777777777777777" 8) ("10000000000000000000000000" 3)))"#;
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 (condition-case e {source} (error (list 'signal e))))")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_collection_floor(1);
    let actual = vm.eval_source(&format!("(condition-case e {source} (error (list 'signal e)))")).and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok(expected.as_str()));
}
