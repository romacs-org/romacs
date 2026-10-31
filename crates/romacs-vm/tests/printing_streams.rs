use romacs_vm::Vm;

fn cases() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("empty_readonly", "(with-temp-buffer (setq buffer-read-only t) (princ \"\" (current-buffer)))", "\"\""),
        ("unquoted_raw_bytes_to_text", "(with-temp-buffer (princ (unibyte-string 128 160 195 255 97) (current-buffer)) (list (point) (vconcat (buffer-string))))", "(18 [92 50 48 48 92 50 52 48 92 51 48 51 92 51 55 55 97])"),
        ("unquoted_raw_bytes_to_marker", "(with-temp-buffer (insert \"x\") (let ((m (copy-marker 1)) (text (unibyte-string 128 160 195 255 97))) (list (eq (princ text m) text) (point) (marker-position m) (vconcat (buffer-string)))))", "(t 19 18 [92 50 48 48 92 50 52 48 92 51 48 51 92 51 55 55 97 120])"),
        ("buffer_point_and_current", "(let ((outer (current-buffer)) (b (get-buffer-create \"print-stream\"))) (unwind-protect (progn (with-current-buffer b (insert \"ab\") (goto-char 2)) (list (princ \"λ\" b) (eq (current-buffer) outer) (with-current-buffer b (list (point) (buffer-string))))) (kill-buffer b)))", "(\"λ\" t (3 \"aλb\"))"),
        ("standard_output_buffer", "(with-temp-buffer (let ((standard-output (current-buffer))) (princ \"out\")) (list (point) (buffer-string)))", "(4 \"out\")"),
        ("all_entries", "(with-temp-buffer (let ((b (current-buffer))) (list (prin1 '(a \"λ\") b) (print 7 b) (terpri b) (write-char 955 b) (point) (buffer-string))))", "((a \"λ\") 7 t 955 13 \"(a \\\"λ\\\")\\n7\\n\\nλ\")"),
        ("properties_not_transferred", "(with-temp-buffer (princ (propertize \"x\" 'face 'bold) (current-buffer)) (list (buffer-string) (get-text-property 1 'face)))", "(\"x\" nil)"),
        ("dead_buffer", "(let ((b (get-buffer-create \"dead-print-stream\"))) (kill-buffer b) (condition-case e (princ \"x\" b) (error e)))", "(error \"Selecting deleted buffer\")"),
        ("detached_marker", "(condition-case e (princ \"x\" (make-marker)) (error e))", "(error \"Marker does not point anywhere\")"),
        ("narrowed_marker", "(with-temp-buffer (insert \"abcd\") (let ((m (copy-marker 1))) (narrow-to-region 2 4) (goto-char 3) (list (condition-case e (princ \"x\" m) (error (car e))) (point) (marker-position m))))", "(error 3 1)"),
        ("readonly_restores_buffer", "(let ((outer (current-buffer)) (b (get-buffer-create \"readonly-print-stream\"))) (unwind-protect (progn (with-current-buffer b (insert \"abcd\") (goto-char 4) (setq buffer-read-only t)) (let ((m (with-current-buffer b (copy-marker 2)))) (list (condition-case e (princ \"x\" m) (error (car e))) (eq outer (current-buffer)) (with-current-buffer b (list (point) (buffer-string))) (marker-position m)))) (kill-buffer b)))", "(buffer-read-only t (2 \"abcd\") 2)"),
        ("unibyte", "(with-temp-buffer (set-buffer-multibyte nil) (let ((text (unibyte-string 255 97))) (list (eq (princ text (current-buffer)) text) (write-char 200 (current-buffer)) (point) (let ((s (buffer-string))) (list (aref s 0) (aref s 1) (aref s 2))))))", "(t 200 4 (255 97 200))"),
        ("quoted_unibyte_to_text", "(with-temp-buffer (prin1 (unibyte-string 255 97) (current-buffer)) (buffer-string))", "\"\\\"\\\\377a\\\"\""),
        ("quoted_text_to_unibyte", "(with-temp-buffer (set-buffer-multibyte nil) (prin1 \"λ\" (current-buffer)) (vconcat (buffer-string)))", "[34 92 120 48 51 98 98 34]"),
        ("marker_point_1_nil", "(with-temp-buffer (insert \"abcd\") (goto-char 1) (let ((m (copy-marker 2 nil))) (list (princ \"λЖ\" m) (point) (marker-position m) (buffer-string))))", "(\"λЖ\" 1 4 \"aλЖbcd\")"),
        ("marker_point_1_t", "(with-temp-buffer (insert \"abcd\") (goto-char 1) (let ((m (copy-marker 2 t))) (list (princ \"λЖ\" m) (point) (marker-position m) (buffer-string))))", "(\"λЖ\" 1 4 \"aλЖbcd\")"),
        ("marker_point_2_nil", "(with-temp-buffer (insert \"abcd\") (goto-char 2) (let ((m (copy-marker 2 nil))) (list (princ \"λЖ\" m) (point) (marker-position m) (buffer-string))))", "(\"λЖ\" 4 4 \"aλЖbcd\")"),
        ("marker_point_2_t", "(with-temp-buffer (insert \"abcd\") (goto-char 2) (let ((m (copy-marker 2 t))) (list (princ \"λЖ\" m) (point) (marker-position m) (buffer-string))))", "(\"λЖ\" 4 4 \"aλЖbcd\")"),
        ("marker_point_4_nil", "(with-temp-buffer (insert \"abcd\") (goto-char 4) (let ((m (copy-marker 2 nil))) (list (princ \"λЖ\" m) (point) (marker-position m) (buffer-string))))", "(\"λЖ\" 6 4 \"aλЖbcd\")"),
        ("marker_point_4_t", "(with-temp-buffer (insert \"abcd\") (goto-char 4) (let ((m (copy-marker 2 t))) (list (princ \"λЖ\" m) (point) (marker-position m) (buffer-string))))", "(\"λЖ\" 6 4 \"aλЖbcd\")"),
    ]
}

