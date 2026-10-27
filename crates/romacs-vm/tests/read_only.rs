use romacs_vm::Vm;

fn printed(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(error) => format!("VM-ERROR {error}"),
    }
}

#[test]
fn readonly_text_and_bytes_keep_content_point_restriction_and_ticks() {
    for multibyte in ["t", "nil"] {
        for edit in [
            "(insert \"x\")",
            "(funcall 'insert \"x\")",
            "(delete-region 2 3)",
            "(erase-buffer)",
            "(self-insert-command 1)",
            "(delete-char 1)",
            "(delete-backward-char 1)",
        ] {
            let mut vm = Vm::new();
            // Ferase_buffer widens before del_range refuses (buffer.c:2491),
            // as pinned GNU does: its restriction is the whole buffer after.
            let restriction = if edit == "(erase-buffer)" { "(save-restriction (widen) (list (buffer-string) (point) (point-min) (point-max)))" } else { "(list (buffer-string) (point) (point-min) (point-max))" };
            vm.eval_source(&format!(
                "(set-buffer-multibyte {multibyte}) (insert \"abcde\") (narrow-to-region 2 5) (goto-char 3) (set-buffer-modified-p nil) (setq readonly-before (append {restriction} (list (buffer-modified-tick) (buffer-chars-modified-tick))) buffer-read-only t last-command-event 120)"
            )).unwrap();
            assert_eq!(printed(&mut vm, &format!(
                "(condition-case e {edit} (buffer-read-only (eq (car (cdr e)) (current-buffer))))"
            )), "t", "READONLY_TYPED_IDENTITY: {multibyte}: {edit}");
            assert_eq!(printed(&mut vm,
                "(list (equal readonly-before (list (buffer-string) (point) (point-min) (point-max) (buffer-modified-tick) (buffer-chars-modified-tick))) (buffer-modified-p))"
            ), "(t nil)", "READONLY_NO_SIDE_EFFECTS: {multibyte}: {edit}");
        }
    }
}

#[test]
fn readonly_warmed_compiled_insert_cannot_bypass_guard() {
    let mut vm = Vm::new();
    vm.eval_source("(insert \"seed\") (defun readonly-append (s) (insert s)) (readonly-append \"a\") (readonly-append \"b\") (setq buffer-read-only t)").unwrap();
    assert_eq!(printed(&mut vm,
        "(condition-case e (readonly-append \"c\") (buffer-read-only (eq (car (cdr e)) (current-buffer))))"
    ), "t", "READONLY_THREADED_INSERT");
    assert_eq!(printed(&mut vm, "(buffer-string)"), "\"seedab\"");
}

#[test]
fn readonly_dynamic_inhibition_unwinds_and_is_buffer_local() {
    let mut vm = Vm::new();
    vm.eval_source("(insert \"old\") (setq readonly-owner (current-buffer) buffer-read-only t)").unwrap();
    for inhibitor in ["t", "'(token)", "'token"] {
        assert_eq!(printed(&mut vm, &format!(
            "(let ((inhibit-read-only {inhibitor})) (erase-buffer) (insert \"owned\") (buffer-string))"
        )), "\"owned\"", "READONLY_NONNIL_INHIBITOR: {inhibitor}");
        assert_eq!(printed(&mut vm,
            "(condition-case e (insert \"bad\") (buffer-read-only t))"
        ), "t", "READONLY_PROTECTION_RESTORED");
    }
    assert_eq!(printed(&mut vm,
        "(progn (set-buffer (get-buffer-create \"other\")) (insert \"other\") (list buffer-read-only (with-current-buffer readonly-owner buffer-read-only)))"
    ), "(nil t)", "READONLY_BUFFER_LOCAL");
    assert_eq!(printed(&mut vm,
        "(with-current-buffer readonly-owner (condition-case e (let ((inhibit-read-only t)) (insert \"!\") (signal 'error nil)) (error nil)) (condition-case e (insert \"bad\") (buffer-read-only t)))"
    ), "t", "READONLY_ERROR_UNWIND");
}

#[test]
fn readonly_literal_file_insert_cannot_change_representation() {
    let path = std::env::temp_dir().join(format!("romacs-readonly-{}.bin", std::process::id()));
    std::fs::write(&path, [255, 0, 10]).unwrap();
    let mut vm = Vm::new();
    vm.eval_source("(insert \"sentinel\") (setq buffer-read-only t)").unwrap();
    let filename = serde_json::to_string(path.to_str().unwrap()).unwrap();
    let result = printed(&mut vm, &format!(
        "(condition-case e (insert-file-contents-literally {filename}) (buffer-read-only (eq (car (cdr e)) (current-buffer))))"
    ));
    std::fs::remove_file(path).unwrap();
    assert_eq!(result, "t", "READONLY_BEFORE_REPRESENTATION_CHANGE");
    assert_eq!(printed(&mut vm, "(list (buffer-string) enable-multibyte-characters)"), "(\"sentinel\" t)");
}
