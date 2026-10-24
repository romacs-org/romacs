use romacs_vm::Vm;

fn check(name: &str, source: &str, expected: &str) {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let expression = if expected.starts_with('"') {
        format!("(equal {source} {expected})")
    } else {
        source.to_owned()
    };
    let expected = if expected.starts_with('"') {
        "t"
    } else {
        expected
    };
    let actual = match vm.eval_source(&expression) {
        Ok(value) => value.display().unwrap(),
        Err(error) => format!("ERROR: {error}"),
    };
    assert_eq!(actual, expected, "{name}: {source}");
}

#[test]
fn sort_stable_keywords_and_legacy_identity() {
    check(
        "sort_stable_reverse",
        "(sort '((2 . a) (1 . b) (2 . c)) :key #'car :lessp #'< :reverse t)",
        "((2 . a) (2 . c) (1 . b))",
    );
    check(
        "sort_copy_default",
        "(let* ((x (list 3 1 2)) (y (sort x :lessp #'<))) (list x y (eq x y)))",
        "((3 1 2) (1 2 3) nil)",
    );
    check("sort_legacy_identity", "(let* ((x (list 3 1 2)) (tail (cdr x)) (y (sort x #'<))) (list x (eq x y) (eq tail (cdr y))))", "((1 2 3) t t)");
    check(
        "sort_vector_copy",
        "(let* ((x (vector 3 1 2)) (y (sort x :lessp #'<))) (list x y (eq x y)))",
        "([3 1 2] [1 2 3] nil)",
    );
    check("sort_vector_in_place", "(let* ((x (vector 3 1 2)) (y (funcall #'sort x :lessp #'< :in-place t))) (list x (eq x y)))", "([1 2 3] t)");
}

#[test]
fn sort_small_run_and_binary_callback_order() {
    for (source, expected) in [
        ("(let ((trace nil)) (sort '(4 3 2 1) :lessp (lambda (a b) (push (list a b) trace) (< a b))) (nreverse trace))", "((3 4) (2 3) (1 2))"),
        ("(let ((trace nil)) (sort '(1 2 3 4) :lessp (lambda (a b) (push (list a b) trace) (< a b))) (nreverse trace))", "((2 1) (3 2) (4 3))"),
        ("(let ((trace nil)) (sort '(3 1 4 2) :lessp (lambda (a b) (push (list a b) trace) (< a b))) (nreverse trace))", "((1 3) (4 1) (4 3) (2 3) (2 1))"),
        ("(let ((trace nil)) (sort '(2 2 1 2) :lessp (lambda (a b) (push (list a b) trace) (< a b))) (nreverse trace))", "((2 2) (1 2) (1 2) (1 2) (2 2) (2 2))"),
        ("(let ((trace nil)) (sort '((2 . a) (1 . b) (2 . c)) :reverse t :key (lambda (x) (push (cdr x) trace) (car x)) :lessp #'<) (nreverse trace))", "(c b a)"),
    ] {
        check("sort_small_gnu_callbacks", source, expected);
    }
}

#[test]
fn sort_errors_and_boundaries() {
    for (name, source, expected) in [
        ("sort_keyword_identity", "(condition-case nil (progn (sort nil (make-symbol \":reverse\") t) 'accepted) (error 'rejected))", "rejected"),
        ("empty_ignores_predicate", "(sort nil 'invalid)", "nil"),
        ("singleton_ignores_key", "(sort '(1) :key 'invalid)", "(1)"),
        (
            "singleton_copy",
            "(let ((x (list 1))) (eq x (sort x)))",
            "nil",
        ),
        (
            "sort_wrong_type",
            "(condition-case e (sort \"abc\") (error e))",
            "(wrong-type-argument list-or-vector-p \"abc\")",
        ),
        (
            "sort_invalid_keywords",
            "(condition-case e (sort nil :key #'car :reverse) (error e))",
            "(error \"Invalid argument list\")",
        ),
        (
            "sort_unknown_keyword",
            "(condition-case e (sort nil :bad 1) (error e))",
            "(error \"Invalid keyword argument\" :bad)",
        ),
        (
            "sort_dotted",
            "(condition-case e (sort '(1 . bad) #'<) (error e))",
            "(wrong-type-argument listp bad)",
        ),
    ] {
        check(name, source, expected);
    }
}

#[test]
fn mapconcat_mapping_separator_and_sequence_contract() {
    for (name, source, expected) in [
        ("mapconcat_callback", "(mapconcat (lambda (x) (list (+ x 64))) '(1 2 3) \"-\")", "\"A-B-C\""),
        ("mapconcat_vector", "(funcall #'mapconcat #'identity [\"a\" \"b\"] '(955))", "\"aλb\""),
        ("mapconcat_string", "(mapconcat #'list \"λ雪\" [45])", "\"λ-雪\""),
        ("mapconcat_default_separator", "(mapconcat #'identity '(\"a\" nil \"b\"))", "\"ab\""),
        ("mapconcat_empty_skips_checks", "(mapconcat 'invalid nil 'invalid)", "\"\""),
        ("mapconcat_single_skips_separator", "(mapconcat #'identity '(\"a\") 'invalid)", "\"a\""),
        ("mapconcat_all_callbacks_before_concat", "(let ((n 0)) (condition-case e (mapconcat (lambda (x) (setq n (1+ n)) x) '(1 2) \"-\") (error (list n e))))", "(2 (wrong-type-argument sequencep 1))"),
        ("mapconcat_dotted_preflight", "(condition-case e (mapconcat 'invalid '(1 . bad)) (error e))", "(wrong-type-argument listp bad)"),
    ] { check(name, source, expected); }
}
