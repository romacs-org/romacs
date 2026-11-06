//! Overlays in a unibyte buffer against pinned GNU (buffer.c): making,
//! moving and deleting them, the queries, edits, `get-char-property',
//! and `set-buffer-multibyte' either way.
use romacs_vm::Vm;

/// One expression per line; each runs in a unibyte buffer of raw bytes.
const CASES: &str = r###"
(let ((o (make-overlay 2 4))) (list (overlay-start o) (overlay-end o) (eq (overlay-buffer o) (current-buffer))))
(let ((o (make-overlay 2 4))) (overlay-put o 'x 1) (list (length (overlays-at 2)) (length (overlays-at 4)) (length (overlays-in 1 8)) (next-overlay-change 1) (previous-overlay-change 8) (get-char-property 2 'x) (get-char-property 4 'x) (get-pos-property 3 'x)))
(let ((o (make-overlay 2 4))) (goto-char 3) (insert "z") (list (overlay-start o) (overlay-end o)))
(let ((o (make-overlay 2 4))) (goto-char 2) (insert (unibyte-string 255)) (list (overlay-start o) (overlay-end o)))
(let ((o (make-overlay 2 4 nil t nil))) (goto-char 2) (insert "z") (list (overlay-start o) (overlay-end o)))
(let ((o (make-overlay 2 5))) (delete-region 3 4) (list (overlay-start o) (overlay-end o)))
(let ((o (make-overlay 2 4))) (move-overlay o 1 3) (list (overlay-start o) (overlay-end o)))
(let ((o (make-overlay 2 4))) (delete-overlay o) (list (overlay-buffer o) (overlays-in 1 8)))
(let ((o (make-overlay 2 4))) (subst-char-in-region 1 8 195 65) (list (overlay-start o) (overlay-end o)))
(let ((o (make-overlay 2 4))) (upcase-region 1 8) (list (overlay-start o) (overlay-end o)))
(let ((o (make-overlay 2 4))) (remove-overlays 3 4) (mapcar (lambda (o) (list (overlay-start o) (overlay-end o))) (overlays-in 1 8)))
(let ((o (make-overlay 2 4))) (narrow-to-region 3 6) (prog1 (list (overlay-start o) (overlay-end o) (length (overlays-in 3 6))) (widen)))
(let ((o (make-overlay 2 4))) (overlay-put o 'x 1) (narrow-to-region 3 6) (prog1 (list (previous-overlay-change 6) (next-overlay-change 3) (get-pos-property 3 'x) (get-char-property 3 'x)) (widen)))
(let ((o (make-overlay 5 6))) (narrow-to-region 2 4) (prog1 (list (previous-overlay-change 4) (next-overlay-change 2)) (widen)))
(let ((o (make-overlay 2 4))) (set-buffer-multibyte t) (list (overlay-start o) (overlay-end o) (buffer-size) (length (overlays-in 1 (point-max)))))
(let ((o (make-overlay 2 3))) (set-buffer-multibyte t) (list (overlay-start o) (overlay-end o) (buffer-size)))
(let ((o (make-overlay 3 3))) (set-buffer-multibyte t) (list (overlay-start o) (overlay-end o) (buffer-size)))
(let ((o (make-overlay 3 5))) (set-buffer-multibyte t) (list (overlay-start o) (overlay-end o) (buffer-size)))
(let ((o (make-overlay 5 8))) (set-buffer-multibyte t) (list (overlay-start o) (overlay-end o) (buffer-size)))
(let ((o (make-overlay 2 4))) (set-buffer-multibyte t) (set-buffer-multibyte nil) (list (overlay-start o) (overlay-end o) (buffer-size)))
(let ((o (make-overlay 2 4))) (delete-all-overlays) (list (overlay-buffer o) (overlays-in 1 8)))
(let ((o (make-overlay 2 4)) (b (current-buffer))) (with-temp-buffer (overlay-start o)))
(let* ((b (get-buffer-create "ov")) (o (with-current-buffer b (set-buffer-multibyte nil) (insert "abc") (make-overlay 1 3)))) (kill-buffer b) (list (overlay-buffer o) (overlay-start o)))
(let ((o (make-overlay 2 4))) (garbage-collect) (list (overlay-start o) (overlay-end o) (length (overlays-at 2))))
(let ((o (make-overlay 2 4))) (overlay-put o 'evaporate t) (delete-region 2 4) (overlay-buffer o))
(let ((o (make-overlay 2 5))) (overlay-put o 'evaporate t) (delete-region 2 4) (list (overlay-start o) (overlay-end o)))
(let ((o (make-overlay 2 4)) (n 0)) (overlay-put o 'modification-hooks (list (lambda (&rest _) (setq n (1+ n))))) (goto-char 3) (insert "z") (delete-region 3 4) n)
(let ((o (make-overlay 2 4)) (n 0)) (overlay-put o 'insert-in-front-hooks (list (lambda (&rest _) (setq n (1+ n))))) (goto-char 2) (insert "z") n)
(let ((o (make-overlay 2 4)) (n 0)) (overlay-put o 'insert-behind-hooks (list (lambda (&rest _) (setq n (1+ n))))) (goto-char 4) (insert (unibyte-string 255)) n)
(let ((o (make-overlay 2 4)) (seen nil)) (overlay-put o 'modification-hooks (list (lambda (_o after b e &rest _) (push (list after b e) seen)))) (goto-char 3) (insert (unibyte-string 200)) (reverse seen))
(let ((o (make-overlay 2 3))) (transpose-regions 1 3 5 7) (list (overlay-start o) (overlay-end o)))
(let ((o (make-overlay 2 4))) (erase-buffer) (list (overlay-start o) (overlay-end o)))
(let ((o (make-overlay 2 4))) (overlay-put o 'x 1) (list (next-single-char-property-change 1 'x) (next-single-char-property-change 2 'x) (next-single-char-property-change 4 'x) (previous-single-char-property-change 8 'x) (previous-single-char-property-change 4 'x) (previous-single-char-property-change 3 'x) (next-char-property-change 1) (previous-char-property-change 8)))
(let ((o (make-overlay 6 7))) (overlay-put o 'x 1) (list (next-single-char-property-change 1 'x) (previous-single-char-property-change 8 'x) (next-char-property-change 6) (previous-char-property-change 7)))
(let ((o (make-overlay 2 4))) (overlay-put o 'x 1) (narrow-to-region 3 6) (prog1 (list (next-single-char-property-change 3 'x) (previous-single-char-property-change 6 'x)) (widen)))
(let ((o (make-overlay 2 3)) (m (copy-marker 2)) (n (copy-marker 3))) (transpose-regions 1 3 5 7) (list (overlay-start o) (overlay-end o) (marker-position m) (marker-position n)))
(let ((o (make-overlay 2 6)) (m (copy-marker 2)) (n (copy-marker 6))) (transpose-regions 1 3 5 7 t) (list (overlay-start o) (overlay-end o) (marker-position m) (marker-position n)))
(let ((o (make-overlay 2 4))) (overlay-put o 'evaporate t) (replace-region-contents 2 4 "" 0) (overlay-buffer o))
(let ((o (make-overlay 2 4))) (overlay-put o 'evaporate t) (replace-region-contents 2 4 "zz" 0) (list (overlay-start o) (overlay-end o)))
(let ((o (make-overlay 2 4 nil t nil))) (overlay-put o 'x 1) (list (get-pos-property 2 'x) (get-pos-property 4 'x)))
(let ((o (make-overlay 2 4 nil nil t))) (overlay-put o 'x 1) (list (get-pos-property 2 'x) (get-pos-property 4 'x)))
(let ((o (make-overlay 2 4))) (replace-region-contents 2 4 "zz" 0) (list (overlay-start o) (overlay-end o)))
(let ((o (make-overlay 3 6))) (replace-region-contents 2 4 "yyy" 0) (list (overlay-start o) (overlay-end o)))
(let ((o (make-overlay 3 3))) (replace-region-contents 2 4 "y" 0) (list (overlay-start o) (overlay-end o)))
(let ((o (make-overlay 1 2))) (replace-region-contents 2 4 (unibyte-string 255 254 253) 0) (list (overlay-start o) (overlay-end o) (buffer-size)))
(progn (dotimes (i 3) (make-overlay (1+ i) (+ i 3))) (mapcar (lambda (o) (overlay-start o)) (sort (overlays-in 1 8) (lambda (a b) (< (overlay-start a) (overlay-start b))))))
"###;
const SETUP: &str = "(with-temp-buffer (set-buffer-multibyte nil) (insert (unibyte-string 65 195 169 10 66 255 67)) ";

#[test]
fn overlays_in_a_unibyte_buffer_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        let source = format!("(prin1-to-string (condition-case e {SETUP}{expression}) (error (list 'signal e))))");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(princ {source})")])
            .output().expect("run pinned GNU");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        // `display' prints a string with its quotes, `princ' without.
        if actual.as_deref().map(|a| a.trim_matches('"')) != Ok(expected.as_str()) {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
