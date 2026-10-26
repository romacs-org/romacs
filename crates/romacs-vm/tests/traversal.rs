use romacs_vm::Vm;

#[test]
fn memq_malformed_list_condition_names_list_on_all_call_paths() {
    let mut vm = Vm::new();
    for call in [
        "(memq 'needle '(a . 7))",
        "(funcall 'memq 'needle '(a . 7))",
        "(apply 'memq '(needle (a . 7)))",
    ] {
        let source = format!("(condition-case e {call} (wrong-type-argument e))");
        let value = vm.eval_source(&source).unwrap();
        assert_eq!(
            value.display().unwrap(),
            "(wrong-type-argument listp (a . 7))",
            "{call}"
        );
    }
}

#[test]
fn plist_get_malformed_and_circular_inputs_return_gnu_nil_on_all_call_paths() {
    let mut vm = Vm::new();
    for plist in ["7", "'(a . 7)", "'(a 1 . 7)", "'(a)"] {
        for call in [
            format!("(plist-get {plist} 'missing)"),
            format!("(funcall 'plist-get {plist} 'missing)"),
            format!("(apply 'plist-get (list {plist} 'missing))"),
        ] {
            let result = vm.eval_source(&call);
            assert!(result.is_ok(), "{call}: {result:?}");
            assert_eq!(result.unwrap().display().unwrap(), "nil", "{call}");
        }
    }
    for call in [
        "(plist-get x 'missing)",
        "(funcall 'plist-get x 'missing)",
        "(apply 'plist-get (list x 'missing))",
    ] {
        let source = format!("(let ((x (list 'a 1))) (setcdr (cdr x) x) {call})");
        let result = vm.eval_source(&source);
        assert!(result.is_ok(), "{source}: {result:?}");
        assert_eq!(result.unwrap().display().unwrap(), "nil");
    }
    for source in [
        "(plist-get '(a 1 . 7) 'a)",
        "(let ((x (list 'a 1))) (setcdr (cdr x) x) (plist-get x 'a))",
    ] {
        assert_eq!(
            vm.eval_source(source).unwrap().as_integer().unwrap(),
            1,
            "{source}"
        );
    }
}

#[test]
fn assq_and_nth_condition_data_match_gnu_on_all_call_paths() {
    let mut vm = Vm::new();
    for (name, args, expected) in [
        (
            "assq",
            "'needle '((a . 1) . 7)",
            "(wrong-type-argument listp ((a . 1) . 7))",
        ),
        ("nth", "'a '(1 2)", "(wrong-type-argument integerp a)"),
        (
            "nth",
            "1e11 '(1 2)",
            "(wrong-type-argument integerp 100000000000.0)",
        ),
        ("nth", "2 '(1 . 7)", "(wrong-type-argument listp (1 . 7))"),
        ("nth", "1 '(1 . 7)", "(wrong-type-argument listp 7)"),
        ("nth", "0 7", "(wrong-type-argument listp 7)"),
    ] {
        for call in [
            format!("({name} {args})"),
            format!("(funcall '{name} {args})"),
            format!("(apply '{name} (list {args}))"),
        ] {
            let source = format!("(condition-case e {call} (wrong-type-argument e))");
            let value = vm.eval_source(&source).unwrap();
            assert_eq!(value.display().unwrap(), expected, "{call}");
        }
    }
}

#[test]
fn nth_large_integer_cycles_return_the_indexed_element() {
    let mut vm = Vm::new();
    for call in [
        "(nth 100000000001 x)",
        "(funcall 'nth 100000000001 x)",
        "(apply 'nth (list 100000000001 x))",
    ] {
        let source = format!("(let ((x (list 1 2 3))) (setcdr (cdr (cdr x)) (cdr x)) {call})");
        assert_eq!(vm.eval_source(&source).unwrap().as_integer().unwrap(), 2);
    }
}

#[test]
fn nth_cycle_period_reduction_handles_prefixes_and_boundaries() {
    let mut vm = Vm::new();
    for prefix in 0..5 {
        for period in 1..6 {
            let len = prefix + period;
            let items = (0..len)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            for index in [0, prefix, len - 1, len, 100000000003i64] {
                let expected = if index < prefix {
                    index
                } else {
                    prefix + (index - prefix) % period
                };
                for call in [
                    format!("(nth {index} x)"),
                    format!("(funcall 'nth {index} x)"),
                    format!("(apply 'nth (list {index} x))"),
                ] {
                    let last = (0..len - 1).fold("x".to_owned(), |tail, _| format!("(cdr {tail})"));
                    let target =
                        (0..prefix).fold("x".to_owned(), |tail, _| format!("(cdr {tail})"));
                    let source =
                        format!("(let ((x (list {items}))) (setcdr {last} {target}) {call})");
                    assert_eq!(
                        vm.eval_source(&source).unwrap().as_integer().unwrap(),
                        expected,
                        "prefix={prefix} period={period} index={index}"
                    );
                }
            }
        }
    }
}

