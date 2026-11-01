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

/// String literals read from Lisp strings take each run of plain characters
/// in one pass (`Reader::string_literal_run'); GNU's results across the
/// run's edges: escapes and continuations inside it, multibyte and unibyte
/// sources with raw bytes, a start after multibyte text, an END cutting the
/// literal, and the position after it.
#[test]
fn string_literal_runs_read_as_gnu_does() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let cases = [
        r#"(read-from-string "\"plain text\" tail")"#,
        r#"(read-from-string "\"a\\\"b\\\\c\\nd\\\ne\\ f\\x41\\ g\" tail")"#,
        r#"(read-from-string "x \"λ μ\\u00e9ν\" y" 1)"#,
        r#"(read-from-string "λλ \"κείμενον\\tλ\"" 3)"#,
        r#"(read-from-string (string-to-unibyte "\"a\377b\"") 0)"#,
        r#"(read-from-string "\"\\351\" rest")"#,
        r#"(read-from-string "\"abcdef\"" 0 4)"#,
        r#"(read-from-string "(\"one\" \"two λ\" . \"three\")")"#,
        r##"(read-from-string "#(\"text\" 0 2 (face bold))")"##,
        r#"(read-from-string "\"\"")"#,
    ];
    let mut failures = Vec::new();
    for case in cases {
        // As character codes with multibyteness, as tests/json.rs compares.
        let source = format!(
            "(let ((r (condition-case e {case} (error (list 'signal e))))) (append (prin1-to-string (list r (and (stringp (car-safe r)) (multibyte-string-p (car r))))) nil))"
        );
        let output = std::process::Command::new(&oracle).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            failures.push(format!("{case}\n  GNU    {expected}\n  Romacs {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
