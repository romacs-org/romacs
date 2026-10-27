//! char-width and string-width against pinned GNU (character.c): ASCII,
//! tabs and `tab-width', control characters and `ctl-arrow', newline, raw
//! bytes, unibyte strings, `char-width-table' (bound here, as the bare VM
//! does not load characters.el), display tables, FROM/TO and errors.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(list (string-width "abc") (string-width "") (string-width "a\tb") (let ((tab-width 4)) (string-width "\t\t")) (let ((tab-width 0)) (string-width "\t")) (let ((tab-width 2000)) (char-width ?\t)) (string-width "\n") (string-width "\x01\x7f") (let ((ctl-arrow nil)) (string-width "\x01\x7f")) (string-width "\x80") (string-width (string-to-multibyte "\x80")) (string-width (string-to-unibyte "\377")) (char-width ?a) (char-width ?\n) (char-width #x3fff80))
(let ((char-width-table (make-char-table nil 1))) (set-char-table-range char-width-table '(#x3000 . #x30ff) 2) (aset char-width-table #x301 0) (aset char-width-table #x2000 2000) (aset char-width-table #x2001 -3) (list (string-width "あいx") (string-width "é") (char-width #x3042) (char-width #x2000) (char-width #x2001) (string-width (propertize "あb" 'face 'bold)) (string-width "あいう" 1) (string-width "あいう" -1) (string-width "あいう" 0 -2) (string-width (propertize "ab" 'display "xxxxx" 'invisible t))))
(progn (put 'display-table 'char-table-extra-slots 18) (let ((buffer-display-table (make-char-table 'display-table))) (aset buffer-display-table ?a [?x ?y ?z]) (aset buffer-display-table ?b [(?x . 1) 42 nil "s" (?y . -1)]) (aset buffer-display-table ?c []) (aset buffer-display-table ?d "not a vector") (aset buffer-display-table ?e [#x400061]) (list (string-width "abcde") (char-width ?a) (char-width ?b) (char-width ?c) (char-width ?d) (char-width ?e) (string-width "f"))))
(progn (put 'display-table 'char-table-extra-slots 18) (let ((standard-display-table (make-char-table 'display-table))) (aset standard-display-table ?a [?\t ?\t]) (list (string-width "a") (let ((buffer-display-table (make-char-table 'display-table))) (string-width "a")) (let ((buffer-display-table (make-char-table 'other))) (string-width "a")))))
(list (condition-case e (string-width "abc" 2 1) (error e)) (condition-case e (string-width "abc" 0 4) (error e)) (condition-case e (string-width "abc" -4) (error e)) (condition-case e (string-width "abc" 'x) (error e)) (condition-case e (string-width 1) (error e)) (condition-case e (string-width nil) (error e)) (condition-case e (char-width -1) (error e)) (condition-case e (char-width #x400000) (error e)) (condition-case e (char-width "a") (error e)) (string-width "abc" 3 3) (string-width "abc" nil 0))
"###;

#[test]
fn string_width_matches_pinned_gnu() {
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
