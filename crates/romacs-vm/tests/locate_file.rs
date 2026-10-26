use romacs_vm::Vm;

#[test]
fn locate_file_searches_paths_and_suffixes_without_loading() {
    let root = std::env::temp_dir().join(format!("romacs-locate-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for dir in ["first", "second", "first/directory.el"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    for file in ["first/library.el", "first/library.elc", "second/library.elc", "first/plain"] {
        std::fs::write(root.join(file), "(error \"must not load\")").unwrap();
    }
    let mut vm = Vm::new();
    vm.eval_source(&format!("(setq default-directory {:?})", format!("{}/", root.join("first").display()))).unwrap();
    let cases = [
        ("(locate-file-internal \"library\" '(\"first\" \"second\") '(\".el\" \".elc\"))".to_owned(), None),
        ("(locate-file-internal \"plain\" nil)".to_owned(), Some(root.join("first/plain"))),
        ("(locate-file-internal \"library\" '(nil) '(\".elc\" \".el\"))".to_owned(), Some(root.join("first/library.elc"))),
        (format!("(locate-file-internal \"library\" '({:?} {:?}) '(\".el\" \".elc\"))", root.join("first").display().to_string(), root.join("second").display().to_string()), Some(root.join("first/library.el"))),
        ("(funcall 'locate-file-internal \"plain\" '(nil))".to_owned(), Some(root.join("first/plain"))),
        ("(locate-file-internal \"plain\" '(nil 1))".to_owned(), Some(root.join("first/plain"))),
        ("(locate-file-internal \"directory\" '(nil) '(\".el\"))".to_owned(), None),
        ("(locate-file-internal \"absent\" '(nil))".to_owned(), None),
        (format!("(locate-file-internal {:?} nil)", root.join("first/./plain").display().to_string()), Some(root.join("first/./plain"))),
    ];
    for (source, path) in cases {
        let expected = path.map_or_else(|| "nil".to_owned(), |p| format!("{:?}", p.display().to_string()));
        let actual = vm.eval_source(&source).and_then(|v| v.display()).map_err(|e| e.to_string());
        assert_eq!(actual, Ok(expected), "locate-file readable search: {source}");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn locate_file_refuses_unimplemented_predicates_explicitly() {
    let mut vm = Vm::new();
    // Only `locate-file-internal' is native: GNU defines `locate-file' in Lisp
    // (lisp/files.el:1113), so a bare VM must not have it at all.
    for name in ["locate-file-internal"] {
        let source = format!("(condition-case e ({name} \"anything\" nil nil 'file-exists-p) (error (car (cdr e))))");
        let actual = vm.eval_source(&source).unwrap().display().unwrap();
        assert_eq!(actual, "\"VM2 locate-file-internal function predicate is unsupported\"");
    }
    let bare = vm.eval_source("(fboundp 'locate-file)").unwrap().display().unwrap();
    assert_eq!(bare, "nil", "locate-file must come from GNU's files.el, never from the substrate");
}

#[test]
fn locate_file_access_mode_predicates_match_pinned_gnu() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("romacs-locate-mode-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for dir in ["bin", "bin/tool-dir", "other"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    for (file, mode) in [("bin/tool", 0o755), ("bin/data", 0o644), ("other/tool", 0o755), ("bin/secret", 0o000)] {
        std::fs::write(root.join(file), "").unwrap();
        std::fs::set_permissions(root.join(file), std::fs::Permissions::from_mode(mode)).unwrap();
    }
    let path = format!("(list {:?} {:?})", root.join("bin").display().to_string(), root.join("other").display().to_string());
    let mut cases = Vec::new();
    for name in ["tool", "data", "secret", "tool-dir", "absent"] {
        for predicate in ["nil", "t", "0", "1", "2", "4", "5", "4294967296"] {
            cases.push(format!("(locate-file-internal {name:?} {path} nil {predicate})"));
        }
    }
    cases.push(format!("(locate-file-internal \"tool\" {path} '(\".sh\" \"\") 1)"));
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = Vm::new();
    for source in cases {
        let output = std::process::Command::new(&oracle).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display()).map_err(|e| e.to_string());
        assert_eq!(actual.as_deref(), Ok(expected.as_str()), "{source}");
    }
    std::fs::remove_dir_all(root).unwrap();
}
