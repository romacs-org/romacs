use romacs_vm::Vm;

#[test]
fn following_char_uses_unicode_and_the_accessible_end() {
    let mut vm = Vm::new();
    let actual = vm.eval_source(r#"(progn (insert "λ!") (goto-char 1)
      (setq first (following-char)) (narrow-to-region 1 2) (goto-char 2)
      (setq bounded (following-char) bounded-point (point)) (widen)
      (list first bounded bounded-point (buffer-string)))"#).unwrap().display().unwrap();
    assert_eq!(actual, "(955 0 2 \"λ!\")");
}

#[test]
fn following_char_preserves_high_octets_in_byte_buffers() {
    let mut vm = Vm::new();
    let actual = vm.eval_source(r#"(progn (set-buffer-multibyte nil) (insert 255)
      (goto-char 1) (setq first (following-char)) (goto-char 2)
      (list first (following-char) (point)))"#).unwrap().display().unwrap();
    assert_eq!(actual, "(255 0 2)");
}

#[test]
fn pristine_gnu_header_extraction_preserves_folding_duplicates_and_point() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let source = include_str!("../../../gnu-lisp/mail/mailheader.el");
    let end = source.find("(defun mail-header-extract-no-properties").unwrap();
    vm.eval_source(&source[..end]).unwrap();
    let actual = vm.eval_source(r#"(with-temp-buffer
      (insert "From: Alice <alice@example.invalid>\nTo: Bob <bob@example.invalid>\nSubject: Folded\n\tΕλληνικά\nX-Trace: first\nX-Trace: second\n\n")
      (goto-char (point-min))
      (list (mail-header-extract) (point)))"#)
        .and_then(|value| value.display()).unwrap_or_else(|error| format!("VM-ERROR: {error}"));
    assert_eq!(actual, r#"(((from . "Alice <alice@example.invalid>") (to . "Bob <bob@example.invalid>") (subject . "Folded Ελληνικά") (x-trace . "first") (x-trace . "second")) 1)"#, "UNCHANGED_GNU_HEADER_PARSER_MUST_RUN_ON_ROMACS");
}
