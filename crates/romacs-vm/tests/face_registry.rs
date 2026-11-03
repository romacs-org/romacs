use romacs_vm::Vm;

#[test]
fn replay_live_recorded_gnu_cases_offline() {
    let evidence: serde_json::Value = serde_json::from_str(include_str!("../evidence/face-registry/review-live-cases.json")).unwrap();
    let cases = evidence["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 11);
    for case in cases {
        let mut vm = Vm::new();
        let source = case["expression"].as_str().unwrap();
        let actual = eval(&mut vm, &format!("(condition-case face-error {source} (error face-error))"));
        assert_eq!(actual, case["result"].as_str().unwrap(), "LIVE_GNU_REPLAY: {source}");
    }
}


#[test]
fn registry_roots_forward_through_collection_without_startup_allocation() {
    let mut vm = Vm::new();
    assert_eq!(vm.heap_objects(), 0, "FACE_ZERO_STARTUP_ALLOCATION");
    eval(&mut vm, "(progn (internal-make-lisp-face 'registry-gc) (internal-make-lisp-face 'registry-gc (selected-frame)) (setq registry-gc-global (internal-lisp-face-p 'registry-gc)) (setq registry-gc-local (internal-lisp-face-p 'registry-gc (selected-frame))) (aset registry-gc-global 16 (list 'parent-a 'parent-b)) (aset registry-gc-local 4 1.25))");
    let before = vm.heap_stats().collections;
    vm.collect_garbage();
    assert!(vm.heap_stats().collections > before);
    assert_eq!(eval(&mut vm, "(list (eq registry-gc-global (internal-lisp-face-p 'registry-gc)) (eq registry-gc-local (internal-lisp-face-p 'registry-gc (selected-frame))) (internal-get-lisp-face-attribute 'registry-gc :inherit t) (internal-get-lisp-face-attribute 'registry-gc :height))"), "(t t (parent-a parent-b) 1.25)");
}


#[test]
fn registry_is_bound_on_first_use_without_a_prior_make() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(list (boundp 'face--new-frame-defaults) (hash-table-p face--new-frame-defaults) (special-variable-p 'face--new-frame-defaults))"), "(t t t)");
}

#[test]
fn string_names_use_current_obarray() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(progn (internal-make-lisp-face 'registry-test) (list (let ((obarray (obarray-make))) (internal-lisp-face-p \"registry-test\")) (let ((obarray 42)) (condition-case e (internal-lisp-face-p \"registry-test\") (error e)))))"), "(nil (wrong-type-argument obarrayp 42))");
}

#[test]
fn make_falls_back_for_alias_cycles_but_predicate_signals() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(progn (internal-make-lisp-face 'default) (put 'registry-cycle 'face-alias 'registry-cycle) (list (vectorp (internal-make-lisp-face 'registry-cycle)) (condition-case e (internal-lisp-face-p 'registry-cycle) (error e))))"), "(t (circular-list registry-cycle))");
}

#[test]
fn invalid_dotted_face_is_wrapped_in_error_data() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(condition-case e (internal-get-lisp-face-attribute '(a . b) :weight t) (error e))"), "(error \"Invalid face\" (a . b))");
}


fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).unwrap().display().unwrap()
}

#[test]
fn predicate_returns_the_live_global_vector() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(progn (internal-make-lisp-face 'registry-test) (let ((v (internal-lisp-face-p 'registry-test))) (list (vectorp v) (= (length v) 20) (eq (aref v 0) 'face) (eq v (internal-lisp-face-p \"registry-test\")) (internal-lisp-face-p 'missing-registry-face))))"), "(t t t t nil)", "FACE_P_LIVE_VECTOR");
}

