use romacs_vm::Vm;

/// skip_chars (syntax.c:1666) reads `[:NAME:]` with re_wctype_parse and
/// tests each character with re_iswctype: classes combine with characters
/// and ranges, `^' negates them too, `[:space:]' and `[:word:]' read the
/// buffer's syntax table, an unknown name signals, and `[:' with no `:]'
/// is two ordinary characters.
#[test]
fn skip_chars_iso_classes_match_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = r#"(let ((b (get-buffer-create "skip-classes")) (r nil))
  (set-buffer b)
  (insert "abc123 \t  def;;,é Ω!_x[:y")
  (goto-char 1)
  (setq r (list (skip-chars-forward "[:alpha:]") (skip-chars-forward "[:digit:]") (skip-chars-forward "[:space:]")
                (skip-chars-forward "^[:punct:]") (skip-chars-forward "[:punct:]") (skip-chars-forward "[:alpha:][:space:]")
                (skip-chars-forward "[:punct:]") (point)
                (progn (goto-char (point-max)) (skip-chars-backward "[:alpha:]:["))
                (progn (goto-char 1) (skip-chars-forward "a-c[:digit:]"))
                (progn (goto-char 1) (skip-chars-forward "^[:space:]"))
                (progn (goto-char 1) (skip-chars-forward "[:ascii:]"))
                (progn (goto-char 1) (skip-chars-forward "[:xdigit:][:upper:][:lower:]"))
                (condition-case e (skip-chars-forward "[:foo:]") (error e))
                (progn (goto-char 1) (skip-chars-forward "[:a"))))
  (let ((table (copy-syntax-table (standard-syntax-table))))
    (modify-syntax-entry ?_ "w" table)
    (modify-syntax-entry ?\; " " table)
    (set-syntax-table table)
    (goto-char (- (point-max) 6))
    (setq r (append r (list (skip-chars-forward "[:word:]") (progn (goto-char 14) (skip-chars-forward "[:space:]"))))))
  (erase-buffer)
  (set-buffer-multibyte nil)
  (insert "\351ab1 ")
  (goto-char 1)
  (append r (list (skip-chars-forward "[:alpha:]") (skip-chars-forward "[:alnum:]") (skip-chars-forward "[:nonascii:]"))))"#;
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {expression}))")])
        .stdin(std::process::Stdio::null())
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let mine = vm.eval_source(&format!("(prin1-to-string {expression})")).unwrap().display().unwrap();
    assert_eq!(mine, format!("{gnu:?}"));
}

/// [:upper:] and [:lower:] are regex-emacs.c's ISUPPER and ISLOWER, buffer.h
/// uppercasep and lowercasep over the buffer's case tables: a character is
/// upper when its downcase differs, lower when it is not upper and its
/// upcase differs.
#[test]
fn skip_chars_case_classes_read_the_buffers_case_tables() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = r#"(let ((b (get-buffer-create "skip-case-classes")) (r nil))
  (set-buffer b)
  (insert "ABab xy")
  (goto-char 1)
  (setq r (list (skip-chars-forward "[:upper:]") (skip-chars-forward "[:lower:]")))
  (let ((d (copy-sequence (standard-case-table))))
    (aset d ?A ?A)
    (aset d ?x ?y)
    (set-case-table d)
    (goto-char 1)
    (setq r (append r (list (skip-chars-forward "[:upper:]") (skip-chars-forward "[:lower:]")
                            (progn (goto-char 2) (skip-chars-forward "[:upper:]"))
                            (progn (goto-char 6) (list (skip-chars-forward "[:upper:]") (skip-chars-forward "[:lower:]")))
                            (progn (goto-char 1) (skip-chars-forward "^[:upper:]"))))))
  r)"#;
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {expression}))")])
        .stdin(std::process::Stdio::null())
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let mine = vm.eval_source(&format!("(prin1-to-string {expression})")).unwrap().display().unwrap();
    assert_eq!(mine, format!("{gnu:?}"));
}
