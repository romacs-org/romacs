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

/// cus-face.el's custom-declare-face: the spec and documentation a face was
/// declared with, where GNU's own `defface` looks before declaring it again.
#[test]
fn native_defface_records_the_declared_spec_and_documentation() {
    let mut vm = Vm::new();
    answer(&mut vm, "(setq native-spec-count 0)");
    answer(&mut vm, "(defface native-spec-face (progn (setq native-spec-count (1+ native-spec-count)) '((t (:foreground \"red\")))) \"Declared\")");
    assert_eq!(answer(&mut vm, "(list native-spec-count (get 'native-spec-face 'face-defface-spec) (get 'native-spec-face 'face-documentation) (internal-get-lisp-face-attribute 'native-spec-face :foreground))"),
               "(1 ((t (:foreground \"red\"))) \"Declared\" \"red\")");
}

/// A spec may name `:height' twice, and a function runs on every
/// recalculation wherever in the plist it stands: it keeps the face from
/// being memoised.
#[test]
fn a_function_height_after_a_number_runs_on_every_recalculation() {
    let mut vm = Vm::new();
    answer(&mut vm, "(setq calls 0)");
    answer(&mut vm, "(defface height-face (list (list t :height 120 :height (lambda (h) (setq calls (1+ calls)) h))) \"Twice\")");
    answer(&mut vm, "(setq calls 0)");
    assert_eq!(answer(&mut vm, "(progn (dotimes (_ 3) (romacs--face-spec-recalc 'height-face (selected-frame))) calls)"), "3");
}

#[test]
fn native_defface_registers_separate_global_and_frame_attributes() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    answer(&mut vm, "(defface native-query-face '((t (:foreground \"red\" :weight bold :inherit (parent-a parent-b)))) \"Queryable face\")");
    assert_eq!(answer(&mut vm, "(list (internal-get-lisp-face-attribute 'native-query-face :foreground) (internal-get-lisp-face-attribute 'native-query-face :weight t) (internal-get-lisp-face-attribute 'native-query-face :inherit t) (eq (internal-lisp-face-p 'native-query-face) (internal-lisp-face-p 'native-query-face (selected-frame))))"), "(\"red\" bold (parent-a parent-b) nil)");
    answer(&mut vm, "(aset (internal-lisp-face-p 'native-query-face (selected-frame)) 5 'light) (garbage-collect)");
    assert_eq!(answer(&mut vm, "(list (internal-get-lisp-face-attribute 'native-query-face :weight) (internal-get-lisp-face-attribute 'native-query-face :weight t))"), "(light bold)");
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
