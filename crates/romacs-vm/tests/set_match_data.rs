use romacs_vm::Vm;

#[test]
fn set_match_data_retains_buffer_across_moving_collection() {
    let mut vm = Vm::new();
    // The fixture calls GNU's Lisp-level buffer constructor from subr.el.
    vm.install_lisp_prelude().unwrap();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    vm.set_load_path(&[root.join("gnu-lisp"), root.join("gnu-lisp/emacs-lisp")]).unwrap();
    for file in ["emacs-lisp/debug-early.el", "emacs-lisp/byte-run.el", "emacs-lisp/backquote.el", "subr.el"] {
        vm.load_file(&root.join("gnu-lisp").join(file)).unwrap();
    }
    vm.set_collection_floor(usize::MAX);
    // Retire the loads' request code now, so the request below, whose garbage
    // the collection measures, does not collect at its end.
    vm.collect_garbage();
    vm.eval_source("(dotimes (i 80) (copy-sequence \"dead allocation\")) (set-buffer (generate-new-buffer \"match-retained\")) (insert \"abc\") (set-match-data (list 1 3 (current-buffer))) (set-buffer (get-buffer-create \"other\"))").unwrap();
    let before = vm.heap_stats();
    vm.collect_garbage();
    let after = vm.heap_stats();
    eprintln!("match-data GC objects {} -> {}, bytes {} -> {}, collections {} -> {}", before.live_objects, after.live_objects, before.live_bytes, after.live_bytes, before.collections, after.collections);
    assert!(after.collections > before.collections);
    assert!(after.live_objects < before.live_objects);
    assert!(after.live_bytes < before.live_bytes);
    let value = vm.eval_source("(list (match-beginning 0) (match-end 0) (eq (car (cdr (cdr (match-data t)))) (get-buffer \"match-retained\")))").unwrap();
    assert_eq!(value.display().unwrap(), "(1 3 t)");
}

#[test]
fn set_match_data_restores_clears_and_checks_inputs() {
    for (source, expected) in [
        ("(progn (set-match-data '(1 3 nil nil 4 5)) (list (match-data t) (match-beginning 0) (match-end 2)))", "((1 3 nil nil 4 5) 1 5)"),
        ("(progn (string-match \"a\" \"a\") (set-match-data nil) (list (match-data) (match-beginning 0)))", "(nil nil)"),
        ("(progn (set-match-data '(1 3)) (set-match-data '(8)) (match-data))", "nil"),
        ("(condition-case e (set-match-data '(1 nil)) (error e))", "(wrong-type-argument integer-or-marker-p nil)"),
        ("(condition-case e (set-match-data '(1 2 . 3)) (error e))", "(wrong-type-argument listp 3)"),
        ("(progn (set-match-data '(-1 3)) (match-data t))", "nil"),
        ("(progn (set-match-data '(2 -1)) (match-data t))", "(2 -1)"),
        ("(let ((m (make-marker))) (set-match-data (list m m)) (list (match-data t) (marker-position m)))", "((0 0) nil)"),
        ("(progn (erase-buffer) (insert \"abc\") (let* ((m (copy-marker 2)) (l (list m (copy-marker 3)))) (set-match-data l t) (list (match-beginning 0) (match-end 0) l (marker-position m))))", "(2 3 (nil nil) nil)"),
    ] {
        let mut vm = Vm::new();
        vm.set_collection_floor(1);
        let value = vm.eval_source(&format!("(condition-case e {source} (error e))")).expect("condition wrapper");
        assert_eq!(value.display().unwrap(), expected, "{source}");
    }
}

#[test]
fn documented_negative_match_end_extraction_is_safely_rejected() {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let value = vm.eval_source(r#"(progn (set-match-data '(2 -1)) (list (match-data t) (condition-case e (match-string 0 "abcd") (error e))))"#).unwrap();
    assert_eq!(value.display().unwrap(), r#"((2 -1) (args-out-of-range "abcd" 2 -1))"#);
}
