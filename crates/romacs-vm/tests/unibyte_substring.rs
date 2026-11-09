//! Text taken out of a unibyte buffer is a unibyte string, its positions
//! bytes, carrying the text's properties (editfns.c make_buffer_string_both,
//! copy_intervals_to_string), and such a string inserts back with them
//! (insdel.c insert_from_string_1, graft_intervals_into_buffer), a byte
//! above 127 becoming its raw-byte character in a multibyte buffer.
//! Against pinned GNU in batch.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(with-temp-buffer (set-buffer-multibyte nil) (insert "ab\377cd") (put-text-property 2 4 'face 'bold) (let ((s (buffer-substring 1 6))) (list s (multibyte-string-p s) (text-properties-at 1 s) (text-properties-at 3 s))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "ab\377cd") (put-text-property 2 4 'face 'bold) (let ((s (buffer-substring 3 6))) (list s (multibyte-string-p s) (length s) (text-properties-at 0 s) (text-properties-at 1 s))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "ab\377cd") (put-text-property 2 4 'face 'bold) (list (buffer-substring 6 3) (buffer-substring 5 6) (buffer-substring 2 2)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "ab\377cd") (put-text-property 2 4 'face 'bold) (let ((s (buffer-substring-no-properties 1 6))) (list s (multibyte-string-p s) (text-properties-at 1 s))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "abcd") (put-text-property 2 4 'face 'bold) (let ((s (buffer-substring 1 5))) (list s (multibyte-string-p s))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "a\377\200b") (put-text-property 1 5 'p 1) (put-text-property 3 4 'q 2) (list (buffer-string) (multibyte-string-p (buffer-string))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "ab\377cd") (put-text-property 2 4 'face 'bold) (narrow-to-region 2 5) (list (buffer-substring 2 5) (condition-case e (buffer-substring 1 5) (error (car e)))))
(with-temp-buffer (set-buffer-multibyte nil) (insert (propertize (string-to-unibyte "a\377b") 'p 1)) (list (buffer-string) (point) (get-text-property 2 'p) (char-after 2)))
(with-temp-buffer (insert "<" (propertize (string-to-unibyte "a\377b") 'p 1) ">") (list (buffer-string) (multibyte-string-p (buffer-string)) (get-text-property 3 'p) (char-after 3) (position-bytes 4)))
(let ((src (get-buffer-create " romacs-unibyte-src"))) (with-current-buffer src (set-buffer-multibyte nil) (insert "ab\377cd") (put-text-property 2 4 'p 1)) (prog1 (with-temp-buffer (insert-buffer-substring src 2 5) (list (buffer-string) (char-after 2) (get-text-property 1 'p) (get-text-property 3 'p))) (kill-buffer src)))
(let ((s (with-temp-buffer (set-buffer-multibyte nil) (insert "x\377y") (put-text-property 1 3 'p 1) (buffer-substring 1 4)))) (with-temp-buffer (set-buffer-multibyte nil) (insert s s) (list (buffer-string) (get-text-property 4 'p) (get-text-property 6 'p))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "ab\377cd") (put-text-property 2 4 'p 1) (let ((s (delete-and-extract-region 2 5))) (list s (multibyte-string-p s) (buffer-string))))
"###;

#[test]
fn unibyte_buffer_text_keeps_its_properties_as_pinned_gnu() {
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
