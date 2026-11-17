use romacs_vm::Vm;

fn check(source: &str, expected: &str, name: &str) {
    let mut vm = Vm::new();
    let actual = vm.eval_source(source).and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok(expected), "{name}: {source}");
}

#[test]
fn bolp_accessible_beginning_and_character_positions() {
    check("(with-temp-buffer (insert \"é🙂a\\nЖb\") (list (progn (goto-char 1) (bolp)) (progn (goto-char 3) (bolp)) (progn (goto-char 5) (bolp)) (progn (narrow-to-region 3 6) (goto-char 3) (bolp))))", "(t nil t t)", "BOLP_ACCESSIBLE_CHARACTER_BOUNDARY");
}

#[test]
fn eolp_accessible_end_and_character_positions() {
    check("(with-temp-buffer (insert \"é🙂a\\nЖb\") (list (progn (goto-char 4) (eolp)) (progn (goto-char 3) (eolp)) (progn (goto-char 7) (eolp)) (progn (narrow-to-region 2 3) (goto-char 3) (eolp))))", "(t nil t t)", "EOLP_ACCESSIBLE_CHARACTER_BOUNDARY");
}

#[test]
fn bobp_uses_accessible_not_absolute_beginning() {
    check("(with-temp-buffer (insert \"é🙂abc\") (list (progn (goto-char 1) (bobp)) (progn (goto-char 2) (bobp)) (progn (narrow-to-region 3 5) (goto-char 3) (bobp)) (progn (goto-char 4) (bobp))))", "(t nil t nil)", "BOBP_ACCESSIBLE_BEGINNING");
}

#[test]
fn eobp_uses_accessible_not_absolute_end() {
    check("(with-temp-buffer (insert \"é🙂abc\") (list (progn (goto-char 6) (eobp)) (progn (goto-char 2) (eobp)) (progn (narrow-to-region 2 4) (goto-char 4) (eobp)) (progn (goto-char 3) (eobp))))", "(t nil t nil)", "EOBP_ACCESSIBLE_END");
}

#[test]
fn empty_and_empty_restriction() {
    check("(with-temp-buffer (list (bolp) (eolp) (bobp) (eobp)))", "(t t t t)", "EMPTY_BUFFER");
    check("(with-temp-buffer (insert \"abc\") (narrow-to-region 2 2) (list (bolp) (eolp) (bobp) (eobp)))", "(t t t t)", "EMPTY_RESTRICTION");
}

#[test]
fn unibyte_predicates() {
    check("(with-temp-buffer (set-buffer-multibyte nil) (insert (unibyte-string 255 10 254)) (goto-char 3) (list (bolp) (eolp) (bobp) (eobp) (progn (narrow-to-region 2 3) (eolp)) (eobp)))", "(t nil nil nil t t)", "UNIBYTE_BOUNDARIES");
}

#[test]
fn callable_and_arity() {
    for name in ["bolp", "eolp", "bobp", "eobp"] {
        check(&format!("(with-temp-buffer (list (fboundp '{name}) (functionp #'{name}) (funcall #'{name})))"), "(t t t)", "CALLABLE");
        check(&format!("(condition-case c (apply #'{name} '(1)) (wrong-number-of-arguments (car c)))"), "wrong-number-of-arguments", "ZERO_ARITY");
    }
}

/// SOURCE prints as pinned GNU prints it.
fn agree(source: &str) {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&oracle).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
    assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let actual = Vm::new().eval_source(source).and_then(romacs_vm::Local::display);
    assert_eq!(actual.as_deref(), Ok(expected.as_str()), "{source}");
}

#[test]
fn predicates_at_narrowed_ends_agree_with_gnu() {
    agree("(with-temp-buffer (insert \"é\\nab\\nЖ\\ncd\") (narrow-to-region 3 8) \
           (let ((p 3) out) (while (<= p 8) (goto-char p) (setq out (cons (list p (bobp) (bolp) (eolp) (eobp)) out) p (1+ p))) (nreverse out)))");
}

#[test]
fn unibyte_predicates_at_narrowed_ends_agree_with_gnu() {
    agree("(with-temp-buffer (set-buffer-multibyte nil) (insert (unibyte-string 255 10 254 10 200)) (narrow-to-region 2 5) \
           (let ((p 2) out) (while (<= p 5) (goto-char p) (setq out (cons (list p (bobp) (bolp) (eolp) (eobp)) out) p (1+ p))) (nreverse out)))");
}

/// An indirect buffer has its own point and narrowing over its base's text.
#[test]
fn indirect_buffer_predicates_agree_with_gnu() {
    agree("(with-temp-buffer (insert \"ab\\ncd\\nef\") (goto-char 2) \
           (let ((ind (make-indirect-buffer (current-buffer) \"ind\"))) \
             (unwind-protect \
                 (list (with-current-buffer ind (narrow-to-region 4 7) (goto-char 7) (list (point) (bobp) (bolp) (eolp) (eobp))) \
                       (list (point) (bobp) (bolp) (eolp) (eobp)) \
                       (with-current-buffer ind (goto-char 4) (list (point) (bobp) (bolp) (eolp) (eobp)))) \
               (kill-buffer ind))))");
}

/// Selecting a killed buffer signals; killing the current buffer makes
/// another current, whose bounds the predicates then read.
#[test]
fn killed_buffers_agree_with_gnu() {
    agree("(let ((b (get-buffer-create \"point-predicates-dead\"))) (kill-buffer b) \
           (list (condition-case e (with-current-buffer b (eobp)) (error e)) \
                 (condition-case e (set-buffer b) (error e)) \
                 (condition-case e (save-current-buffer (set-buffer b) (bobp)) (error e))))");
    agree("(let ((k (get-buffer-create \"point-predicates-killed\"))) (with-current-buffer k (insert \"x\") (kill-buffer k) \
           (list (buffer-live-p (current-buffer)) (eq (current-buffer) k) (eobp) (bobp))))");
}

/// Switching buffers back and forth reads each one's own point.
#[test]
fn switching_buffers_agrees_with_gnu() {
    agree("(let ((a (get-buffer-create \"point-predicates-a\")) (b (get-buffer-create \"point-predicates-b\"))) (with-current-buffer a (insert \"xy\")) \
           (prog1 (list (progn (set-buffer a) (goto-char 1) (eobp)) (progn (set-buffer b) (eobp)) (progn (set-buffer a) (eobp)) \
                        (with-current-buffer b (insert \"z\") (bobp)) (eobp)) \
             (kill-buffer a) (kill-buffer b)))");
}
