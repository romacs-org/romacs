use romacs_vm::Vm;

fn answer(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(error) => format!("VM error: {error}"),
    }
}

#[test]
fn defvar_one_argument_declares_special_only_in_lexical_scope() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(defun s123-read-special () s123-local-special) (let () (defvar s123-local-special) (let ((s123-local-special 17)) (s123-read-special)))"), "17");
    assert_eq!(answer(&mut vm, "(list (boundp 's123-local-special) (special-variable-p 's123-local-special))"), "(nil nil)");
}

#[test]
fn defvar_initializer_does_not_write_enclosing_lexical_binding() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(let ((s123-shadow 9)) (defvar s123-shadow 4) (list s123-shadow (symbol-value 's123-shadow)))"), "(9 4)");
    assert_eq!(answer(&mut vm, "(defvar s123-shadow (error \"must not run\") \"recorded\") (list s123-shadow (get 's123-shadow 'variable-documentation))"), "(4 \"recorded\")");
}

#[test]
fn defvar_local_initializes_once_and_marks_automatic_locality() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(defvar-local s123-buffer-option 11 \"local option\") (defvar-local s123-buffer-option (error \"must not run\")) (list s123-buffer-option (local-variable-if-set-p 's123-buffer-option) (get 's123-buffer-option 'variable-documentation))"), "(11 t \"local option\")");
}

#[test]
fn defcustom_records_unevaluated_standard_and_keyword_metadata() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(setq s123-custom-count 0 s123-custom-option 91) (defcustom s123-custom-option (progn (setq s123-custom-count (1+ s123-custom-count)) 23) \"option\" :type 'integer :group 's123-options :set 'ignore) (list s123-custom-option s123-custom-count (get 's123-custom-option 'custom-type) (get 's123-custom-option 'custom-set) (car (get 's123-custom-option 'standard-value)))"), "(91 0 integer ignore (progn (setq s123-custom-count (1+ s123-custom-count)) 23))");
}

#[test]
fn defcustom_locality_preserves_defaults_and_permanent_locals() {
    let mut vm = Vm::new();
    let source = "(setq locality-count 0) (defcustom locality-normal nil \"normal\" :local nil) (defcustom locality-auto 'initial \"auto\" :local t) (defcustom locality-permanent 'initial \"permanent\" :local (progn (setq locality-count (1+ locality-count)) 'permanent)) (defcustom locality-permanent-only nil \"permanent only\" :local 'permanent-only) (list locality-count (local-variable-if-set-p 'locality-normal) (local-variable-if-set-p 'locality-auto) (get 'locality-auto 'permanent-local) (local-variable-if-set-p 'locality-permanent) (get 'locality-permanent 'permanent-local) (local-variable-if-set-p 'locality-permanent-only) (get 'locality-permanent-only 'permanent-local))";
    assert_eq!(answer(&mut vm, source), "(1 nil t nil t t nil t)");
    let source = "(let ((old (current-buffer)) (new (get-buffer-create \"locality-fixture\"))) (unwind-protect (progn (set-buffer new) (setq locality-auto 'changed locality-permanent 'changed) (let ((before (list locality-auto locality-permanent (default-value 'locality-auto) (default-value 'locality-permanent)))) (kill-all-local-variables) (list before locality-auto locality-permanent (progn (set-buffer old) (list locality-auto locality-permanent))))) (set-buffer old) (kill-buffer new)))";
    assert_eq!(answer(&mut vm, source), "((changed changed initial initial) initial changed (initial initial))");
}

#[test]
fn defvar_keymap_parses_key_descriptions_and_preserves_existing_value() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(defvar-keymap s123-map :doc \"map\" \"C-c a\" 's123-command \"<f5>\" 's123-other) (defvar-keymap s123-map \"b\" (error \"must not run\")) (list (keymapp s123-map) (lookup-key s123-map (kbd \"C-c a\")) (lookup-key s123-map (kbd \"<f5>\")) (lookup-key s123-map (kbd \"b\")) (get 's123-map 'variable-documentation))"), "(t s123-command s123-other nil \"map\")");
}

