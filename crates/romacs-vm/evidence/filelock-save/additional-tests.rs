
#[test]
fn disabled_locking_still_checks_supersession() {
    let p = dir("supersession").join("target"); let q = quoted(&p); let lock = p.with_file_name(".#target");
    let mut vm = Vm::new(); prepare(&mut vm, &lock);
    assert_eq!(eval(&mut vm, &format!("(write-region \"\" nil {q} nil t)")), "nil");
    eval(&mut vm, "(defun userlock--ask-user-about-supersession-threat (f) (setq fl-threat f))");
    assert_eq!(eval(&mut vm, &format!("(progn (set-visited-file-modtime -1) (set-buffer-modified-p t) (let ((create-lockfiles nil)) (lock-buffer)) (equal fl-threat {q}))")), "t");
    assert!(std::fs::symlink_metadata(&lock).is_err());
}
#[test]
fn nil_lockname_and_unlock_error_callback() {
    let p = dir("unlock-error").join("target"); let q = quoted(&p); let lock = p.with_file_name(".#target");
    let mut vm = Vm::new();
    eval(&mut vm, "(defun make-lock-file-name (f) nil)");
    assert_eq!(eval(&mut vm, &format!("(file-locked-p {q})")), "nil");
    prepare(&mut vm, &lock);
    assert_eq!(eval(&mut vm, &format!("(write-region \"\" nil {q} nil t)")), "nil");
    std::fs::write(&lock, b"broken").unwrap();
    eval(&mut vm, "(defun userlock--handle-unlock-error (e) (setq fl-error e))");
    assert_eq!(eval(&mut vm, "(progn (set-buffer-modified-p t) (unlock-buffer) (list (car fl-error) (nth 1 fl-error) (nth 2 fl-error)))"), "(file-error \"Unlocking file\" \"Invalid argument\")");
    assert_eq!(std::fs::read(&lock).unwrap(), b"broken");
}
