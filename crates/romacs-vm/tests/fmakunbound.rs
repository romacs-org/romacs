use romacs_vm::Vm;

#[test]
fn fmakunbound_clears_only_the_named_function_cell() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(progn (setq fm-value 17) (fset 'fm-value (lambda () 9)) (list (eq (fmakunbound 'fm-value) 'fm-value) (null (symbol-function 'fm-value)) (fboundp 'fm-value) fm-value))", "(t t nil 17)"),
        ("(progn (fset 'fm-target (lambda () 3)) (fset 'fm-alias 'fm-target) (fmakunbound 'fm-alias) (list (fboundp 'fm-target) (fboundp 'fm-alias) (fm-target)))", "(t nil 3)"),
        ("(list (fmakunbound 'fm-absent) (fmakunbound :fm-keyword))", "(fm-absent :fm-keyword)"),
        ("(condition-case e (fmakunbound nil) (error e))", "(setting-constant nil)"),
        ("(condition-case e (fmakunbound t) (error e))", "(setting-constant t)"),
        ("(condition-case e (fmakunbound 12) (error e))", "(wrong-type-argument symbolp 12)"),
        ("(progn (fset 'fm-cached (lambda () 1)) (defun fm-caller () (fm-cached)) (fm-caller) (fmakunbound 'fm-cached) (condition-case e (fm-caller) (error e)))", "(void-function fm-cached)"),
        ("(progn (fset 'fm-indirect (lambda () 1)) (fmakunbound 'fm-indirect) (condition-case e (funcall 'fm-indirect) (error e)))", "(void-function fm-indirect)"),
    ] {
        assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), expected, "{source}");
    }
}
