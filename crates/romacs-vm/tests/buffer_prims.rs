use romacs_vm::Vm;
#[test]
fn inventory() {
    let mut vm = Vm::new();
    for name in ["format-message", "line-end-position", "line-beginning-position", "point-marker", "save-restriction", "add-text-properties", "buffer-file-name", "char-before", "insert-buffer-substring", "next-single-char-property-change", "next-single-property-change", "point-max-marker", "previous-single-property-change", "rename-buffer", "replace-region-contents", "save-current-buffer"] {
        println!("{name}: {}", vm.eval_source(&format!("(fboundp '{name})")).unwrap().display().unwrap());
    }
    println!("save-current-buffer direct special form: {}", vm.eval_source("(save-current-buffer 42)").unwrap().display().unwrap());
    vm.install_lisp_prelude().unwrap();
    for name in ["format-message", "line-end-position", "line-beginning-position", "point-marker", "save-restriction", "add-text-properties", "buffer-file-name", "char-before", "insert-buffer-substring", "next-single-char-property-change", "next-single-property-change", "point-max-marker", "previous-single-property-change", "rename-buffer", "replace-region-contents", "save-current-buffer"] {
        println!("prelude {name}: {}", vm.eval_source(&format!("(fboundp '{name})")).unwrap().display().unwrap());
    }
}
#[test]
fn buffer_file_name_reads_requested_buffer() {
    let mut vm = Vm::new();
    let result = vm.eval_source("(let ((b (get-buffer-create \"file-test\"))) (set-buffer b) (setq buffer-file-name \"λ.txt\") (list (buffer-file-name) (buffer-file-name b) (condition-case e (buffer-file-name \"file-test\") (error e))))").and_then(|v| v.display());
    assert_eq!(result.unwrap_or_else(|e| format!("ERROR: {e:?}")), "(\"λ.txt\" \"λ.txt\" (wrong-type-argument bufferp \"file-test\"))", "buffer-file-name must read the requested buffer");
}

#[test]
fn char_before_honors_accessible_multibyte_positions() {
    let mut vm = Vm::new();
    let result = vm.eval_source("(with-temp-buffer (insert \"aλ文z\") (goto-char 3) (narrow-to-region 2 4) (list (char-before) (char-before 2) (char-before 4) (char-before 5) (point)))").and_then(|v| v.display());
    assert_eq!(result.unwrap_or_else(|e| format!("ERROR: {e:?}")), "(955 nil 25991 nil 3)", "char-before respects BEGV/ZV and character coordinates");
}
