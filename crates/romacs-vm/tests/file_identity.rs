use romacs_vm::Vm;
fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).map(|v| v.display().unwrap()).unwrap_or_else(|e| format!("ERROR {e}"))
}
#[test]
fn file_identity_native_aliases_and_replacement() {
    let root = std::env::temp_dir().join(format!("romacs-identity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("original");
    let hard = root.join("hard");
    let sym = root.join("sym");
    std::fs::write(&file, "disk").unwrap();
    std::fs::hard_link(&file, &hard).unwrap();
    std::os::unix::fs::symlink(&file, &sym).unwrap();
    let quote = |p: &std::path::Path| serde_json::to_string(p.to_str().unwrap()).unwrap();
    let mut vm = Vm::new();
    vm.set_collection_floor(1_000_000);
    eval(&mut vm, "(let ((i 0)) (while (< i 2000) (copy-sequence \"garbage\") (setq i (1+ i))))");
    assert_eq!(eval(&mut vm, &format!("(progn (setq identity-buffer (get-buffer-create (generate-new-buffer-name \"identity\"))) (set-buffer identity-buffer) (insert-file-contents {} t) (goto-char 1) (insert \"live\") t)", quote(&file))), "t");
    assert_eq!(eval(&mut vm, &format!("(eq (get-file-buffer {}) identity-buffer)", quote(&root.join("./original")))), "t", "IDENTITY_NATIVE_EXPANDED_PATH");
    let before = vm.eval_source("identity-buffer").unwrap();
    let bits = format!("{before:?}");
    let rooted = before.root();
    vm.collect_garbage();
    assert_ne!(format!("{:?}", rooted.get(&mut vm).unwrap()), bits, "IDENTITY_BUFFER_ACTUALLY_MOVED");
    for alias in [&hard, &sym] {
        assert_eq!(eval(&mut vm, &format!("(get-file-buffer {})", quote(alias))), "nil", "IDENTITY_GNU_GET_FILE_BUFFER_IS_PATH_ONLY");
        assert_eq!(eval(&mut vm, &format!("(eq (romacs-find-buffer-visiting {}) identity-buffer)", quote(alias))), "t", "IDENTITY_NATIVE_INODE_ALIAS: {}", alias.display());
    }
    // An atomic replacement must not bind the surviving old hardlink to the
    // buffer now visiting a different inode at its original name.
    let replacement = root.join("replacement");
    std::fs::write(&replacement, "new").unwrap();
    std::fs::rename(&replacement, &file).unwrap();
    assert_eq!(eval(&mut vm, &format!("(romacs-find-buffer-visiting {})", quote(&hard))), "nil", "IDENTITY_REPLACED_VISITED_INODE");
    let new_alias = root.join("new-hard");
    std::fs::hard_link(&file, &new_alias).unwrap();
    assert_eq!(eval(&mut vm, &format!("(romacs-find-buffer-visiting {})", quote(&new_alias))), "nil", "IDENTITY_NEW_INODE_IS_NOT_OLD_VISIT");
    assert_eq!(eval(&mut vm, &format!("(eq (get-file-buffer {}) identity-buffer)", quote(&file))), "t", "IDENTITY_EXACT_NAME_SURVIVES_REPLACEMENT");
    assert_eq!(eval(&mut vm, &format!("(eq (romacs-find-buffer-visiting {}) identity-buffer)", quote(&sym))), "t", "IDENTITY_TRUENAME_SURVIVES_REPLACEMENT");
    std::fs::remove_file(&file).unwrap();
    assert_eq!(eval(&mut vm, &format!("(romacs-find-buffer-visiting {})", quote(&hard))), "nil", "IDENTITY_REMOVED_VISITED_NAME");
    assert_eq!(eval(&mut vm, "(progn (setq buffer-file-name nil) (romacs-find-buffer-visiting \"nonexistent\"))"), "nil", "IDENTITY_CLEARED_VISIT");
    assert_eq!(eval(&mut vm, &format!("(with-temp-buffer (insert-file-contents {} t) (eq (get-file-buffer {}) (current-buffer)))", quote(&hard), quote(&hard))), "t", "IDENTITY_CURRENT_ANONYMOUS_BUFFER");
    std::fs::remove_dir_all(root).unwrap();
}
