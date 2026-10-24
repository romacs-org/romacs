use romacs_vm::Vm;

#[test]
fn system_name_native_contract() {
    let mut vm = Vm::new();
    for (source, expected) in [
        ("(list (boundp 'system-name) (fboundp 'system-name) (functionp #'system-name) (stringp system-name) (eq system-name (system-name)) (eq system-name (funcall #'system-name)) (multibyte-string-p (system-name)))", "(t t t t t t nil)"),
        ("(let ((system-name \"custom-host\")) (list system-name (system-name) (funcall #'system-name)))", "(\"custom-host\" \"custom-host\" \"custom-host\")"),
        ("(let ((system-name 42)) (system-name))", "42"),
        ("(let ((system-name nil)) (system-name))", "nil"),
        ("(let ((system-name '(a b))) (eq system-name (system-name)))", "t"),
        ("(condition-case e (apply #'system-name '(1)) (error (list (car e) (car (cdr (cdr e))))))", "(wrong-number-of-arguments 1)"),
    ] {
        let actual = vm.eval_source(source).and_then(|v| v.display());
        assert_eq!(actual.as_deref(), Ok(expected), "SYSTEM_NAME_NATIVE_CONTRACT: {source}");
    }
    vm.collect_garbage();
    let actual = vm.eval_source("(eq system-name (system-name))").and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok("t"), "SYSTEM_NAME_GC_ROOT");
}

#[test]
fn system_name_initial_host() {
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap();
    let host = host.strip_suffix('\n').unwrap_or(&host).replace([' ', '\t'], "-");
    let mut vm = Vm::new();
    let source = format!("(equal (system-name) {})", serde_json::to_string(&host).unwrap());
    let actual = vm.eval_source(&source).and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok("t"), "SYSTEM_NAME_INITIAL_HOST");
}
