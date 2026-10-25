use romacs_vm::Vm;

fn expressions() -> Vec<String> {
    let mut expressions = Vec::new();
    for spec in ["%f", "%e", "%g", "%.0f", "%.2e", "%.3g", "%#.0f", "%#.0e", "%#.3g", "%+012.3f", "% 12.3g", "%-12.2f"] {
        for value in ["0", "-0.0", "1.25", "-123.456", "2.5", "3.5", "999.99", "0.00009999", "0.00001", "1.0e+INF", "-1.0e+INF", "0.0e+NaN", "-0.0e+NaN", "9007199254740993", "(1- (* 4294967296 4294967296))"] {
            expressions.push(format!("(format {spec:?} {value})"));
        }
    }
    for spec in ["%d", "%i", "%x", "%X", "%o", "%b", "%B", "%#.0o", "%#012x", "%+.4d", "% 8x", "%-10.6o", "%.0d"] {
        for value in ["0", "-0.0", "1.75", "-1.75", "255", "-255", "(1- (* 4294967296 4294967296))", "(+ 1 (* 1099511627776 1099511627776))"] {
            expressions.push(format!("(format {spec:?} {value})"));
        }
    }
    for expression in [
        r#"(format "%9s %2d %f" "passed" 1 0.00125)"#,
        r#"(format "%s/%s: u%04x" 1 9 955)"#,
        r#"(format "[%5s][%.1s][%.2s]" "界x" "界x" "界x")"#,
        r#"(format "[%08s][%-8.3S]" "abc" '(1 2))"#,
        r#"(format "%2$s %1$s %% %2$s" "one" "two")"#,
        r#"(format "%5.0c" 955)"#,
        r#"(vconcat (format "[%5c][%5c][%.0c]" 9 0 9))"#,
        r#"(format "[%.0s][%.2s]" "\n" "éx")"#,
        r#"(format "%f" 'wrong)"#,
        r#"(format "%d" 'wrong)"#,
        r#"(format "%c" 1.5)"#,
        r#"(format "%x" 1.0e+INF)"#,
        r#"(format-message "%x" 1.0e+INF)"#,
        r#"(message "%x" 1.0e+INF)"#,
        r#"(format "%d" 0.0e+NaN)"#,
        r#"(format "%010f" 0.0e+NaN)"#,
        r#"(format "%d %d" 1)"#,
        r#"(vconcat (format "[%5s][%.1s]" (unibyte-string 255) (unibyte-string 255)))"#,
    ] { expressions.push(expression.to_owned()); }
    expressions
}

#[test]
fn ert_and_json_numeric_formats_are_supported() {
    let mut vm = Vm::new();
    for (source, expected) in [
        (r#"(format "%9s %2d %f" "passed" 1 0.00125)"#, "\"   passed  1 0.001250\""),
        (r#"(format "u%04x" 955)"#, "\"u03bb\""),
        (r#"(format "%d" (+ 1 (* 1099511627776 1099511627776)))"#, "\"1208925819614629174706177\""),
        (r#"(format "%.0f %.0f" 2.5 3.5)"#, "\"2 4\""),
        (r#"(format "%f" -0.0)"#, "\"-0.000000\""),
    ] {
        assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), expected, "{source}");
    }
}

#[test]
fn numeric_formats_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in expressions() {
        let source = format!("(prin1-to-string (condition-case e (list 'ok {expression}) (error (list 'signal (car e)))))");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(princ {source})")]).output().unwrap();
        assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        let actual = vm.eval_source(&source).and_then(|v| v.display())
            .map(|s| serde_json::from_str::<String>(&s).unwrap());
        if actual.as_deref() != Ok(expected.as_str()) { failures.push(format!("{expression}: GNU {expected:?}; VM {actual:?}")); }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