#[test]
fn printing_to_buffers_and_markers_matches_gnu() {
    for (name, source, expected) in cases() {
        let mut vm = Vm::new();
        assert_eq!(vm.eval_source(source).and_then(|v| v.display()).as_deref(), Ok(expected), "{name}: {source}");
    }
}

#[test]
fn printing_streams_pinned_gnu_control() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    for (name, source, expected) in cases() {
        let result = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(let ((print-escape-newlines t)) (prin1 {source}))")])
            .output().unwrap();
        assert!(result.status.success(), "{name}: {}", String::from_utf8_lossy(&result.stderr));
        assert_eq!(String::from_utf8(result.stdout).unwrap(), expected, "{name}");
    }
}

#[test]
fn printing_callbacks_survive_collection_and_errors() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let source = "(let ((out nil) (object (list \"λ\"))) (list (eq object (prin1 object (lambda (c) (garbage-collect) (setq out (cons c out))))) (nreverse out)))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(t (40 34 955 34 41))");
    let source = "(let ((calls 0)) (list (condition-case err (princ \"abc\" (lambda (c) (garbage-collect) (setq calls (1+ calls)) (if (= calls 2) (error \"output-stop\")))) (error (cdr err))) calls))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "((\"output-stop\") 2)");
    assert_eq!(vm.eval_source("(with-temp-buffer (princ \"ok\" (current-buffer)) (buffer-string))").unwrap().display().unwrap(), "\"ok\"");
}

#[test]
fn printing_errors_restore_current_buffer_and_keep_text_atomic() {
    let mut vm = Vm::new();
    let source = "(let ((outer (current-buffer)) (b (get-buffer-create \"print-error\"))) (unwind-protect (list (condition-case err (prin1 'object b '((unsupported . t))) (error (car err))) (eq outer (current-buffer)) (with-current-buffer b (buffer-string))) (kill-buffer b)))";
    // An unknown override key is print_bind_overrides's (wrong-type-argument symbolp nil), as GNU's.
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(wrong-type-argument t \"\")");
}

/// print.c: on a terminal, `t` is the echo area. Successive prints extend
/// the message printing started; `message` replaces it. Batch (or an
/// embedder that renders no echo area) prints to standard output.
#[test]
fn printing_to_t_reaches_the_echo_area_an_embedder_renders() {
    let mut vm = Vm::new();
    let ask = |vm: &mut Vm, source: &str| vm.eval_source(source).unwrap().display().unwrap();
    assert_eq!(ask(&mut vm, "(prin1 'unshown t) (current-message)"), "nil");
    vm.set_echo_area_host(true);
    assert_eq!(ask(&mut vm, "(list (prin1 '(a \"λ\") t) (current-message))"), "((a \"λ\") \"(a \\\"λ\\\")\")");
    assert_eq!(ask(&mut vm, "(princ \" b\") (current-message)"), "\"(a \\\"λ\\\") b\"");
    assert_eq!(ask(&mut vm, "(message \"m\") (princ 7 t) (current-message)"), "\"7\"");
    assert_eq!(ask(&mut vm, "(setq noninteractive t) (princ 8 t) (setq noninteractive nil) (current-message)"), "\"7\"");
}

/// `message' of a raw byte succeeds as pinned GNU's does, returning the
/// unibyte string it formatted, and leaves it as the echo area's message.
/// The log is off here: logging inserts the text into *Messages*, and a VM2
/// multibyte buffer cannot hold raw bytes yet (D005), which the next test
/// pins.
#[test]
fn message_accepts_raw_bytes_as_gnu() {
    let source = "(string-to-multibyte (prin1-to-string (condition-case e (let ((message-log-max nil)) (message \"%s\" (unibyte-string 233 120))) (error (list 'error e)))))";
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&oracle).args(["-Q", "--batch", "--eval", &format!("(princ {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    assert_eq!(expected, "\"\\351x\"");
    let mut vm = Vm::new();
    vm.set_echo_area_host(true);
    let printed = vm.eval_source(source).unwrap().display().unwrap();
    assert_eq!(serde_json::from_str::<String>(&printed).unwrap(), expected);
    assert_eq!(vm.eval_source("(equal (current-message) (unibyte-string 233 120))").unwrap().display().unwrap(), "t");
}

/// With the log on, GNU inserts the raw byte into *Messages* as an eight-bit
/// character (VM2-RAWTEXT1); `message' returns its unibyte string and the
/// echo area shows it.
#[test]
fn logged_raw_byte_message_reaches_the_echo_area() {
    let mut vm = Vm::new();
    vm.set_echo_area_host(true);
    let source = "(progn (message \"before\") \
                  (list (condition-case e (message \"%s\" (unibyte-string 233 120)) (error (car e))) (current-message) \
                        (with-current-buffer \"*Messages*\" (char-after (- (point-max) 3)))))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(#u8(233 120) #u8(233 120) 4194281)");
}
