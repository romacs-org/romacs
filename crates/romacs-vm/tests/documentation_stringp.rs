use romacs_vm::Vm;
#[test]
fn documentation_stringp_matches_gnu() {
    let mut vm = Vm::new();
    assert_eq!(vm.eval_source("(list (documentation-stringp (unibyte-string 255)) (documentation-stringp (cons (unibyte-string 255) 1)))").unwrap().display().unwrap(), "(t t)", "DOCUMENTATION_STRINGP_RAW_BYTES");
    assert_eq!(vm.eval_source("(condition-case e (list (documentation-stringp \"doc\") (documentation-stringp 0) (documentation-stringp -1) (documentation-stringp '(\"f\" . 3)) (documentation-stringp '(\"f\" 3)) (documentation-stringp '(a . 3)) (documentation-stringp nil) (documentation-stringp #x2000000000000000) (funcall 'documentation-stringp \"doc\")) (error e))").unwrap().display().unwrap(), "(t t t t nil nil nil nil t)", "DOCUMENTATION_STRINGP_GNU");
}