#[test]
fn variable_definition_values_survive_counted_moving_collection() {
    let mut vm = Vm::new();
    vm.set_gc_moving(true);
    vm.set_collection_floor(usize::MAX);
    assert_eq!(answer(&mut vm, "(dotimes (i 80) (copy-sequence \"dead\")) (defvar-keymap s123-root-map \"a\" 's123-root-command) (defcustom s123-root-option (list (copy-sequence \"retained\")) \"root\" :type '(repeat string)) (defvar-local s123-root-local nil) (setq s123-root-local (list (copy-sequence \"local-retained\"))) (list (keymapp s123-root-map) s123-root-option)"), "(t (\"retained\"))");
    let old = format!("{:?}", vm.eval_source("s123-root-map").unwrap());
    let before = vm.heap_stats().collections;
    vm.collect_garbage();
    let after = vm.heap_stats().collections;
    let new = format!("{:?}", vm.eval_source("s123-root-map").unwrap());
    assert!(after > before, "forced collection must increment collection counter");
    assert_ne!(old, new, "fixture must relocate the retained keymap");
    assert_eq!(answer(&mut vm, "(list (lookup-key s123-root-map (kbd \"a\")) s123-root-option (car (get 's123-root-option 'standard-value)) (get 's123-root-option 'custom-type) s123-root-local)"), "(s123-root-command (\"retained\") (list (copy-sequence \"retained\")) (repeat string) (\"local-retained\"))");
}

#[test]
fn local_values_bind_and_switch_buffers_without_crossing_ownership() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(defvar-local option 10) (setq first (current-buffer) second (get-buffer-create (generate-new-buffer-name \"second\"))) (setq option 11) (let ((option 12)) (set-buffer second) (setq observed option) (setq option 22) (set-buffer first) (setq observed (list observed option))) (list observed option (progn (set-buffer second) option))"), "((10 12) 11 22)");
}

#[test]
fn defvar_documentation_is_installed_before_initializer() {
    let mut vm = Vm::new();
    assert_eq!(answer(&mut vm, "(defvar doc-visible (get 'doc-visible 'variable-documentation) \"visible\") doc-visible"), "\"visible\"");
}

/// GNU does NOT extend a local special declaration to lambda parameters, and
/// the asymmetry with `let` is deliberate on GNU's side rather than an accident
/// of ours -- do not "fix" this back. `funcall_lambda` (`src/eval.c:3433`) binds
/// lexically whenever `lexenv` is non-nil and never consults a local-special
/// marker, while `Flet`/`FletX` check `declared_special` and
/// `Fmemq (var, Vinternal_interpreter_environment)`. A one-argument `Fdefvar`
/// enters the interpreter environment, which `let` reads and lambda binding does
/// not. Verified on live GNU 32.0.50, interpreted, with `lexical-binding: t` on
/// line 1 of the probe: the lambda-parameter form signals `void-variable` while
/// the `let` form answers 18. (A probe without that cookie runs dynamically and
/// answers 18 for both, which proves nothing.)
#[test]
fn local_declaration_does_not_reach_lambda_parameters_but_does_reach_let() {
    let mut vm = Vm::new();
    // The reader is a separate `defun` on purpose: its body can only observe a
    // DYNAMIC binding, so it discriminates. A lexically nested closure would see
    // the value under either discipline and could not.
    assert!(
        vm.eval_source("(defun read-lambda-special () lambda-special) (let () (defvar lambda-special) (funcall (lambda (lambda-special) (read-lambda-special)) 18))").is_err(),
        "a lambda parameter must not be made special by a local (defvar x)"
    );
    let mut vm = Vm::new();
    assert_eq!(
        answer(&mut vm, "(defun read-let-special () let-special) (let () (defvar let-special) (let ((let-special 18)) (read-let-special)))"),
        "18",
        "but `let` must honour the local special declaration"
    );
}

#[test]
fn variable_definition_forms_reject_malformed_shapes() {
    let mut vm = Vm::new();
    for source in ["(defvar)", "(defvar x 1 \"d\" 2)", "(defvar-local)", "(defcustom x 1 \"d\" :type)", "(defvar-keymap x \"a\")", "(defvar-keymap x :unknown t)", "(defvar-keymap x \"a\" 'one \"a\" 'two)"] {
        assert!(vm.eval_source(source).is_err(), "must reject {source}");
    }
}
