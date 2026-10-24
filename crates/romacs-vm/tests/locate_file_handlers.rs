use romacs_vm::Vm;

// Oracle: GNU 32.0.50, commit 45b0f7699df36b3849edc3bdef37d4cb9e75b9ea.
// openp selects on each suffixed candidate for file-exists-p, not on the
// mere presence of file-name-handler-alist. Handler dispatch remains refused.
fn fixture() -> (std::path::PathBuf, Vm) {
    let root = std::env::temp_dir().join(format!("romacs-locate-handlers-{}-{:?}", std::process::id(), std::thread::current().id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("local.el"), "must not load").unwrap();
    let mut vm = Vm::new();
    vm.eval_source(&format!("(setq default-directory {:?})", format!("{}/", root.display()))).unwrap();
    (root, vm)
}

#[test]
fn unrelated_handlers_do_not_block_local_lookup_or_change_match_data() {
    let (root, mut vm) = fixture();
    let expected = format!("{:?}", root.join("local.el").display().to_string());
    for table in [
        "'((\"\\\\`/unrelated/\" . handler))",
        "'((\"LOCAL\" . handler))", // case-fold-search must not affect handlers
        "'(ignored (42 . handler))", // malformed entries are skipped by GNU
    ] {
        let source = format!("(let ((file-name-handler-alist {table}) (case-fold-search t)) (string-match \"a\\\\(b\\\\)\" \"zab\") (let ((before (match-data))) (list (locate-file-internal \"local\" '(nil) '(\".el\")) (equal before (match-data)))))");
        assert_eq!(vm.eval_source(&source).unwrap().display().unwrap(), format!("({expected} t)"), "{source}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn handler_operations_and_inhibition_are_operation_specific() {
    let (root, mut vm) = fixture();
    let expected = format!("{:?}", root.join("local.el").display().to_string());
    for setup in [
        "(put 'handler 'operations '(write-region))",
        "(put 'handler 'operations '(file-exists-p)) (setq inhibit-file-name-operation 'file-exists-p inhibit-file-name-handlers '(handler))",
        // The later-starting nil handler wins over the first handler.
        "(put 'handler 'operations '(file-exists-p)) (setq file-name-handler-alist '((\"local\" . handler) (\"el\\\\'\" . nil)))",
    ] {
        let source = format!("(let ((file-name-handler-alist '((\"local\\\\.el\\\\'\" . handler))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil)) {setup} (locate-file-internal \"local\" '(nil) '(\".el\")))");
        assert_eq!(vm.eval_source(&source).unwrap().display().unwrap(), expected, "{source}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn matching_candidate_handlers_remain_explicitly_refused() {
    let (root, mut vm) = fixture();
    for setup in [
        "(put 'handler 'operations '(file-exists-p))",
        "(put 'handler 'operations '(file-exists-p)) (setq inhibit-file-name-operation 'write-region inhibit-file-name-handlers '(handler))",
        "(put 'handler 'operations '(expand-file-name))",
    ] {
        let source = format!("(let ((file-name-handler-alist '((\"local\" . handler))) (inhibit-file-name-operation nil) (inhibit-file-name-handlers nil)) {setup} (condition-case e (locate-file-internal \"local\" '(nil) '(\".el\")) (error (car (cdr e)))))");
        assert_eq!(vm.eval_source(&source).unwrap().display().unwrap(), "\"VM2 locate-file-internal matching file name handler is unsupported\"", "{source}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_handler_regexp_signals_without_destroying_match_data() {
    let (root, mut vm) = fixture();
    let source = "(let ((file-name-handler-alist '((\"[\" . handler)))) (string-match \"a\" \"a\") (let ((before (match-data))) (list (condition-case e (locate-file-internal \"local\" '(nil) '(\".el\")) (error (car e))) (equal before (match-data)))))";
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(), "(invalid-regexp t)");
    std::fs::remove_dir_all(root).unwrap();
}
