//! Scaling probes: a loop of searches and edits over a buffer of N lines
//! copies O(N) bytes in all, not O(N) per edit. Counted, not timed: the
//! bytes texts copy whole or move (`text::copied_bytes'), at N and 4N.
use crate::Vm;

/// The bytes LOOP copies over a buffer of LINES lines `wordI/ABC'.
fn copied(lines: usize, body: &str) -> usize {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let source = format!(
        "(with-temp-buffer
           (let ((i 0)) (while (< i {lines}) (insert \"word\" (number-to-string i) \"/ABC (x)\\n\") (setq i (1+ i))))
           (goto-char (point-min))
           (insert \"x\")
           {body})"
    );
    let before = crate::text::copied_bytes();
    vm.eval_source(&source).unwrap();
    crate::text::copied_bytes() - before
}

/// Each loop copies at most about four times as much over four times the
/// lines (a quadratic loop copies sixteen), and a small multiple of the
/// text: a quadratic loop over 4000 lines copies thousands of times it.
fn assert_linear(name: &str, body: &str) {
    let (small, large) = (copied(1000, body), copied(4000, body));
    let text = 4000 * "word1000/ABC (x)\n".len();
    eprintln!("{name}: {small} bytes copied at 1000 lines, {large} at 4000");
    assert!(large <= 5 * small.max(1) && large <= 20 * text, "{name}: {small} bytes at 1000 lines, {large} at 4000");
}

#[test]
fn search_edit_loops_copy_linearly() {
    let loops = [
        ("re-search-forward + replace-match", r#"(while (re-search-forward "/[^\n]*$" nil t) (replace-match ""))"#),
        ("re-search-backward + replace-match", r#"(goto-char (point-max)) (while (re-search-backward "/[A-Z]+" nil t) (replace-match "-\\&") (goto-char (match-beginning 0)))"#),
        ("search-forward + delete-region", r#"(while (search-forward "/" nil t) (delete-region (1- (point)) (line-end-position)))"#),
        ("looking-at + forward-line", r#"(while (not (eobp)) (when (looking-at "word\\([0-9]+\\)") (replace-match "W\\1" t)) (forward-line 1))"#),
        ("bounded search + insert", r#"(while (re-search-forward "\\bA" (line-end-position 2) t) (insert "b") (forward-line 1))"#),
        ("char-before, forward-word, current-column", r#"(while (not (eobp)) (insert "y") (char-before) (forward-word 1) (current-column) (forward-line 1))"#),
        ("parse-partial-sexp per line", r#"(while (not (eobp)) (insert "z") (parse-partial-sexp (line-beginning-position) (line-end-position)) (forward-line 1))"#),
    ];
    for (name, body) in loops {
        assert_linear(name, body);
    }
}
