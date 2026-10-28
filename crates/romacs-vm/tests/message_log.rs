//! GNU xdisp.c message_dolog: `message' logs to *Messages* in a bare VM.
use romacs_vm::Vm;

fn eval(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(error) => format!("ERROR: {error}"),
    }
}

fn log(vm: &mut Vm) -> String {
    eval(vm, "(with-current-buffer \"*Messages*\" (save-restriction (widen) (buffer-substring-no-properties (point-min) (point-max))))")
}

#[test]
fn message_logs_and_collapses_repeats() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    assert_eq!(eval(&mut vm, "(get-buffer \"*Messages*\")"), "nil");
    assert_eq!(eval(&mut vm, "(message \"a %d\" 1)"), "\"a 1\"");
    assert_eq!(log(&mut vm), "\"a 1\\n\"");
    for _ in 0..3 { eval(&mut vm, "(message \"b\")"); }
    assert_eq!(log(&mut vm), "\"a 1\\nb [3 times]\\n\"");
    eval(&mut vm, "(message \"Loading x...\")");
    eval(&mut vm, "(message \"Loading x...done\")");
    assert_eq!(log(&mut vm), "\"a 1\\nb [3 times]\\nLoading x...done\\n\"");
    // nil and "" clear the echo area without logging.
    assert_eq!(eval(&mut vm, "(list (message nil) (message \"\"))"), "(nil \"\")");
    assert_eq!(log(&mut vm), "\"a 1\\nb [3 times]\\nLoading x...done\\n\"");
    assert_eq!(eval(&mut vm, "(with-current-buffer \"*Messages*\" (list buffer-undo-list (buffer-modified-p)))"), "(t t)");
}

#[test]
fn message_log_max_and_inhibit_message() {
    let mut vm = Vm::new();
    eval(&mut vm, "(let ((message-log-max nil)) (message \"unlogged\"))");
    assert_eq!(eval(&mut vm, "(get-buffer \"*Messages*\")"), "nil");
    eval(&mut vm, "(let ((message-log-max 2)) (message \"1\") (message \"2\") (message \"3\"))");
    assert_eq!(log(&mut vm), "\"2\\n3\\n\"");
    eval(&mut vm, "(let ((message-log-max t)) (message \"4\") (message \"5\"))");
    assert_eq!(log(&mut vm), "\"2\\n3\\n4\\n5\\n\"");
    eval(&mut vm, "(let ((message-log-max 0)) (message \"6\"))");
    assert_eq!(log(&mut vm), "\"\"");
    // inhibit-message still logs, and leaves the echo area alone.
    assert_eq!(eval(&mut vm, "(progn (setq noninteractive nil) (message \"shown\") (let ((inhibit-message t)) (message \"hidden\")) (current-message))"), "\"shown\"");
    assert_eq!(log(&mut vm), "\"shown\\nhidden\\n\"");
    eval(&mut vm, "(setq messages-buffer-name \"*Other log*\")");
    eval(&mut vm, "(message \"elsewhere\")");
    assert_eq!(eval(&mut vm, "(with-current-buffer \"*Other log*\" (buffer-string))"), "\"elsewhere\\n\"");
}

#[test]
fn message_log_keeps_point_narrowing_and_current_buffer() {
    let mut vm = Vm::new();
    eval(&mut vm, "(message \"one\") (message \"two\")");
    assert_eq!(eval(&mut vm, "(progn (set-buffer (get-buffer-create \"user\")) (message \"three\") (buffer-name))"), "\"user\"");
    // Point inside the log stays; point at its end follows new text.
    assert_eq!(eval(&mut vm, "(with-current-buffer \"*Messages*\" (goto-char 2) (message \"four\") (point))"), "2");
    assert_eq!(eval(&mut vm, "(with-current-buffer \"*Messages*\" (goto-char (point-max)) (message \"five\") (= (point) (point-max)))"), "t");
    assert_eq!(eval(&mut vm, "(with-current-buffer \"*Messages*\" (narrow-to-region 1 4) (message \"six\") (list (point-min) (point-max)))"), "(1 4)");
    assert_eq!(eval(&mut vm, "(with-current-buffer \"*Messages*\" (widen) (narrow-to-region 5 (point-max)) (message \"seven\") (list (point-min) (= (point-max) (1+ (buffer-size)))))"), "(5 t)");
    // A read-only log with change hooks still logs, without running them.
    assert_eq!(eval(&mut vm, "(with-current-buffer \"*Messages*\" (widen) (setq buffer-read-only t) (let ((ran nil)) (set (make-local-variable 'after-change-functions) (list (lambda (&rest _) (setq ran t)))) (message \"eight\") (list ran (buffer-substring-no-properties (- (point-max) 6) (point-max)))))"), "(nil \"eight\\n\")");
}

#[test]
fn message_creates_log_in_messages_buffer_mode_and_signals_format_errors() {
    let mut vm = Vm::new();
    eval(&mut vm, "(defun messages-buffer-mode () (setq major-mode 'messages-buffer-mode buffer-read-only t))");
    eval(&mut vm, "(message \"x\")");
    assert_eq!(eval(&mut vm, "(with-current-buffer \"*Messages*\" (list major-mode buffer-read-only (buffer-string)))"), "(messages-buffer-mode t \"x\\n\")");
    // Fmessage formats with Fformat_message, so it signals what format does.
    for args in ["1", "\"%d\" 'a", "\"%s\""] {
        assert_eq!(eval(&mut vm, &format!("(equal (condition-case e (message {args}) (error e)) (condition-case e (format {args}) (error e)))")), "t", "{args}");
    }
}

/// message_log_check_duplicate parses " [N times]" in C, so a repeated
/// message leaves the caller's match data alone.
#[test]
fn repeated_message_keeps_match_data() {
    let mut vm = Vm::new();
    let source = "(progn (message \"same\") (message \"same\") (string-match \"b\" \"abc\") (message \"same\") \
                  (list (match-data) (with-current-buffer \"*Messages*\" (buffer-string))))";
    assert_eq!(eval(&mut vm, source), "((1 2) \"same [3 times]\\n\")");
}
