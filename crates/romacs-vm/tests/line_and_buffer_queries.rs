//! pos-bol, pos-eol, buffer-base-buffer and line-number-display-width
//! against pinned GNU (editfns.c, buffer.c, indent.c): counts, clipping,
//! narrowing, bignums, fields ignored, buffers without a base, and the
//! width while `display-line-numbers' is off.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(with-temp-buffer (insert "aα\nbb\ncc") (goto-char 5) (list (pos-bol) (pos-eol) (pos-bol 0) (pos-eol 2) (pos-eol 999) (pos-bol -999) (pos-bol 1) (pos-eol 1) (pos-eol 0) (pos-eol -1) (pos-bol (1+ most-positive-fixnum)) (pos-eol (1- most-negative-fixnum)) (point) (progn (narrow-to-region 2 8) (list (pos-bol -999) (pos-eol 999) (point))) (condition-case e (pos-bol 'x) (error e)) (condition-case e (pos-eol 1.5) (error e))))
(with-temp-buffer (insert "ab cd\nef") (put-text-property 1 4 'field 'f) (goto-char 3) (list (pos-bol) (pos-eol) (progn (goto-char (point-max)) (list (pos-bol) (pos-eol)))))
(let ((b (get-buffer-create " rt-base"))) (list (buffer-base-buffer) (buffer-base-buffer b) (progn (kill-buffer b) (buffer-base-buffer b)) (condition-case e (buffer-base-buffer 'x) (error e)) (condition-case e (buffer-base-buffer "rt-none") (error e))))
(list (line-number-display-width) (line-number-display-width 'columns) (line-number-display-width t) (with-temp-buffer (line-number-display-width 'columns)))
"###;

#[test]
fn line_and_buffer_queries_match_pinned_gnu() {
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
