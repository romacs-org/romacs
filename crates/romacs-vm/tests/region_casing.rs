use romacs_vm::Vm;

/// casify_region (casefiddle.c) for downcase-region and capitalize-region
/// beside upcase-region and upcase-initials-region: the changed text, point,
/// the buffer's modified state, the change hooks' arguments, the interactive
/// spec, and a unibyte buffer's bytes.
#[test]
fn region_casing_commands_match_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = r#"(let ((b (get-buffer-create "region-casing")) (log nil) (out nil))
  (set-buffer b)
  (dolist (f '(upcase-region downcase-region capitalize-region upcase-initials-region))
    (erase-buffer)
    (insert "hELLO wORLD-foo ΣΟΦΊΑ ǆemal straße 'quoted x1y")
    (set-buffer-modified-p nil)
    (setq log nil)
    (let ((after-change-functions (list (lambda (b e l) (setq log (cons (list b e l) log))))))
      (goto-char 3)
      (funcall f 2 (- (point-max) 2)))
    (setq out (cons (list f (buffer-string) (point) (buffer-modified-p) log (interactive-form f)) out)))
  (erase-buffer)
  (set-buffer-multibyte nil)
  (insert "abc DEF \351t\351")
  (downcase-region 1 (point-max))
  (setq out (cons (append (buffer-string) nil) out))
  (capitalize-region 1 (point-max))
  (setq out (cons (append (buffer-string) nil) out))
  (setq out (cons (condition-case e (capitalize-region 1 'x) (error e)) out))
  (nreverse out))"#;
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

/// A case table whose upcase maps capital sigma to small sigma: upcase-region
/// still gives a word-final changed sigma its final form, as GNU's
/// case_character does in either direction.
#[test]
fn region_casing_with_a_sigma_table_matches_gnu() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expression = "(with-temp-buffer (let* ((up (char-table-extra-slot (current-case-table) 0)) (old (aref up 931))) \
        (unwind-protect (progn (aset up 931 963) (insert (string ?A 931 32 ?x 931 ?y 32 931)) (upcase-region 1 (point-max)) \
          (list (append (buffer-string) nil) (progn (erase-buffer) (insert (string ?a 931 ?b)) (downcase-region 1 (point-max)) (append (buffer-string) nil)))) \
        (aset up 931 old))))";
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

/// What the buffer cannot hold: a table casing 中 to a surrogate. The edit
/// loop cases the characters before it, then refuses (pinned GNU stores the
/// surrogate; Romacs's text cannot).
#[test]
fn region_casing_to_an_unstorable_character_refuses_where_it_meets_it() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let got = vm.eval_source("(prin1-to-string (with-temp-buffer (let* ((up (char-table-extra-slot (current-case-table) 0)) (old (aref up 20013))) \
        (unwind-protect (progn (aset up 20013 55296) (insert (string ?a ?b 20013 ?c ?d)) \
          (list (condition-case e (upcase-region 1 (point-max)) (error (car e))) (append (buffer-string) nil))) \
        (aset up 20013 old)))))").unwrap().display().unwrap();
    assert_eq!(got, "\"(error (65 66 20013 99 100))\"");
}

/// A before-change hook that empties an undo-disabled buffer leaves an empty
/// range: the casing changes nothing and signals nothing.
#[test]
fn region_casing_of_a_range_a_hook_emptied_changes_nothing() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let got = vm.eval_source("(prin1-to-string (with-temp-buffer (buffer-disable-undo) (insert \"hello world\") \
        (let ((before-change-functions (list (lambda (b e) (delete-region 1 (point-max)))))) \
          (list (condition-case e (upcase-region 5 9) (error (car e))) (buffer-string)))))").unwrap().display().unwrap();
    assert_eq!(got, "\"(nil \\\"\\\")\"");
}
