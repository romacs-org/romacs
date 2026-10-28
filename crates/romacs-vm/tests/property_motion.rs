//! Property motion against pinned GNU (textprop.c): next-single-,
//! previous-single- and previous-property-change over strings and buffers,
//! with limits, markers, narrowing and `category', and the char-property
//! versions that compose them with overlay changes; and marker positions in
//! get-text-property and text-properties-at, as button.el's button-get passes them.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(let ((s (concat "ab" (propertize "cd" 'p 1 'q 2) (propertize "ef" 'p 1) "gh"))) (list (mapcar (lambda (i) (list (next-single-property-change i 'p s) (next-single-property-change i 'q s) (previous-single-property-change i 'p s) (previous-single-property-change i 'q s) (previous-property-change i s))) '(0 1 2 3 4 5 6 7 8)) (next-single-property-change 0 'p s 1) (next-single-property-change 0 'p s 20) (next-single-property-change 7 'p s 3) (previous-single-property-change 8 'p s 7) (previous-single-property-change 8 'p s 0) (previous-single-property-change 5 'p s 3) (previous-property-change 8 s 7) (previous-property-change 8 s -2)))
(let ((s (propertize "abc" 'p 1))) (list (next-single-property-change 0 'p s) (next-single-property-change 0 'p s 2) (previous-single-property-change 3 'p s) (previous-single-property-change 3 'p s 1) (previous-property-change 3 s) (next-single-property-change 0 'p "") (previous-single-property-change 0 'p "") (condition-case e (next-single-property-change 4 'p s) (error e)) (condition-case e (previous-single-property-change -1 'p s) (error e))))
(with-temp-buffer (insert "abcdefghij") (put-text-property 3 5 'p 1) (put-text-property 5 7 'p 2) (put-text-property 4 8 'q 1) (list (mapcar (lambda (i) (list (next-single-property-change i 'p) (previous-single-property-change i 'p) (previous-property-change i))) '(1 2 3 4 5 6 7 8 9 10 11)) (next-single-property-change 1 'p nil (copy-marker 2)) (previous-single-property-change (copy-marker 11) 'q nil 9) (previous-single-property-change 11 'q (current-buffer) 3) (progn (narrow-to-region 4 9) (list (next-single-property-change 4 'p) (next-single-property-change 7 'q) (previous-single-property-change 9 'q) (previous-single-property-change 5 'p) (previous-property-change 9) (condition-case e (next-single-property-change 2 'p) (error e)) (condition-case e (previous-single-property-change 10 'p) (error e))))))
(progn (put 'romacs-pm-cat 'p 7) (let ((s (concat "a" (propertize "b" 'category 'romacs-pm-cat) (propertize "c" 'p 7) "d"))) (list (next-single-property-change 0 'p s) (next-single-property-change 1 'p s) (previous-single-property-change 4 'p s) (previous-single-property-change 3 'p s) (previous-property-change 3 s))))
(with-temp-buffer (insert "abcdefghij") (put-text-property 3 4 'p 1) (let ((o (make-overlay 6 8))) (overlay-put o 'p 2) (list (mapcar (lambda (i) (list (next-char-property-change i) (previous-char-property-change i) (previous-single-char-property-change i 'p))) '(1 2 3 4 5 6 7 8 9 10 11)) (next-char-property-change 1 2) (next-char-property-change 1 (copy-marker 20)) (previous-char-property-change 11 9) (previous-char-property-change 11 (copy-marker -5)) (previous-single-char-property-change 11 'p nil 9) (previous-single-char-property-change 11 'p nil (copy-marker 7)) (previous-single-char-property-change 3 'p nil 5) (previous-single-char-property-change (copy-marker 9) 'p (current-buffer)) (progn (narrow-to-region 4 9) (list (previous-single-char-property-change 9 'p) (previous-single-char-property-change 9 'p nil 2) (previous-char-property-change 9) (next-char-property-change 4))))))
(let ((s (concat "ab" (propertize "cd" 'p 1) "e")) (b (get-buffer-create " romacs-pm"))) (with-current-buffer b (erase-buffer) (insert "xyz") (put-text-property 2 3 'p 1)) (list (previous-single-char-property-change 5 'p s) (previous-single-char-property-change 3 'p s) (previous-single-char-property-change 2 'p s) (previous-single-char-property-change 2 'p s 1) (condition-case e (previous-single-char-property-change 2 'p s 'x) (error e)) (previous-single-char-property-change 4 'p b) (previous-single-char-property-change 3 'p b) (condition-case e (previous-single-char-property-change 3 'p 'x) (error e)) (condition-case e (previous-single-char-property-change 'x 'p) (error e)) (condition-case e (next-char-property-change 1 'x) (error e)) (condition-case e (previous-char-property-change 1 'x) (error e)) (eq (current-buffer) b)))
(with-temp-buffer (insert "x Go") (put-text-property 3 5 'q 3) (put-text-property 1 2 'r 1) (let ((s (propertize "xy" 'q 2)) (m (copy-marker 4 t))) (list (get-text-property (copy-marker 3) 'q) (text-properties-at (copy-marker 1)) (get-text-property (copy-marker 1) 'q s) (get-text-property m 'q (marker-buffer m)) (text-properties-at m) (get-text-property (point-marker) 'q))))
"###;

#[test]
fn property_motion_matches_pinned_gnu() {
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
