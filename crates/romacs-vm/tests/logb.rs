use romacs_vm::Vm;

#[test]
fn logb_matches_pinned_gnu_floatfns_values() {
    let mut vm = Vm::new();
    for (argument, expected) in [
        ("0", "-1.0e+INF"),
        ("0.0", "-1.0e+INF"),
        ("-0.0", "-1.0e+INF"),
        ("1", "0"),
        ("-1", "0"),
        ("2", "1"),
        ("-2", "1"),
        ("3", "1"),
        ("7", "2"),
        ("8", "3"),
        ("9", "3"),
        ("9007199254740991", "52"),
        ("9007199254740992", "53"),
        (
            "(car (read-from-string \"#xfffffffffffffffffffffffff\"))",
            "99",
        ),
        (
            "(car (read-from-string \"#x10000000000000000000000000\"))",
            "100",
        ),
        (
            "(car (read-from-string \"#x-10000000000000000000000000\"))",
            "100",
        ),
        ("0.5", "-1"),
        ("-0.75", "-1"),
        ("1.9999999999999998", "0"),
        ("2.0", "1"),
        ("5e-324", "-1074"),
        ("-5e-324", "-1074"),
        ("2.2250738585072014e-308", "-1022"),
        ("1.7976931348623157e308", "1023"),
        ("1.0e+INF", "1.0e+INF"),
        ("-1.0e+INF", "1.0e+INF"),
        ("0.0e+NaN", "0.0e+NaN"),
        ("-0.0e+NaN", "-0.0e+NaN"),
    ] {
        for function in ["logb", "funcall 'logb"] {
            let source = format!("({function} {argument})");
            let actual = vm.eval_source(&source).and_then(|v| v.display());
            assert_eq!(actual, Ok(expected.into()), "LOGB_GNU_VALUE: {source}");
        }
    }
}

#[test]
fn logb_is_callable_and_preserves_nonfinite_identity() {
    let mut vm = Vm::new();
    for source in [
        "(list (fboundp 'logb) (functionp (symbol-function 'logb)))",
        "(list (let ((x 1.0e+INF)) (eq x (logb x))) (let ((x -0.0e+NaN)) (eq x (funcall 'logb x))))",
    ] {
        assert_eq!(
            vm.eval_source(source).unwrap().display().unwrap(),
            "(t t)",
            "LOGB_CALLABLE_IDENTITY: {source}"
        );
    }
}

#[test]
fn logb_rejects_nonnumbers_and_wrong_arity() {
    let mut vm = Vm::new();
    for argument in ["nil", "'bad", "\"x\"", "[1]", "'(1 . 2)", "(make-marker)"] {
        for function in ["logb", "funcall 'logb"] {
            let source = format!(
                "(let ((x {argument})) (condition-case e ({function} x) (wrong-type-argument (equal e (list 'wrong-type-argument 'numberp x)))))"
            );
            assert_eq!(
                vm.eval_source(&source).unwrap().display().unwrap(),
                "t",
                "LOGB_NUMBER_TYPE: {source}"
            );
        }
    }
    for source in [
        "(logb)",
        "(logb 1 2)",
        "(funcall 'logb)",
        "(funcall 'logb 1 2)",
    ] {
        let source = format!("(condition-case e {source} (error (car e)))");
        assert_eq!(
            vm.eval_source(&source).unwrap().display().unwrap(),
            "wrong-number-of-arguments",
            "LOGB_ARITY: {source}"
        );
    }
}
