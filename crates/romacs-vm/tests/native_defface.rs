use romacs_vm::Vm;

fn answer(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).unwrap().display().unwrap()
}

#[test]
fn native_defface_needs_no_gnu_custom_files() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(defface native-face-test '((t (:foreground \"red\"))) \"Native face\" :group 'basic-faces)"), "native-face-test");
    assert_eq!(answer(&mut vm, "(fboundp 'custom-declare-face)"), "nil");
}

#[test]
fn native_defgroup_records_metadata_only() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(defgroup native-face-group nil \"Group\" :prefix \"nf-\" :group 'parent)"), "native-face-group");
    assert_eq!(answer(&mut vm, "(list (get 'native-face-group 'group-documentation) (get 'native-face-group 'custom-prefix) (get 'native-face-group 'custom-group))"), "(\"Group\" \"nf-\" parent)");
    assert_eq!(answer(&mut vm, "(defgroup native-version-group nil \"Version\" :version \"22.1\") (get 'native-version-group 'custom-version)"), "\"22.1\"");
}

#[test]
fn native_defcustom_delays_without_evaluating_initializer() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(setq native-delay-count 0) (defcustom native-delay-option (setq native-delay-count (1+ native-delay-count)) \"Delayed\" :type 'integer :initialize #'custom-initialize-delay) (list native-delay-count (boundp 'native-delay-option) custom-delayed-init-variables (get 'native-delay-option 'standard-value))"), "(0 nil (native-delay-option) ((setq native-delay-count (1+ native-delay-count))))");
    assert_eq!(answer(&mut vm, "(get 'native-delay-option 'variable-documentation)"), "\"Delayed\"");
    assert_eq!(answer(&mut vm, "(setq native-bound-option 91) (defcustom native-bound-option (error \"must not run\") \"New documentation\" :initialize 'custom-initialize-delay) (list native-bound-option (get 'native-bound-option 'variable-documentation))"), "(91 \"New documentation\")");
    assert_eq!(answer(&mut vm, "(setq custom-delayed-init-variables t) (defcustom native-after-startup (setq native-delay-count (1+ native-delay-count)) \"After startup\" :initialize #'custom-initialize-delay) (list native-delay-count native-after-startup)"), "(1 1)");
}
