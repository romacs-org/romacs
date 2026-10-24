use romacs_vm::Vm;
fn check(source: &str, expected: &str) {
    let mut vm = Vm::new();
    let result = vm.eval_source(source).and_then(|v| v.display());
    assert_eq!(result.as_deref(), Ok(expected), "OFFSET definition behavioral contract");
}
#[test]
fn offset_target_definition_is_callable() {
    check("(list (fboundp 'define-charset-internal) (define-charset-internal 'offset-probe nil [0 3 0 0 0 0 0 0] nil nil nil nil nil nil nil nil 1024 nil nil nil nil '(:probe 9)) (charset-plist 'offset-probe))", "(t nil (:probe 9))");
}
#[test]
fn offset_definition_is_indirectly_callable() {
    check("(list (functionp #'define-charset-internal) (funcall #'define-charset-internal 'offset-probe nil [0 3 0 0 0 0 0 0] nil nil nil nil nil nil nil nil 1024 nil nil nil nil '(:indirect)) (charset-plist 'offset-probe))", "(t nil (:indirect))");
}
#[test]
fn invalid_code_space_is_rejected_before_publication() {
    check("(condition-case e (define-charset-internal 'offset-probe nil [0 256 0 0 0 0 0 0] nil nil nil nil nil nil nil nil 1024 nil nil nil nil nil) (error e))", "(args-out-of-range 256 0 255)");
}
#[test]
fn aliases_preserve_old_attributes_on_redefinition() {
    check("(progn (define-charset-internal 'offset-probe nil [0 3 0 0 0 0 0 0] nil nil nil nil nil nil nil nil 1024 nil nil nil nil '(:old)) (define-charset-alias 'offset-alias 'offset-probe) (define-charset-internal 'offset-probe nil [0 3 0 0 0 0 0 0] nil nil nil nil nil nil nil nil 2048 nil nil nil nil '(:new)) (list (charset-plist 'offset-probe) (charset-plist 'offset-alias)))", "((:new) (:old))");
}
#[test]
fn late_invalid_plist_preserves_previous_binding() {
    check("(progn (define-charset-internal 'offset-probe nil [0 3 0 0 0 0 0 0] nil nil nil nil nil nil nil nil 1024 nil nil nil nil '(:old)) (condition-case e (define-charset-internal 'offset-probe nil [0 3 0 0 0 0 0 0] nil nil nil nil nil nil nil nil 2048 nil nil nil nil 7) (error nil)) (charset-plist 'offset-probe))", "(:old)");
}
#[test]
fn map_definition_succeeds_with_pinned_data() {
    check("(condition-case e (define-charset-internal 'offset-map nil [0 3 0 0 0 0 0 0] nil nil nil nil nil nil nil nil nil \"8859-2\" nil nil nil nil) (error e))", "nil");
}
