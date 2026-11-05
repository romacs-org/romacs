//! transpose-regions (editfns.c) against pinned GNU in batch: the text,
//! point, markers (transpose_markers, or kept with LEAVE-MARKERS), text
//! properties, overlays, undo, change hooks and its errors.
use romacs_vm::Vm;

/// One expression per line.  The multibyte cases compare characters only:
/// after a transposition with LEAVE-MARKERS pinned GNU leaves point's
/// PT_BYTE stale, and buf_charpos_to_bytepos consults it (position-bytes
/// of character 2 in "xyzw-αβ" reads 3); `multibyte_positions_stay_coherent'
/// checks Romacs's bytes itself.
const CASES: &str = r###"
(with-temp-buffer (insert "abcXYZdefg") (goto-char 2) (list (transpose-regions 1 3 7 11) (buffer-string) (point)))
(with-temp-buffer (insert "abcdef") (goto-char 5) (list (transpose-regions 1 4 4 7) (buffer-string) (point)))
(with-temp-buffer (insert "abcXYZdefg") (list (transpose-regions 7 11 1 3) (buffer-string)))
(with-temp-buffer (insert "abcXYZdefg") (list (transpose-regions 11 7 3 1) (buffer-string)))
(with-temp-buffer (insert "abcdef") (condition-case e (transpose-regions 1 4 3 6) (error e)))
(with-temp-buffer (insert "abcdef") (let ((n 0)) (set (make-local-variable 'before-change-functions) (list (lambda (&rest _) (setq n (1+ n))))) (list (transpose-regions 1 1 1 4) (transpose-regions 2 4 4 4) (buffer-string) n (buffer-modified-p))))
(with-temp-buffer (insert "abcdef") (list (transpose-regions 1 1 3 5) (buffer-string)))
(with-temp-buffer (insert "abcdef") (condition-case e (transpose-regions 1 3 4 9) (error e)))
(with-temp-buffer (insert "abcdef") (narrow-to-region 2 6) (condition-case e (transpose-regions 1 3 4 6) (error e)))
(with-temp-buffer (insert "abcdef") (setq buffer-read-only t) (condition-case e (transpose-regions 1 3 4 6) (error e)))
(with-temp-buffer (insert "αβ-xyzw") (let ((ms (mapcar #'copy-marker '(1 2 3 4 5 6 7 8)))) (goto-char 2) (transpose-regions 1 3 4 8) (list (buffer-string) (point) (mapcar #'marker-position ms) (position-bytes (point)) (mapcar (lambda (m) (position-bytes m)) ms))))
(with-temp-buffer (insert "αβ-xyzw") (let ((ms (mapcar #'copy-marker '(1 2 3 4 5 6 7 8)))) (goto-char 2) (transpose-regions 1 3 4 8 t) (list (buffer-string) (point) (mapcar #'marker-position ms))))
(with-temp-buffer (insert "ab-cde") (let ((a (copy-marker 1)) (b (copy-marker 1 t)) (c (copy-marker 3)) (d (copy-marker 4 t)) (e (copy-marker 7))) (transpose-regions 1 3 4 7) (mapcar #'marker-position (list a b c d e))))
(with-temp-buffer (insert (propertize "ab" 'face 'bold) (propertize "--" 'p 1) (propertize "xyz" 'face 'italic 'q 2)) (transpose-regions 1 3 5 8) (format "%S" (buffer-string)))
(with-temp-buffer (insert (propertize "ab" 'face 'bold) "--" (propertize "xy" 'face 'italic)) (transpose-regions 1 3 5 7) (format "%S" (buffer-string)))
(with-temp-buffer (insert (propertize "αb" 'face 'bold) "-" (propertize "xyz" 'face 'italic)) (transpose-regions 1 3 4 7) (format "%S" (buffer-string)))
(with-temp-buffer (insert "abcXYZdefg") (let ((o (make-overlay 2 3)) (p (make-overlay 1 11)) (q (make-overlay 4 8))) (transpose-regions 1 3 7 11) (list (overlay-start o) (overlay-end o) (overlay-start p) (overlay-end p) (overlay-start q) (overlay-end q))))
(with-temp-buffer (insert "αβ-xyzw") (let ((o (make-overlay 2 6))) (transpose-regions 1 3 4 8) (list (overlay-start o) (overlay-end o))))
(with-temp-buffer (buffer-enable-undo) (insert "abcXYZdefg") (undo-boundary) (setq buffer-undo-list nil) (goto-char 1) (transpose-regions 1 3 7 11) (format "%S" buffer-undo-list))
(with-temp-buffer (buffer-enable-undo) (insert "abcXYZdef") (undo-boundary) (setq buffer-undo-list nil) (goto-char 1) (transpose-regions 1 4 7 10) (format "%S" buffer-undo-list))
(with-temp-buffer (buffer-enable-undo) (insert "abcdef") (undo-boundary) (setq buffer-undo-list nil) (goto-char 1) (transpose-regions 1 4 4 7) (format "%S" buffer-undo-list))
(with-temp-buffer (insert "abcXYZdefg") (let (log) (set (make-local-variable 'before-change-functions) (list (lambda (b e) (push (list 'before b e) log)))) (set (make-local-variable 'after-change-functions) (list (lambda (b e l) (push (list 'after b e l (buffer-string)) log)))) (transpose-regions 2 3 7 9) (nreverse log)))
(with-temp-buffer (insert "abcdef") (set-buffer-modified-p nil) (transpose-regions 1 2 5 7) (list (buffer-modified-p) (buffer-string)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "abcXYZdefg") (let ((m (copy-marker 2))) (goto-char 9) (transpose-regions 1 3 7 11) (list (buffer-string) (point) (marker-position m) (multibyte-string-p (buffer-string)))))
(with-temp-buffer (insert "abc") (let ((m (make-marker))) (condition-case e (transpose-regions m 2 2 3) (error e))))
(with-temp-buffer (insert "abcdef") (list (transpose-regions (copy-marker 1) (copy-marker 3) 4 (copy-marker 6)) (buffer-string)))
(with-temp-buffer (insert "abc") (condition-case e (transpose-regions 'x 2 2 3) (error e)))
(list (commandp 'transpose-regions) (func-arity 'transpose-regions) (format "%S" (interactive-form 'transpose-regions)))
(with-temp-buffer (set-buffer-multibyte nil) (buffer-enable-undo) (insert "abcd") (put-text-property 1 3 'p 1) (undo-boundary) (setq buffer-undo-list nil) (delete-region 1 3) (format "%S" buffer-undo-list))
(with-temp-buffer (set-buffer-multibyte nil) (buffer-enable-undo) (insert "abcd") (put-text-property 1 3 'p 1) (undo-boundary) (setq buffer-undo-list nil) (transpose-regions 1 3 3 5) (format "%S" (list buffer-undo-list (buffer-substring-no-properties 1 5) (text-properties-at 1) (text-properties-at 3))))
(with-temp-buffer (buffer-enable-undo) (insert (propertize "ab" 'face 'bold) "--" (propertize "xyz" 'p 1 'q 2)) (undo-boundary) (setq buffer-undo-list nil) (goto-char 1) (transpose-regions 1 3 5 8) (format "%S" buffer-undo-list))
(with-temp-buffer (buffer-enable-undo) (insert (propertize "ab" 'face 'bold) "--" (propertize "xy" 'p 1)) (undo-boundary) (setq buffer-undo-list nil) (goto-char 1) (transpose-regions 1 3 5 7) (format "%S" buffer-undo-list))
(with-temp-buffer (insert "abcdef") (condition-case e (transpose-regions 99 1 3 4) (error e)))
(with-temp-buffer (insert "abcdef") (condition-case e (transpose-regions 1 2 (copy-marker 4) (* 4 most-positive-fixnum)) (error e)))
(with-temp-buffer (buffer-enable-undo) (insert (propertize "ab" 'face 'bold) "--" (propertize "xy" 'p 1)) (undo-boundary) (setq buffer-undo-list nil) (goto-char 1) (transpose-regions 5 7 1 3) (format "%S" buffer-undo-list))
(with-temp-buffer (buffer-enable-undo) (insert (propertize "ab" 'face 'bold) "--" (propertize "xyz" 'p 1)) (undo-boundary) (setq buffer-undo-list nil) (goto-char 1) (transpose-regions 5 8 1 3) (format "%S" (list buffer-undo-list (buffer-substring-no-properties 1 8) (mapcar (lambda (p) (text-properties-at p)) '(1 2 3 4 5 6 7)))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "x" (propertize "ab" 'p 1) "c") (list (get-text-property 1 'p) (get-text-property 2 'p) (get-text-property 3 'p) (get-text-property 4 'p)))
(with-temp-buffer (set-buffer-multibyte nil) (insert-and-inherit (propertize "a" 'p 1) "b" ?c) (list (buffer-substring-no-properties 1 4) (get-text-property 1 'p) (get-text-property 2 'p) (get-text-property 3 'p)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "x") (put-text-property 1 2 'q 2) (goto-char 2) (insert-and-inherit (propertize "a" 'p 1 'q 3)) (goto-char 1) (insert-and-inherit "y") (list (buffer-substring-no-properties 1 4) (text-properties-at 1) (text-properties-at 2) (text-properties-at 3)))
(with-temp-buffer (set-buffer-multibyte nil) (let ((m (copy-marker 1)) (k (copy-marker 1 t))) (insert-before-markers (propertize "ab" 'p 1) "c") (list (marker-position m) (marker-position k) (get-text-property 1 'p) (get-text-property 2 'p) (get-text-property 3 'p))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "ab") (put-text-property 1 2 'p 1) (narrow-to-region 2 3) (goto-char (point-min)) (insert-and-inherit "x") (widen) (list (buffer-substring-no-properties 1 4) (get-text-property 2 'p)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "ab") (put-text-property 2 3 'q 1) (put-text-property 2 3 'front-sticky t) (narrow-to-region 1 2) (goto-char (point-max)) (insert-and-inherit "x") (widen) (list (buffer-substring-no-properties 1 4) (get-text-property 2 'q)))
(with-temp-buffer (insert "ab") (put-text-property 1 2 'p 1) (narrow-to-region 2 3) (goto-char (point-min)) (insert-and-inherit "x") (widen) (list (buffer-substring-no-properties 1 4) (get-text-property 2 'p)))
(with-temp-buffer (insert "ab") (put-text-property 2 3 'q 1) (put-text-property 2 3 'front-sticky t) (narrow-to-region 1 2) (goto-char (point-max)) (insert-and-inherit "x") (widen) (list (buffer-substring-no-properties 1 4) (get-text-property 2 'q)))
(with-temp-buffer (set-buffer-multibyte nil) (insert (propertize "a" 'p 1) "b") (delete-region 1 2) (goto-char (point-max)) (list (condition-case e (let ((default-text-properties '(read-only t))) (insert "c") (buffer-substring-no-properties 1 (point-max))) (error (car e)))))
"###;

#[test]
fn transpose_regions_matches_pinned_gnu() {
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

/// After a multibyte transposition, with and without LEAVE-MARKERS, every
/// position's byte is its character's: position-bytes agrees with
/// string-bytes of the text before it.
#[test]
fn multibyte_positions_stay_coherent() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    for leave in ["nil", "t"] {
        let check = format!("(with-temp-buffer (insert \"αβ-xyzw\")
            (let ((ms (mapcar #'copy-marker '(1 2 3 4 5 6 7 8))) (o (make-overlay 2 6)))
              (goto-char 2) (transpose-regions 1 3 4 8 {leave})
              (let ((ok t))
                (dolist (p (append (list (point) (overlay-start o) (overlay-end o)) (mapcar #'marker-position ms)) ok)
                  (unless (= (position-bytes p) (1+ (string-bytes (buffer-substring 1 p)))) (setq ok (list 'bad p)))))))");
        assert_eq!(vm.eval_source(&check).and_then(|v| v.display()).as_deref(), Ok("t"), "LEAVE-MARKERS {leave}");
    }
}
