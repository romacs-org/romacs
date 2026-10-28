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
