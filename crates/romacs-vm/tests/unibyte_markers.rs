//! Markers in unibyte buffers against pinned GNU in batch: GNU's markers
//! live in every buffer (insdel.c adjust_markers_for_*), and url-http.el's
//! connection buffers, which are unibyte, carry the process mark and its
//! header markers.
use romacs_vm::Vm;

const SETUP: &str = r#"(progn (set-buffer (get-buffer-create "u")) (set-buffer-multibyte nil) (erase-buffer) (insert "abcdef"))"#;

const CASES: &[&str] = &[
    "(let ((m (set-marker (make-marker) 3))) (list (marker-position m) (eq (marker-buffer m) (current-buffer)) (copy-marker m)))",
    "(let ((m (copy-marker 3)) (r (copy-marker 3 t))) (goto-char 3) (insert \"XY\") (list m r (point)))",
    "(let ((m (copy-marker 3)) (r (copy-marker 3 t))) (goto-char 3) (insert-before-markers \"XY\") (list m r (point)))",
    "(let ((a (copy-marker 2)) (b (copy-marker 4)) (c (copy-marker 6))) (delete-region 3 5) (list a b c))",
    "(let ((a (copy-marker 2)) (b (copy-marker 3)) (c (copy-marker 4)) (d (copy-marker 5 t))) (goto-char 4) (list (save-excursion (replace-region-contents 2 4 \"MNO\" 0) (point)) a b c d (point)))",
    "(let ((m (copy-marker 7))) (goto-char (point-max)) (insert \"\\377\\200\") (list m (marker-position m) (set-marker m 100) (set-marker m 0)))",
    "(let ((m (copy-marker 4))) (set-marker-insertion-type m t) (goto-char 4) (insert \"z\") (list m (marker-insertion-type m)))",
    "(let ((m (copy-marker 4))) (set-buffer-multibyte t) (list enable-multibyte-characters m (progn (set-buffer-multibyte nil) m)))",
    "(let ((m (copy-marker 4))) (set-buffer-multibyte t) (goto-char 1) (insert \"λ\") (set-buffer-multibyte nil) (list m (buffer-size)))",
    "(let ((m (copy-marker 3)) (other (get-buffer-create \"o\"))) (with-current-buffer other (set-buffer-multibyte nil) (insert \"12345\")) (set-marker m 2 other) (prog1 (list (marker-position m) (equal (buffer-name (marker-buffer m)) \"o\")) (kill-buffer other)))",
    "(let ((m (copy-marker 3)) (other (get-buffer-create \"k\"))) (with-current-buffer other (set-buffer-multibyte nil) (insert \"12345\")) (set-marker m 2 other) (kill-buffer other) (list (marker-position m) (marker-buffer m)))",
    "(list (save-excursion (goto-char 2) (insert \"q\") (point)) (point) (save-restriction (narrow-to-region 2 4) (goto-char 1) (insert \"N\") (list (point-min) (point-max))) (point-min) (point-max))",
    "(list (char-after (copy-marker 2)) (progn (goto-char (copy-marker 5)) (point)) (buffer-substring (copy-marker 2) (copy-marker 4)))",
];

#[test]
fn unibyte_markers_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES {
        let source = format!("(progn {SETUP} (prin1-to-string (condition-case e {expression} (error (list 'signal e)))))");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(princ {source})")])
            .output()
            .unwrap();
        assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_collection_floor(1);
        let actual = vm.eval_source(&source).and_then(|v| v.display()).map(|printed| {
            serde_json::from_str::<String>(&printed).unwrap_or(printed)
        }).unwrap_or_else(|e| format!("ERR {e}"));
        if actual != expected {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
