//! GNU source pin 45b0f7699df36b3849edc3bdef37d4cb9e75b9ea, buffer.c,
//! insdel.c and textprop.c. Live oracle is separately labelled in lane evidence.
use romacs_vm::Vm;
fn eval(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(v) => v.display().unwrap(),
        Err(e) => format!("VM-ERROR {e}"),
    }
}
#[test]
fn native_modification_history() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(with-temp-buffer (list (buffer-modified-p) (buffer-modified-tick) (buffer-chars-modified-tick)))"), "(nil 1 1)", "COUNTERS_INITIAL_NATIVE_STATE");
    assert_eq!(eval(&mut vm, "(with-temp-buffer (insert \"saved\") (set-buffer-modified-p nil) (insert \"x\") (delete-region 6 7) (list (buffer-string) (buffer-modified-p)))"), "(\"saved\" t)", "COUNTERS_REVERSE_EDIT_REMAINS_MODIFIED_NOT_UNDO");
    assert_eq!(eval(&mut vm, "(with-temp-buffer (list (set-buffer-modified-p 'yes) (buffer-modified-p) (buffer-modified-tick) (buffer-chars-modified-tick) (set-buffer-modified-p nil) (buffer-modified-p) (buffer-modified-tick)))"), "(nil t 2 1 nil nil 2)", "COUNTERS_EXPLICIT_MODIFIED_TRANSITIONS");
    assert_eq!(eval(&mut vm, "(with-temp-buffer (insert \"λλλλ\") (list (buffer-modified-tick) (buffer-chars-modified-tick)))"), "(4 4)", "COUNTERS_LOGARITHMIC_CHARACTER_NOT_BYTE_LENGTH");
    assert_eq!(eval(&mut vm, "(with-temp-buffer (insert \"abc\") (set-buffer-modified-p nil) (delete-region 1 2) (list (buffer-modified-p) (buffer-modified-tick) (buffer-chars-modified-tick)))"), "(t 4 4)", "COUNTERS_GENERIC_DELETE_SHARED_MUTATION");
    assert_eq!(eval(&mut vm, "(with-temp-buffer (insert \"λλλλ\") (goto-char 1) (insert \"λλλλ\") (buffer-modified-tick))"), "7", "COUNTERS_GENERIC_LOGARITHMIC_CHARACTER_LENGTH");
    assert_eq!(eval(&mut vm, "(with-temp-buffer (insert \"\") (delete-region 1 1) (list (buffer-modified-p) (buffer-modified-tick)))"), "(nil 1)", "COUNTERS_EMPTY_EDITS_NO_CHANGE");
    assert_eq!(eval(&mut vm, "(with-temp-buffer (list (restore-buffer-modified-p 'autosaved) (buffer-modified-p) (set-buffer-modified-p nil) (buffer-modified-p)))"), "(autosaved autosaved nil nil)", "COUNTERS_AUTOSAVED_STATE");
}
#[test]
fn native_modification_properties_and_binary_conversion() {
    for binary in ["", "(set-buffer-multibyte nil)"] {
        let mut vm = Vm::new();
        let source = format!("(with-temp-buffer {binary} (insert \"abc\") (set-buffer-modified-p nil) (put-text-property 1 4 'p 7) (list (buffer-modified-p) (buffer-modified-tick) (buffer-chars-modified-tick) (progn (put-text-property 1 4 'p 7) (buffer-modified-tick))))");
        assert_eq!(eval(&mut vm, &source), "(t 4 3 4)", "COUNTERS_PROPERTY_ONLY_AND_NOOP {binary}");
    }
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(with-temp-buffer (insert \"abc\") (set-buffer-modified-p nil) (set-buffer-multibyte nil) (set-buffer-multibyte t) (list (buffer-modified-p) (buffer-modified-tick) (buffer-chars-modified-tick)))"), "(nil 3 3)", "COUNTERS_CONVERSION_PRESERVES_SAVED_STATE");
    assert_eq!(eval(&mut vm, "(with-temp-buffer (set-buffer-multibyte nil) (insert \"abc\") (set-buffer-modified-p nil) (insert \"x\") (delete-region 4 5) (list (equal (buffer-string) \"abc\") (buffer-modified-p) (buffer-modified-tick)))"), "(t t 5)", "COUNTERS_BINARY_REVERSE_EDIT");
}
#[test]
fn native_modification_optional_buffer_and_gc() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(with-temp-buffer (let ((other (current-buffer))) (set-buffer-modified-p t) (with-temp-buffer (list (buffer-modified-p other) (buffer-modified-tick other) (buffer-chars-modified-tick other) (buffer-modified-p)))))"), "(t 2 1 nil)", "COUNTERS_GC_MOVED_BUFFER_STATE");
    assert_eq!(eval(&mut vm, "(setq counter-buffer (get-buffer-create (generate-new-buffer-name \"gc-counter\"))) (with-current-buffer counter-buffer (insert \"x\") (set-buffer-modified-p nil))"), "nil");
    vm.collect_garbage();
    assert_eq!(eval(&mut vm, "(list (buffer-modified-p counter-buffer) (buffer-modified-tick counter-buffer))"), "(nil 2)", "COUNTERS_GC_PRESERVES_SAVED_STATE");
    assert_eq!(eval(&mut vm, "(condition-case e (buffer-modified-p \"not-a-buffer\") (wrong-type-argument (car e)))"), "wrong-type-argument", "COUNTERS_OPTIONAL_BUFFER_TYPE");
}
#[test]
fn native_modification_nil_property_is_a_real_change() {
    for binary in ["", "(set-buffer-multibyte nil)"] {
        let mut vm = Vm::new();
        let source = format!("(with-temp-buffer {binary} (insert \"abc\") (set-buffer-modified-p nil) (put-text-property 1 4 'p nil) (list (buffer-modified-p) (buffer-modified-tick) (buffer-chars-modified-tick) (progn (put-text-property 1 4 'p nil) (buffer-modified-tick))))");
        assert_eq!(eval(&mut vm, &source), "(t 4 3 4)", "COUNTERS_NIL_PROPERTY_ABSENCE_IS_CHANGE {binary}");
    }
}
#[test]
fn native_modification_missing_visit_clears_empty_modified_buffer() {
    let mut vm = Vm::new();
    let absent = std::env::temp_dir().join(format!("counter-no-such-file-{}", std::process::id()));
    assert!(!absent.exists());
    let name = serde_json::to_string(absent.to_str().unwrap()).unwrap();
    let source = format!("(with-temp-buffer (set-buffer-modified-p t) (condition-case nil (insert-file-contents {name} t) (file-missing nil)) (buffer-modified-p))");
    assert_eq!(eval(&mut vm, &source), "nil", "COUNTERS_MISSING_VISIT_CLEARS_MODIFIED_BEFORE_SIGNAL");
}
#[test]
fn native_undo_frontier_is_explicit_not_an_inverse_edit() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(list (fboundp 'undo) (fboundp 'undo-only) (fboundp 'primitive-undo) (fboundp 'buffer-enable-undo))"), "(nil nil nil nil)", "COUNTERS_UNDO_SUBSTRATE_UNAVAILABLE_NOT_PARITY");
}
#[test]
fn native_modification_visit_success_and_failed_save() {
    let dir = std::env::temp_dir().join(format!("romacs-counters-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("target");
    std::fs::write(&file, b"saved").unwrap();
    let name = serde_json::to_string(file.to_str().unwrap()).unwrap();
    let missing = serde_json::to_string(dir.join("absent/target").to_str().unwrap()).unwrap();
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, &format!("(with-temp-buffer (insert-file-contents {name} t) (list (buffer-modified-p) (progn (insert \"x\") (write-region nil nil {name} nil t) (buffer-modified-p))))")), "(nil nil)", "COUNTERS_VISIT_READ_AND_SAVE_CLEAR");
    assert_eq!(eval(&mut vm, &format!("(with-temp-buffer (insert \"x\") (condition-case nil (write-region nil nil {missing} nil t) (file-error nil)) (buffer-modified-p))")), "t", "COUNTERS_FAILED_SAVE_STAYS_MODIFIED");
    assert_eq!(eval(&mut vm, &format!("(with-temp-buffer (insert \"x\") (write-region nil nil {name} nil 'silent) (buffer-modified-p))")), "t", "COUNTERS_NONVISITING_WRITE_STAYS_MODIFIED");
    std::fs::remove_dir_all(dir).unwrap();
}
