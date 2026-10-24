use romacs_vm::Vm;

fn check(source: &str, expected: &str, contract: &str) {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let actual = vm.eval_source(source).and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok(expected), "{contract}: {source}");
}

#[test]
fn atan_two_argument_quadrant_and_negative_zero() {
    check(
        "(list (atan 0) (atan 1 0) (atan -0.0 1) (atan 1 nil))",
        "(0.0 1.5707963267948966 -0.0 0.7853981633974483)",
        "atan two-argument quadrant and negative zero",
    );
}

#[test]
fn exp_zero_underflow_and_overflow() {
    check(
        "(list (exp 0) (exp -1000) (exp 1000))",
        "(1.0 0.0 1.0e+INF)",
        "exp zero underflow overflow",
    );
}

#[test]
fn number_to_string_decimal_and_unibyte() {
    check(
        "(list (string-equal (number-to-string -42) \"-42\") (string-equal (number-to-string -0.0) \"-0.0\") (string-equal (number-to-string 1.25) \"1.25\") (multibyte-string-p (number-to-string 3)))",
        "(t t t nil)",
        "number-to-string decimal unibyte",
    );
    check(
        "(list (string-equal (number-to-string 5e-324) \"5e-324\") (string-equal (number-to-string (string-to-number \"7.0e+NaN\")) \"7.0e+NaN\"))",
        "(t t)",
        "number-to-string subnormal and NaN payload",
    );
    check(
        "(progn (setq float-output-format \"%.2f\") (string-equal (number-to-string 1.234) \"1.23\"))",
        "t",
        "number-to-string float-output-format",
    );
}
