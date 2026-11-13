//! The join of `mapconcat` (lisp/romacs/vm/prelude.el and `romacs--concat-list`)
//! against the pinned GNU: every kind of separator, the `identity` shortcut
//! over a list and a vector, multibyte and unibyte parts, and the text
//! properties the parts and the separator carry.
use romacs_vm::Vm;

const CASES: &[&str] = &[
    r#"(mapconcat #'identity '("ab" "cd" "ef") ",")"#,
    r#"(mapconcat #'identity '("ab" "cd" "ef") nil)"#,
    r#"(mapconcat #'identity '("ab" "cd" "ef") "")"#,
    r#"(mapconcat #'identity '("ab" "cd" "ef"))"#,
    r#"(mapconcat #'identity '("ab") ",")"#,
    r#"(mapconcat #'identity nil ",")"#,
    r#"(mapconcat #'identity ["ab" "cd"] "--")"#,
    r#"(mapconcat #'identity '("ab" "cd") '(?- ?>))"#,
    r#"(mapconcat #'identity '("ab" "cd") [?+])"#,
    r#"(mapconcat #'identity '("ab" "cd") [])"#,
    r#"(mapconcat #'identity '("aé" "λb") "·")"#,
    r#"(mapconcat #'identity '("ab" "λb") ",")"#,
    r#"(mapconcat #'identity (list (unibyte-string 97 200) "λ") ",")"#,
    r#"(multibyte-string-p (mapconcat #'identity (list (unibyte-string 97 200) (unibyte-string 98)) ","))"#,
    r#"(mapconcat #'symbol-name '(a b c) " ")"#,
    r#"(mapconcat #'number-to-string [1 2 3] "+")"#,
    r#"(mapconcat (lambda (c) (string c c)) "abc" "|")"#,
    r#"(mapconcat (lambda (x) (list x x)) '(?a ?b) ",")"#,
    r#"(let ((s (mapconcat #'identity (list (propertize "ab" 'face 'bold) "cd") (propertize "," 'p 1)))) (list s (text-properties-at 0 s) (text-properties-at 2 s) (text-properties-at 3 s)))"#,
    r#"(let* ((parts (list "ab" "cd")) (s (mapconcat #'identity parts ","))) (list s parts (eq (car parts) s)))"#,
    r#"(condition-case e (mapconcat #'identity '("a" . "b") ",") (error e))"#,
    r#"(condition-case e (mapconcat #'identity '("a" 1) ",") (error (car e)))"#,
];

#[test]
fn mapconcat_joins_as_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    for case in CASES {
        let source = format!("(prin1-to-string {case})");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(princ {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = match vm.eval_source(&format!("(append {source} nil)")) {
            Ok(value) => value.display().map(|codes| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>()),
            Err(error) => Err(error),
        };
        if actual.as_deref() != Ok(expected.as_str()) { failures.push(format!("{case}\n  GNU    {expected}\n  Romacs {actual:?}")); }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
