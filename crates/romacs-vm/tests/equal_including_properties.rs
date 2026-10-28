//! equal-including-properties against pinned GNU (fns.c internal_equal
//! with EQUAL_INCLUDING_PROPERTIES, compare_string_intervals): property
//! presence (nil values included), run splits, order, `equal' values,
//! nested strings in lists and vectors, and unibyte strings.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(list (equal-including-properties "a" "a") (equal-including-properties (propertize "a" 'p 1) "a") (equal-including-properties "a" (propertize "a" 'p 1)) (equal-including-properties (propertize "a" 'p nil) "a") (equal-including-properties (propertize "ab" 'p 1) (concat (propertize "a" 'p 1) (propertize "b" 'p 1))) (equal-including-properties (propertize "a" 'p (list 1)) (propertize "a" 'p (list 1))) (equal-including-properties (propertize "a" 'p 1 'q 2) (propertize "a" 'q 2 'p 1)) (equal-including-properties (propertize "a" 'p 1) (propertize "a" 'p 2)) (equal-including-properties (propertize "a" 'p 1) (propertize "a" 'q 1)) (equal-including-properties (propertize "a" 'p 1 'q 2) (propertize "a" 'p 1)))
(list (equal-including-properties (concat "a" (propertize "b" 'p 1)) (concat (propertize "a" 'p 1) "b")) (equal-including-properties (concat "x" (propertize "yz" 'face 'bold) "w") (let ((s (copy-sequence "xyzw"))) (put-text-property 1 3 'face 'bold s) s)) (equal-including-properties (list (propertize "a" 'p 1)) (list "a")) (equal-including-properties (list 1 (vector (propertize "a" 'p 1))) (list 1 (vector (propertize "a" 'p 1)))) (equal-including-properties ["x"] ["x"]) (equal-including-properties '(1 . 2) '(1 . 2)) (equal-including-properties 1.0 1.0) (equal-including-properties (propertize "a" 'p (propertize "v" 'q 1)) (propertize "a" 'p "v")) (equal-including-properties "" "") (equal-including-properties (string-to-unibyte "\377") (string-to-unibyte "\377")) (equal (propertize "a" 'p 1) "a"))
"###;

#[test]
fn equal_including_properties_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        // As character codes, as tests/json.rs compares.
        let source = format!("(append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil)");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