#[test]
fn make_resets_slots_without_replacing_identity() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(let ((v (internal-make-lisp-face 'registry-test))) (aset v 5 'bold) (list (eq v (internal-make-lisp-face 'registry-test)) (aref v 5)))"), "(t unspecified)", "FACE_MAKE_RESETS_IN_PLACE");
}

#[test]
fn getter_reads_direct_slots_and_translates_ignore_defface() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(let ((v (internal-make-lisp-face 'registry-test))) (aset v 5 'bold) (aset v 4 ':ignore-defface) (aset v 16 'unknown-parent) (list (internal-get-lisp-face-attribute 'registry-test :weight t) (internal-get-lisp-face-attribute 'registry-test :height t) (internal-get-lisp-face-attribute 'registry-test :inherit t)))"), "(bold unspecified unknown-parent)", "FACE_GET_DIRECT_SLOT");
}

#[test]
fn relative_predicate_does_not_validate_values() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(list (face-attribute-relative-p :height nil) (face-attribute-relative-p :height 10) (face-attribute-relative-p 42 'unspecified) (face-attribute-relative-p :weight ':ignore-defface) (face-attribute-relative-p :weight 'bold))"), "(t nil t t nil)", "FACE_RELATIVE_NONFIXNUM_HEIGHT");
}

#[test]
fn discrete_values_are_fresh_boolean_lists_not_font_enumerations() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(let ((v (internal-lisp-face-attribute-values :underline))) (list v (eq v (internal-lisp-face-attribute-values :underline)) (internal-lisp-face-attribute-values :weight) (condition-case e (internal-lisp-face-attribute-values 42) (error e))))"), "((t nil) nil nil (wrong-type-argument symbolp 42))", "FACE_VALUES_FRESH_BOOLEAN_LIST");
}

#[test]
fn local_and_global_registries_are_distinct() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(progn (internal-make-lisp-face 'registry-test) (list (internal-lisp-face-p 'registry-test (selected-frame)) (condition-case e (internal-get-lisp-face-attribute 'registry-test :weight) (error e)) (let ((local (internal-make-lisp-face 'registry-test (selected-frame)))) (aset local 5 'bold) (list (internal-get-lisp-face-attribute 'registry-test :weight) (internal-get-lisp-face-attribute 'registry-test :weight t)))))"), "(nil (error \"Invalid face\" registry-test) (bold unspecified))");
}

#[test]
fn aliases_and_validation_order_match_gnu() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(progn (internal-make-lisp-face 'registry-test) (put 'registry-alias 'face-alias 'registry-test) (list (eq (internal-lisp-face-p 'registry-alias) (internal-lisp-face-p 'registry-test)) (condition-case e (internal-get-lisp-face-attribute 'no-face 42 t) (error e)) (condition-case e (internal-get-lisp-face-attribute 'registry-test :bogus t) (error e)) (condition-case e (internal-get-lisp-face-attribute \"registry-test\" :weight t) (error e)) (condition-case e (internal-lisp-face-p 'registry-test t) (error e))))"), "(t (error \"Invalid face\" no-face) (error \"Invalid face attribute name\" :bogus) (wrong-type-argument symbolp \"registry-test\") (wrong-type-argument frame-live-p t))");
}

/// xfaces.c Finternal_lisp_face_equal_p, as pinned GNU answers through
/// faces.el's `face-equal' (-Q --batch, the same faces set with
/// `set-face-attribute'): `(t nil t t t ... (error "Invalid face" ...))'.
#[test]
fn lisp_faces_compare_as_gnu_s() {
    let mut vm = Vm::new();
    eval(&mut vm, "(progn (dolist (face '(fe-a fe-b fe-c)) (internal-make-lisp-face face) (internal-make-lisp-face face (selected-frame)))
                    (dolist (face '(fe-a fe-b)) (internal-set-lisp-face-attribute face :foreground \"red\" 0) (internal-set-lisp-face-attribute face :weight 'bold 0))
                    (internal-set-lisp-face-attribute 'fe-c :inherit 'fe-a 0))");
    assert_eq!(eval(&mut vm, "(list (internal-lisp-face-equal-p 'fe-a 'fe-b) (internal-lisp-face-equal-p 'fe-a 'fe-c)
                                    (internal-lisp-face-equal-p 'fe-a 'fe-c nil t) (internal-lisp-face-equal-p 'fe-c 'fe-a nil t)
                                    (internal-lisp-face-equal-p 'fe-a 'fe-b t)
                                    (condition-case e (internal-lisp-face-equal-p 'fe-a 'fe-nosuch) (error e)))"),
               "(t nil t t t (error \"Invalid face\" fe-nosuch))");
    // An alias cycle signals, as pinned GNU's: `((circular-list cy-x) (circular-list cy-x))'.
    assert_eq!(eval(&mut vm, "(progn (put 'cy-x 'face-alias 'cy-y) (put 'cy-y 'face-alias 'cy-x)
                                     (list (condition-case e (internal-lisp-face-equal-p 'cy-x 'fe-a) (error e))
                                           (condition-case e (internal-lisp-face-equal-p 'fe-a 'cy-x) (error e))))"),
               "((circular-list cy-x) (circular-list cy-x))");
}
