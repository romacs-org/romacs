use romacs_vm::Vm;

#[test]
fn read_from_string_positions_and_faults() {
    for (source, expected) in [
        (r#"(read-from-string "  42 tail")"#, "(42 . 4)"),
        (r#"(read-from-string "λ abc" 2)"#, "(abc . 5)"),
        (r#"(read-from-string "foo" -1)"#, "(o . 3)"),
        (r#"(read-from-string "foo" 0 8)"#, "(args-out-of-range \"foo\" 0 8)"),
        (r#"(read-from-string "foo" 2 1)"#, "(args-out-of-range \"foo\" 2 1)"),
        (r#"(read-from-string "foo" 4)"#, "(args-out-of-range \"foo\" 4 nil)"),
        (r#"(read-from-string "foo" t)"#, "(wrong-type-argument integerp t)"),
        (r#"(read-from-string "")"#, "(end-of-file)"),
        (r#"(read-from-string "(")"#, "(end-of-file)"),
        (r#"(read-from-string ")")"#, "(invalid-read-syntax \")\")"),
        (r#"(read-from-string "[1 (a . b) \"λ\"] rest")"#, "([1 (a . b) \"λ\"] . 15)"),
    ] {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let value = vm.eval_source(&format!("(condition-case e {source} (error e))")).unwrap();
        assert_eq!(value.display().unwrap(), expected, "{source}");
    }
}

#[test]
fn documented_reader_boundaries_are_not_claimed_as_gnu_parity() {
    for (source, expected) in [
        (r#"(read-from-string "a#b")"#, "(a#b . 3)"),
        (r#"(let ((r (read-from-string (concat "a" (string 160) "b")))) (list (length (symbol-name (car r))) (cdr r)))"#, "(3 3)"),
        (r#"(let ((r (read-from-string (concat "a" (string 11) "b")))) (list (length (symbol-name (car r))) (cdr r)))"#, "(1 1)"),
        (r#"(list (numberp (car (read-from-string "2.25e+INF"))) (numberp (car (read-from-string "-3.75e+INF"))))"#, "(nil nil)"),
        (r#"(read-from-string "(a . b c)")"#, "(invalid-read-syntax \"dotted list requires one tail at byte 8\")"),
        (r#"(let ((i 0) (s "0")) (while (< i 256) (setq s (concat "(" s ")")) (setq i (1+ i))) (cdr (read-from-string s)))"#, "513"),
        (r#"(let ((i 0) (s "0")) (while (< i 257) (setq s (concat "(" s ")")) (setq i (1+ i))) (cdr (read-from-string s)))"#, "(invalid-read-syntax \"reader nesting limit exceeded at byte 257\")"),
    ] {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let value = vm.eval_source(&format!("(condition-case e {source} (error e))")).unwrap();
        assert_eq!(value.display().unwrap(), expected, "{source}");
    }
}
