use romacs_vm::Vm;

fn check(source: &str, expected: &str, assertion: &str) {
    for compiled in [true, false] {
        let mut vm = Vm::new();
        let result = if compiled {
            vm.compile_source(source)
                .and_then(|entry| vm.execute(entry))
        } else {
            vm.eval_source(source)
        };
        let actual = result.and_then(|v| v.display());
        assert_eq!(
            actual,
            Ok(expected.into()),
            "{assertion}: compiled={compiled}: {source}"
        );
    }
}

#[test]
fn fixnum_overflow_promotes_in_direct_compiled_and_indirect_calls() {
    for (operator, arguments, expected) in [
        ("1+", "most-positive-fixnum", "2305843009213693952"),
        ("1-", "most-negative-fixnum", "-2305843009213693953"),
        ("+", "most-positive-fixnum 1", "2305843009213693952"),
        ("-", "most-negative-fixnum 1", "-2305843009213693953"),
        ("-", "most-negative-fixnum", "2305843009213693952"),
        ("*", "most-positive-fixnum 3", "6917529027641081853"),
        ("/", "most-negative-fixnum -1", "2305843009213693952"),
        ("abs", "most-negative-fixnum", "2305843009213693952"),
    ] {
        for function in [operator.to_owned(), format!("funcall '{operator}")] {
            check(
                &format!("({function} {arguments})"),
                expected,
                "INTEGER_PROMOTION",
            );
        }
    }
    check(
        "(1+ (logb (1+ most-positive-fixnum)))",
        "62",
        "CL_PRELOADED_FORM67",
    );
    check(
        "(defun promoted-inc (x) (1+ x)) (promoted-inc most-positive-fixnum)",
        "2305843009213693952",
        "INLINE_DEOPT_PROMOTION",
    );
}

#[test]
fn bignum_arguments_normalize_after_every_basic_integer_operator() {
    let big = "(car (read-from-string \"#x2000000000000000\"))";
    for (expression, expected) in [
        ("(1- b)", "2305843009213693951"),
        ("(1+ (- b))", "-2305843009213693951"),
        ("(+ b (- b) 9)", "9"),
        ("(+ b b b (- b) (- b) (- b) 7)", "7"),
        ("(- b b 3)", "-3"),
        ("(* b b 0)", "0"),
        ("(/ (* b b) b b)", "1"),
        ("(/ (- (+ b 1)) 2)", "-1152921504606846976"),
        ("(/ b)", "0"),
        ("(+)", "0"),
        ("(-)", "0"),
        ("(*)", "1"),
    ] {
        let source = format!("(let ((b {big})) (let ((x {expression})) (list x (fixnump x))))");
        check(&source, &format!("({expected} t)"), "INTEGER_NORMALIZATION");
    }
    check(
        "(let ((p most-positive-fixnum)) (let ((x (+ p p p p p (- p) (- p) (- p) (- p) (- p) 7))) (list x (fixnump x))))",
        "(7 t)",
        "INTEGER_NARY_FIXNUM_OVERFLOW",
    );
    check(
        &format!("(let ((b {big})) (eq b (abs b)))"),
        "t",
        "ABS_NONNEGATIVE_IDENTITY",
    );
    for expression in ["(+ b)", "(- (- b))", "(* b)", "(abs (- b))", "(1- (1+ b))"] {
        check(
            &format!("(let ((b {big})) (list (= b {expression}) (bignump {expression})))"),
            "(t t)",
            "BIGNUM_ARGUMENT",
        );
    }
}

#[test]
fn promoted_comparisons_are_exact_for_all_sibling_consumers() {
    let big = "(car (read-from-string \"#x2000000000000001\"))";
    for (expression, expected) in [
        ("(= b 2.305843009213694e18)", "nil"),
        ("(< 2.305843009213694e18 b)", "t"),
        ("(> b 2.305843009213694e18)", "t"),
        ("(<= b 2.305843009213694e18)", "nil"),
        ("(>= 2.305843009213694e18 b)", "nil"),
        ("(/= b 2.305843009213694e18)", "t"),
        ("(< (- b) -2.305843009213694e18)", "t"),
        ("(= (1- b) 2.305843009213694e18)", "t"),
        ("(< b 1.0e+INF)", "t"),
        ("(> b -1.0e+INF)", "t"),
        ("(= b 0.0e+NaN)", "nil"),
        ("(< b 0.0e+NaN)", "nil"),
        ("(/= b 0.0e+NaN)", "t"),
        ("(eq (max b 2.305843009213694e18) b)", "t"),
        ("(eq (min b 2.305843009213694e18) b)", "nil"),
        ("(< 2 1 'bad)", "nil"),
        ("(= 1 2 'bad)", "nil"),
        ("(< 'bad)", "t"),
        ("(= 'bad)", "t"),
    ] {
        check(
            &format!("(let ((b {big})) {expression})"),
            expected,
            "EXACT_PROMOTED_COMPARISON",
        );
    }
    let huge = format!("(car (read-from-string \"#x1{}\"))", "0".repeat(50));
    check(
        &format!(
            "(let ((b {huge})) (list (= b 1.6069380442589903e60) (> (1+ b) 1.6069380442589903e60) (< 1.6069380442589903e60 (1+ b)) (= (1+ b) 1.6069380442589903e60) (< b 1.0e+INF)))"
        ),
        "(t t t nil t)",
        "HUGE_INTEGER_COMPARISON",
    );
}

