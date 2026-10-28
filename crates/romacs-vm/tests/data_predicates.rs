use romacs_vm::Vm;
fn check(name: &str, positives: &[&str], negatives: &[&str]) {
    let mut vm = Vm::new();
    for (values, expected) in [(positives, "t"), (negatives, "nil")] {
        for value in values {
            for source in [format!("({name} {value})"), format!("(funcall #'{name} {value})")] {
                let actual = match vm.eval_source(&source) {
                    Ok(value) => value.display().unwrap(),
                    Err(error) => format!("ERROR: {error}"),
                };
                assert_eq!(actual, expected, "{name} predicate assertion: {source}");
            }
        }
    }
}

#[test]
fn floatp_gnu_numeric_tower() {
    check("floatp", &["0.0", "-0.0", "1.5", "-1.5", "1.0e+INF", "-1.0e+INF", "0.0e+NaN"],
        &["0", "-1", "2305843009213693951", "#x2000000000000000", "#x-2000000000000001", "nil", "t", "'x", "\"x\"", "'(1)", "[1]", "(make-marker)", "(current-buffer)", "(symbol-function 'car)", "(lambda (x) x)"]);
}
#[test]
fn integer_or_marker_p_gnu_numeric_tower() {
    check("integer-or-marker-p", &["0", "-1", "2305843009213693951", "-2305843009213693952", "#x2000000000000000", "#x-2000000000000001", "#xffffffffffffffffffffffffffffffff", "(make-marker)", "(set-marker (make-marker) 1)"],
        &["0.0", "-0.0", "1.5", "1.0e+INF", "0.0e+NaN", "nil", "t", "'x", "\"x\"", "'(1)", "[1]", "(current-buffer)", "(symbol-function 'car)", "(lambda (x) x)"]);
}
#[test]
fn number_or_marker_p_gnu_numeric_tower() {
    check("number-or-marker-p", &["0", "-1", "2305843009213693951", "-2305843009213693952", "#x2000000000000000", "#x-2000000000000001", "#xffffffffffffffffffffffffffffffff", "0.0", "-0.0", "1.5", "1.0e+INF", "0.0e+NaN", "(make-marker)", "(set-marker (make-marker) 1)"],
        &["nil", "t", "'x", "\"x\"", "'(1)", "[1]", "(current-buffer)", "(symbol-function 'car)", "(lambda (x) x)"]);
}
#[test]
fn char_or_string_p_gnu_character_boundaries() {
    check("char-or-string-p", &["0", "65", "4194303", "\"\"", "\"λ\"", "(unibyte-string 255)"],
        &["-1", "4194304", "2305843009213693951", "#x2000000000000000", "0.0", "nil", "t", "'x", "'(1)", "[1]", "(make-marker)", "(current-buffer)", "(symbol-function 'car)", "(lambda (x) x)"]);
}

#[test]
fn bare_symbol_p_gnu_objects() {
    check("bare-symbol-p", &["nil", "t", "'x", ":key", "(make-symbol \"x\")"],
        &["0", "0.0", "#x2000000000000000", "\"x\"", "'(1)", "[1]", "(make-marker)", "(current-buffer)", "(symbol-function 'car)", "(lambda (x) x)"]);
}

#[test]
fn markerp_gnu_objects() {
    check("markerp", &["(make-marker)", "(set-marker (make-marker) 1)"], &["nil", "0", "#x2000000000000000", "0.0", "\"x\"", "'(1)", "[1]", "(current-buffer)"]);
}

#[test]
fn vector_or_char_table_p_gnu_objects() {
    check("vector-or-char-table-p", &["[]", "[1]", "(make-char-table 'x)", "(standard-syntax-table)"], &["nil", "0", "\"x\"", "'(1)", "(make-marker)", "(current-buffer)"]);
}

#[test]
fn arrayp_gnu_objects() {
    check("arrayp", &["[]", "[1]", "\"\"", "\"λ\"", "(unibyte-string 255)", "(make-char-table 'x)", "(standard-syntax-table)"], &["nil", "0", "'(1)", "(make-marker)", "(current-buffer)"]);
}

#[test]
fn sequencep_gnu_objects() {
    check("sequencep", &["nil", "'(1 . 2)", "[]", "[1]", "\"\"", "\"λ\"", "(unibyte-string 255)", "(make-char-table 'x)", "(standard-syntax-table)"], &["t", "0", "0.0", "#x2000000000000000", "(make-marker)", "(current-buffer)"]);
}

#[test]
fn data_predicates_construct_the_first_bignum() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(condition-case e (read-from-string \"2305843009213693952\") (error e))", "(2305843009213693952 . 19)"),
        ("(condition-case e (string-to-number \"2305843009213693952\") (error e))", "2305843009213693952"),
    ] {
        assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), expected, "first bignum: {source}");
    }
}

#[test]
fn natnump_gnu_numeric_tower() {
    check("natnump", &["0", "1", "1152921504606846975", "1152921504606846976", "2305843009213693951", "#x2000000000000000", "#x10000000000000000", "#xb7abc627050305adf14a3d9e3ffffffffff"],
        &["-1", "-1152921504606846976", "-1152921504606846977", "#x-2000000000000001", "#x-2f050fe938943acc45f65567fffffffff", "0.0", "-0.0", "1.0", "1.0e+INF", "0.0e+NaN", "nil", "t", "'x", "\"x\"", "'(1)", "[1]", "(make-marker)", "(current-buffer)", "(symbol-function 'car)", "(lambda (x) x)", "(make-hash-table)", "(make-char-table 'x)"]);
}