#[test]
fn traversal_handlers_preserve_values_errors_and_redefinition() {
    let mut vm = Vm::new();
    for (source, expected) in [
        (
            "(list (car nil) (cdr nil) (null nil) (not 1) (eq 'x 'x) (eq (cons 1 2) (cons 1 2)))",
            "(nil nil t nil t nil)",
        ),
        (
            "(let ((x (list 1 2 3))) (list (memq 2 x) (memq 9 x) (nth -1 x) (nth 100 x)))",
            "((2 3) nil 1 nil)",
        ),
        (
            "(list (assq 'b '(ignore (a . 1) (b . 2))) (plist-get '(a 1 b 2) 'b) (plist-get '(a) 'a))",
            "((b . 2) 2 nil)",
        ),
        (
            "(let ((x (cons 1 2))) (setcar x 3) (setcdr x 4) x)",
            "(3 . 4)",
        ),
        (
            "(condition-case e (car 7) (wrong-type-argument e))",
            "(wrong-type-argument listp 7)",
        ),
        (
            "(condition-case e (setcar nil 7) (wrong-type-argument e))",
            "(wrong-type-argument consp nil)",
        ),
        (
            "(condition-case nil (car) (wrong-number-of-arguments 9))",
            "9",
        ),
        (
            "(let ((x (cons 1 nil))) (setcdr x x) (condition-case nil (memq 7 x) (circular-list 9)))",
            "9",
        ),
        (
            "(let ((x (cons '(a . 1) nil))) (setcdr x x) (condition-case nil (assq 'b x) (circular-list 9)))",
            "9",
        ),
        (
            "(let ((x (list 'a 1))) (setcdr (cdr x) x) (plist-get x 'b))",
            "nil",
        ),
        (
            "(defun probe (x) (car x)) (probe '(1)) (defalias 'car (lambda (x) 42)) (probe '(1))",
            "42",
        ),
    ] {
        let value = vm
            .eval_source(source)
            .unwrap_or_else(|e| panic!("{source}: {e}"));
        assert_eq!(value.display().unwrap(), expected, "{source}");
    }
}

#[test]
fn null_redefinition_preserves_not_primitive_value_per_d003() {
    let mut vm = Vm::new();
    let value = vm
        .eval_source("(defalias 'null (lambda (x) 'nn)) (list (null nil) (not nil))")
        .unwrap();
    assert_eq!(value.display().unwrap(), "(nn t)");
    let value = vm.eval_source("(defalias 'not 'null) (not nil)").unwrap();
    assert_eq!(value.display().unwrap(), "nn");
}

#[test]
fn traversal_tail_dispatch_has_bounded_native_stack() {
    std::thread::Builder::new().stack_size(256 * 1024).spawn(|| {
        let mut vm = Vm::new();
        let value = vm.eval_source("(let ((x (list 1 2 3)) (i 0)) (while (< i 200000) (car x) (cdr x) (assq 1 nil) (plist-get nil 1) (nth 1 x) (setcar x 1) (setq i (1+ i))) i)").unwrap();
        assert_eq!(value.as_integer().unwrap(), 200000);
    }).unwrap().join().unwrap();
}

#[test]
fn arithmetic_tail_dispatch_has_bounded_native_stack() {
    std::thread::Builder::new().stack_size(256 * 1024).spawn(|| {
        let mut vm = Vm::new();
        let value = vm.eval_source("(let ((n 0) (v [3 4]) (i 0)) (while (< i 200000) (setq n (% (* n 31) (aref v 1))) (length v) (setq i (1+ i))) i)").unwrap();
        assert_eq!(value.as_integer().unwrap(), 200000);
    }).unwrap().join().unwrap();
}

#[test]
fn funcall_and_apply_tail_dispatch_has_bounded_native_stack() {
    std::thread::Builder::new().stack_size(256 * 1024).spawn(|| {
        let mut vm = Vm::new();
        let value = vm.eval_source("(let ((f #'abs) (g (lambda (&rest xs) (car xs))) (n 0) (i 0)) (while (< i 200000) (setq n (+ (funcall f i) (apply #'+ (list i 1)) (apply g i nil) (length (apply #'concat (list \"a\" \"b\")))) i (1+ i))) n)").unwrap();
        assert_eq!(value.as_integer().unwrap(), 3 * 199999 + 1 + 2);
    }).unwrap().join().unwrap();
}

#[test]
fn indirect_traversal_calls_use_the_same_semantics() {
    let mut vm = Vm::new();
    let value = vm
        .eval_source(
            "(list (funcall 'eq 3 3) (apply 'memq '(2 (1 2 3))) (funcall 'nth 2 '(4 5 6)))",
        )
        .unwrap();
    assert_eq!(value.display().unwrap(), "(t (2 3) 6)");
}