#[test]
fn arithmetic_preserves_float_transition_and_integer_division() {
    for (source, expected) in [
        ("(/ -7 2)", "-3"),
        ("(/ 7 -2)", "-3"),
        ("(/ -7 -2)", "3"),
        ("(/ 5 2 2.0)", "1.25"),
        ("(+ 9007199254740992 1 -9007199254740992 0.0)", "1.0"),
        ("(+ -0.0)", "-0.0"),
        ("(+ (1+ most-positive-fixnum) 0.5)", "2.305843009213694e+18"),
    ] {
        check(source, expected, "ARITHMETIC_FLOAT_TRANSITION");
    }
}

#[test]
fn arithmetic_conditions_and_markers_keep_gnu_contracts() {
    for (source, expected) in [
        ("(/ 1 0)", "arith-error"),
        ("(/ 0)", "arith-error"),
        ("(/ (1+ most-positive-fixnum) 0)", "arith-error"),
        ("(/)", "wrong-number-of-arguments"),
        ("(1+)", "wrong-number-of-arguments"),
        ("(1- 1 2)", "wrong-number-of-arguments"),
        ("(abs)", "wrong-number-of-arguments"),
    ] {
        check(
            &format!("(condition-case e {source} (error (car e)))"),
            expected,
            "ARITHMETIC_CONDITION",
        );
    }
    for operator in ["+", "-", "*", "/", "1+", "1-", "abs", "<", "="] {
        let predicate = if operator == "abs" {
            "numberp"
        } else {
            "number-or-marker-p"
        };
        let args = if ["<", "="].contains(&operator) {
            "1 'bad"
        } else {
            "'bad"
        };
        check(
            &format!("(condition-case e ({operator} {args}) (error e))"),
            &format!("(wrong-type-argument {predicate} bad)"),
            "ARITHMETIC_TYPE",
        );
    }
    check(
        "(let ((b (1+ most-positive-fixnum))) (condition-case e (+ b 'bad) (error e)))",
        "(wrong-type-argument number-or-marker-p bad)",
        "BIGNUM_NOT_BLAMED",
    );
    check(
        "(condition-case e (1+ (make-marker)) (error e))",
        "(error \"Marker does not point anywhere\")",
        "ARITHMETIC_DETACHED_MARKER",
    );
    check(
        "(with-temp-buffer (insert \"abcd\") (let ((m (copy-marker 3))) (list (+ m 1) (- m 1) (* m 2) (/ m 2) (1+ m) (1- m) (= m 3) (< m 4))))",
        "(4 2 6 1 4 2 t t)",
        "ARITHMETIC_MARKER",
    );
    check(
        "(let ((m (make-marker))) (condition-case e (abs m) (error (equal e (list 'wrong-type-argument 'numberp m)))))",
        "t",
        "ABS_REJECTS_MARKER",
    );
}

#[test]
fn promoted_integers_survive_moving_gc_and_number_string_roundtrip() {
    let mut vm = Vm::new();
    vm.eval_source("(list 1.0 [2.0])").unwrap();
    let root = vm
        .eval_source("(list (1+ most-positive-fixnum) (1- most-negative-fixnum))")
        .unwrap()
        .root();
    vm.collect_garbage();
    assert_eq!(
        root.get(&vm).unwrap().display().unwrap(),
        "(2305843009213693952 -2305843009213693953)",
        "BIGNUM_GC_ROOT"
    );
    vm.set_collection_floor(1);
    let collections = vm.heap_stats().collections;
    let source = "(defun retained-big (x) (1+ x)) (let ((b (retained-big most-positive-fixnum)) (i 0)) (while (< i 50000) (setq b (+ b 1) i (1+ i))) (list (equal (number-to-string b) \"2305843009213743952\") (= (- b 50001) (string-to-number (number-to-string (- b 50001)))) (fixnump (- b 50001))))";
    let entry = vm.compile_source(source).unwrap();
    let actual = vm.execute(entry).unwrap().display().unwrap();
    assert_eq!(actual, "(t t t)", "BIGNUM_GC_ROUNDTRIP");
    assert!(
        vm.heap_stats().collections > collections,
        "BIGNUM_GC_EXECUTED"
    );
    let decimal = root.get(&vm).unwrap().display().unwrap();
    assert_eq!(
        decimal, "(2305843009213693952 -2305843009213693953)",
        "BIGNUM_GC_RETAINED"
    );
    let decimal = String::from_utf8(
        vm.eval_source("(number-to-string (1+ most-positive-fixnum))")
            .unwrap()
            .bytes()
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let integer = num_bigint::BigInt::parse_bytes(decimal.as_bytes(), 10).unwrap();
    let source = format!("#x{}", integer.to_str_radix(16));
    let reread = vm.read_source(&source).unwrap().unwrap().0;
    assert_eq!(
        reread.display().unwrap(),
        decimal,
        "BIGNUM_NUMBER_STRING_RADIX_ROUNDTRIP"
    );
}
