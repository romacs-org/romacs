//! upcase-word, downcase-word and capitalize-word (casefiddle.c
//! casify_word) and window-text-width and window-text-height (window.c)
//! against pinned GNU in batch.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(with-temp-buffer (insert "hello world foo") (goto-char 1) (list (upcase-word 1) (point) (buffer-string)))
(with-temp-buffer (insert "hello world foo") (goto-char 3) (list (upcase-word 2) (point) (buffer-string)))
(with-temp-buffer (insert "Hello World Foo") (goto-char (point-max)) (list (downcase-word -2) (point) (buffer-string)))
(with-temp-buffer (insert "hELLO wORLD") (goto-char 1) (list (capitalize-word 2) (point) (buffer-string)))
(with-temp-buffer (insert "heLLo") (goto-char 3) (list (capitalize-word 1) (point) (buffer-string)))
(with-temp-buffer (insert "one two  ") (goto-char 1) (list (upcase-word 5) (point) (buffer-string)))
(with-temp-buffer (insert "  one two") (goto-char (point-max)) (list (upcase-word -5) (point) (buffer-string)))
(with-temp-buffer (insert "straße groß") (goto-char 1) (list (upcase-word 2) (point) (buffer-string)))
(with-temp-buffer (insert "abc def") (goto-char 1) (list (upcase-word 0) (point) (buffer-string)))
(with-temp-buffer (insert "abc") (condition-case e (upcase-word nil) (error e)))
(with-temp-buffer (insert "abc def") (goto-char 1) (let ((m (copy-marker 6))) (list (upcase-word 2) (point) (marker-position m) (buffer-string))))
(with-temp-buffer (insert "a b c") (narrow-to-region 3 4) (goto-char 3) (list (upcase-word 3) (point) (progn (widen) (buffer-string))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "abc def") (goto-char 1) (list (upcase-word 5) (buffer-string) (point)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "abc def") (list (capitalize-word -5) (buffer-string) (point)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "ABC def") (goto-char 1) (list (downcase-word 1) (buffer-string) (point)))
(list (= (window-text-width) (window-body-width)) (= (window-text-width nil t) (window-body-width nil t)) (= (window-text-height) (window-body-height)) (= (window-text-height nil t) (window-body-height nil t)) (natnump (window-text-width)))
(condition-case e (window-text-width 'nope) (error e))
(list (commandp 'upcase-word) (interactive-form 'upcase-word) (interactive-form 'downcase-word) (interactive-form 'capitalize-word))
"###;

#[test]
fn case_words_and_window_text_sizes_match_pinned_gnu() {
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
