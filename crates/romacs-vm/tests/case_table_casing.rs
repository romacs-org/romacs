use romacs_vm::Vm;

/// casefiddle.c case_character reads the current buffer's case tables for
/// strings, characters and regions alike: a nil entry is identity
/// (downcase/upcase in buffer.h), capitalize's in-word characters take the
/// downcase table, and a word start without a titlecase property takes
/// the upcase table.
#[test]
fn casing_honours_custom_case_tables_as_gnu_does() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = r#"(let ((b (get-buffer-create "case-tables")) (out nil))
  (set-buffer b)
  (let ((d (copy-sequence (standard-case-table))))
    (aset d ?A nil)
    (set-case-table d)
    (erase-buffer) (insert "AB ab")
    (downcase-region 1 (point-max))
    (setq out (list (buffer-string) (downcase "AB") (downcase ?A) (capitalize "xA AB"))))
  (let ((d (copy-sequence (standard-case-table))))
    (aset d ?A ?z)
    (set-case-table d)
    (erase-buffer) (insert "AA AA")
    (capitalize-region 1 (point-max))
    (setq out (append out (list (buffer-string) (capitalize "AA AA") (downcase "AA") (upcase-initials "aA")))))
  (let ((d (make-char-table 'case-table)) (u (make-char-table 'case-table)))
    (aset d 931 120) (aset u ?q ?W)
    (set-char-table-extra-slot d 0 u)
    (set-char-table-extra-slot d 1 (make-char-table 'case-table))
    (set-case-table d)
    (erase-buffer) (insert "qq AQ")
    (capitalize-region 1 (point-max))
    (setq out (append out (list (buffer-string) (downcase "AΣ") (upcase "qa") (capitalize "qq") (capitalize ?q)))))
  out)"#;
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

/// case_character turns a downcased capital sigma at a word's end into
/// final sigma only when the downcase table changed it: with an empty
/// table (nil entries) or an explicit identity entry, capitalize and
/// capitalize-region leave the sigma alone.
#[test]
fn capitalize_final_sigma_follows_the_downcase_table() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = r#"(let ((b (get-buffer-create "case-sigma")) (out nil))
  (set-buffer b)
  (dolist (identity '(nil t))
    (let ((d (make-char-table 'case-table)))
      (when identity (aset d 931 931))
      (set-char-table-extra-slot d 0 (make-char-table 'case-table))
      (set-char-table-extra-slot d 1 (make-char-table 'case-table))
      (set-case-table d)
      (erase-buffer) (insert "AΣ ΑΣΣ")
      (capitalize-region 1 (point-max))
      (setq out (append out (list (capitalize "AΣ") (capitalize "ΑΣΣ x") (buffer-string))))))
  (set-case-table (standard-case-table))
  (erase-buffer) (insert "AΣ ΑΣΣ")
  (capitalize-region 1 (point-max))
  (append out (list (capitalize "AΣ") (buffer-string))))"#;
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
