//! The C primitives GNU's minibuffer completion calls, against pinned GNU in
//! batch: the same forms must print the same. `completion--flex-cost-gotoh`
//! (minibuf.c) scores the `flex` style, case-sensitively and with
//! `completion-ignore-case'; `read-file-name' asks
//! `next-read-file-uses-dialog-p' (fileio.c) first.
use romacs_vm::Vm;

const CASES: &[&str] = &[
    // minibuf.c's worked example: the scattered match loses to the tight one.
    r#"(completion--flex-cost-gotoh "goto" "eglot--goto")"#,
    r#"(completion--flex-cost-gotoh "fin" "find-file")"#,
    r#"(completion--flex-cost-gotoh "ff" "find-file")"#,
    r#"(completion--flex-cost-gotoh "fifi" "find-file-other-window")"#,
    r#"(completion--flex-cost-gotoh "abc" "abc")"#,
    r#"(completion--flex-cost-gotoh "abc" "xaxbxc")"#,
    r#"(completion--flex-cost-gotoh "aa" "aaaa")"#,
    r#"(completion--flex-cost-gotoh "ba" "abab")"#,
    r#"(completion--flex-cost-gotoh "zz" "find-file")"#,
    r#"(completion--flex-cost-gotoh "FIN" "find-file")"#,
    r#"(let ((completion-ignore-case t)) (completion--flex-cost-gotoh "FIN" "find-file"))"#,
    r#"(let ((completion-ignore-case t)) (completion--flex-cost-gotoh "zz" "find-file"))"#,
    r#"(let ((completion-ignore-case t)) (completion--flex-cost-gotoh "λΣ" "αλφα-σ"))"#,
    r#"(completion--flex-cost-gotoh "λσ" "αλφα-σ")"#,
    r#"(completion--flex-cost-gotoh "" "find-file")"#,
    r#"(completion--flex-cost-gotoh "f" "")"#,
    r#"(completion--flex-cost-gotoh "f" (make-string 511 ?f))"#,
    r#"(completion--flex-cost-gotoh "f" (make-string 40000 ?f))"#,
    r#"(completion--flex-cost-gotoh (make-string 127 ?a) (make-string 511 ?a))"#,
    r#"(completion--flex-cost-gotoh (make-string 128 ?a) (make-string 511 ?a))"#,
    r#"(completion--flex-cost-gotoh "a\377" (string-to-unibyte "xa\377"))"#,
    r#"(completion--flex-cost-gotoh 'fin "find-file")"#,
    r#"(completion--flex-cost-gotoh "fin" nil)"#,
    r#"(next-read-file-uses-dialog-p)"#,
    r#"(let ((last-nonmenu-event nil) (use-dialog-box t) (use-file-dialog t)) (next-read-file-uses-dialog-p))"#,
];

#[test]
fn completion_primitives_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    let mut vm = Vm::new();
    for expression in CASES {
        // As character codes, as tests/json.rs compares.
        let source = format!("(append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil)");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
