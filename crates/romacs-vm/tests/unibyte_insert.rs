//! Multibyte text inserted into a unibyte buffer becomes bytes, each
//! character its low byte (insdel.c copy_text, CHAR_TO_BYTE8): ASCII as
//! itself, a raw byte's character as that byte, Latin-1 and other
//! characters as their code's low byte.  Against pinned GNU in batch;
//! process output into a unibyte buffer is in tests/process.rs.
use romacs_vm::Vm;

/// One expression per line, each in a unibyte buffer.
const CASES: &str = r###"
(with-temp-buffer (set-buffer-multibyte nil) (insert "<" "abc" ">") (list (buffer-string) (point) (buffer-size)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "<" "é" ">") (list (buffer-string) (point) (buffer-size)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "<" (string-to-multibyte "\351\377") ">") (list (buffer-string) (point) (buffer-size)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "<" "α中" ">") (list (buffer-string) (point) (buffer-size)))
(with-temp-buffer (set-buffer-multibyte nil) (insert ?é #x3fffe9 ?α ?a) (list (buffer-string) (point) (buffer-size)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "x" (propertize "éαb" 'p 1) "y") (list (append (buffer-string) nil) (get-text-property 1 'p) (get-text-property 2 'p) (get-text-property 4 'p) (get-text-property 5 'p)))
(with-temp-buffer (set-buffer-multibyte nil) (insert (concat (propertize "é" 'p t) "a" (propertize (string-to-multibyte "\377") 'q t) "b")) (list (append (buffer-string) nil) (mapcar (lambda (i) (list (get-text-property i 'p) (get-text-property i 'q))) '(1 2 3 4))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "<>") (let ((m (copy-marker 2))) (goto-char 2) (insert-before-markers (concat "aé" (string-to-multibyte "\377") "αz") ?α) (list (buffer-string) (point) (marker-position m))))
(let ((src (get-buffer-create "romacs-src"))) (with-current-buffer src (insert "<aé" (string-to-multibyte "\377") "αz>")) (prog1 (with-temp-buffer (set-buffer-multibyte nil) (insert-buffer-substring src 2 6) (list (buffer-string) (point))) (kill-buffer src)))
(with-temp-buffer (set-buffer-multibyte nil) (buffer-enable-undo) (insert "abcd") (undo-boundary) (replace-region-contents 2 4 "éα" 0) (list (buffer-string) (point) buffer-undo-list))
(with-temp-buffer (set-buffer-multibyte nil) (insert "abcdefghij") (put-text-property 2 4 'p 1) (put-text-property 4 6 'p 2) (put-text-property 8 9 'q 3) (list (next-property-change 1) (next-property-change 2) (next-property-change 4) (next-property-change 6) (next-property-change 9) (previous-property-change 11) (previous-property-change 9) (previous-property-change 6) (previous-property-change 4) (next-single-property-change 1 'p) (next-single-property-change 2 'p) (next-single-property-change 6 'p) (previous-single-property-change 8 'p) (previous-single-property-change 5 'p) (next-single-char-property-change 1 'q) (next-single-char-property-change 8 'q) (previous-single-char-property-change 11 'q) (previous-single-char-property-change 9 'q) (next-property-change 1 nil 3) (next-single-property-change 1 'p nil 2)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "abcdefghij") (put-text-property 2 4 'p 1) (narrow-to-region 3 7) (list (next-property-change 3) (next-single-property-change 3 'p) (previous-single-property-change 7 'p) (previous-single-char-property-change 7 'p) (next-single-char-property-change 5 'p) (condition-case e (next-property-change 9) (error e))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "a") (put-text-property 1 2 'p t) (list (next-property-change (point-max)) (next-single-property-change (point-max) 'p) (previous-property-change 1)))
"###;

#[test]
fn multibyte_text_inserted_into_a_unibyte_buffer_becomes_bytes_as_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        let source = format!("(prin1-to-string (condition-case e {expression} (error (list 'signal e))))");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
