//! GNU filelock.c contracts; live oracle and scope in evidence/filelock-save.

include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/pinned_gnu.rs"));

struct LiveReviewGnu(std::process::Child);
impl Drop for LiveReviewGnu {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn review_live_gnu_lock_is_retained_by_candidate() {
    let root = dir("review-live-gnu");
    let file = root.join("target");
    let lock = root.join(".#target");
    let ready = root.join("ready");
    std::fs::write(&file, b"original").unwrap();
    let source = format!(
        "(progn (lock-file {}) (write-region \"ready\" nil {} nil 'silent) (while t (sleep-for 0.05)))",
        quoted(&file), quoted(&ready)
    );
    let mut gnu = LiveReviewGnu(std::process::Command::new(pinned_gnu())
        .args(["--batch", "-Q", "--eval", &source])
        .stdin(std::process::Stdio::null()).spawn().expect("pinned GNU is required"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !ready.exists() {
        assert!(gnu.0.try_wait().unwrap().is_none(), "GNU exited before acquiring lock");
        assert!(std::time::Instant::now() < deadline, "GNU lock readiness timeout");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let identity = std::fs::read_link(&lock).expect("GNU must create a real lock");
    println!("GNU_CREATED_LOCK {}", identity.display());
    assert!(gnu.0.try_wait().unwrap().is_none());
    let mut vm = Vm::new();
    prepare(&mut vm, &lock);
    // Compare the owner through Lisp to avoid string representation differences.
    let user = identity.to_str().unwrap().rsplit_once('@').unwrap().0;
    let result = eval(&mut vm, &format!("(equal (file-locked-p {}) {})", quoted(&file), serde_json::to_string(user).unwrap()));
    let retained = std::fs::read_link(&lock);
    assert!(gnu.0.try_wait().unwrap().is_none(), "GNU must remain alive through query");
    assert_eq!(result, "t", "LIVE_GNU_OWNER_RECOGNIZED; retained={retained:?}");
    assert_eq!(retained.unwrap(), identity, "LIVE_GNU_LOCK_UNCHANGED");
    drop(gnu);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn review_candidate_lock_is_retained_by_pinned_gnu() {
    let root = dir("review-candidate-lock");
    let file = root.join("target");
    let lock = root.join(".#target");
    let mut vm = Vm::new();
    prepare(&mut vm, &lock);
    assert_eq!(eval(&mut vm, &format!("(progn (write-region \"\" nil {} nil t) (set-buffer-modified-p t) (lock-buffer))", quoted(&file))), "nil");
    let identity = std::fs::read_link(&lock).expect("candidate must create a real lock");
    println!("CANDIDATE_CREATED_LOCK {}", identity.display());
    let user = identity.to_str().unwrap().rsplit_once('@').unwrap().0;
    let source = format!("(princ (equal (file-locked-p {}) {}))", quoted(&file), serde_json::to_string(user).unwrap());
    let output = std::process::Command::new(pinned_gnu()).args(["--batch", "-Q", "--eval", &source])
        .output().expect("pinned GNU is required");
    assert!(output.status.success(), "GNU query failed: {output:?}");
    let result = String::from_utf8(output.stdout).unwrap();
    println!("GNU_EXPECTED_OWNER_QUERY {result}");
    let retained = std::fs::read_link(&lock);
    assert_eq!(result, "t", "GNU_RECOGNIZES_CANDIDATE_OWNER; retained={retained:?}");
    assert_eq!(retained.unwrap(), identity, "GNU_RETAINS_CANDIDATE_LOCK");
    assert_eq!(eval(&mut vm, "(unlock-buffer)"), "nil");
    assert!(std::fs::symlink_metadata(lock).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

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

use romacs_vm::Vm;
use std::path::{Path, PathBuf};
fn eval(vm: &mut Vm, s: &str) -> String {
    match vm.eval_source(s) { Ok(v) => v.display().unwrap(), Err(e) => format!("VM-ERROR {e}") }
}
fn quoted(p: &Path) -> String { serde_json::to_string(p.to_str().unwrap()).unwrap() }
fn dir(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("romacs-filelock-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap(); p
}
fn prepare(vm: &mut Vm, lock: &Path) {
    assert_eq!(eval(vm, &format!("(defun make-lock-file-name (f) {})", quoted(lock))), "make-lock-file-name");
}
#[test]
fn callable_and_buffer_guards() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(list (fboundp 'lock-buffer) (fboundp 'unlock-buffer) (fboundp 'file-locked-p) (functionp #'lock-buffer) (lock-buffer) (funcall #'unlock-buffer))"), "(t t t t nil nil)", "FILELOCK_CALLABLE");
    assert_eq!(eval(&mut vm, "(condition-case e (lock-buffer 17) (error e))"), "(wrong-type-argument stringp 17)");
    assert_eq!(eval(&mut vm, "(condition-case e (file-locked-p 17) (error e))"), "(wrong-type-argument stringp 17)");
}
#[test]
fn handlers_use_file_operations_not_buffer_operations() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(defun fl-handler (op file) (setq fl-seen (list op file)) 'handled)"), "fl-handler");
    assert_eq!(eval(&mut vm, "(let ((file-name-handler-alist '((\"remote:\" . fl-handler)))) (list (file-locked-p \"remote:a\") fl-seen))"), "(handled (file-locked-p \"remote:a\"))");
    assert_eq!(eval(&mut vm, "(let ((file-name-handler-alist '((\"remote:\" . fl-handler)))) (set-buffer-modified-p t) (list (lock-buffer \"remote:a\") fl-seen))"), "(nil (lock-file \"remote:a\"))");
    // Flock_file returns its handler's value; Flock_buffer and Funlock_file nil.
    assert_eq!(eval(&mut vm, "(let ((file-name-handler-alist '((\"remote:\" . fl-handler)))) (list (lock-file \"remote:a\") (unlock-file \"remote:b\") fl-seen))"), "(handled nil (unlock-file \"remote:b\"))");
    let p = dir("handler").join("target"); let q = quoted(&p);
    assert_eq!(eval(&mut vm, &format!("(write-region \"\" nil {q} nil t)")), "nil");
    eval(&mut vm, "(set-buffer-modified-p t)");
    assert_eq!(eval(&mut vm, "(let ((file-name-handler-alist '((\"target\" . fl-handler)))) (list (unlock-buffer) fl-seen))"), format!("(nil (unlock-file {q}))"));
}
#[test]
fn real_lock_ownership_and_modified_guards() {
    let p = dir("ownership").join("target"); let q = quoted(&p);
    let lock = p.with_file_name(".#target");
    let mut vm = Vm::new(); prepare(&mut vm, &lock);
    assert_eq!(eval(&mut vm, &format!("(progn (write-region \"\" nil {q} nil t) (lock-buffer) (file-locked-p {q}))")), "nil");
    assert!(!lock.exists() && std::fs::symlink_metadata(&lock).is_err());
    assert_eq!(eval(&mut vm, &format!("(progn (set-buffer-modified-p t) (lock-buffer) (file-locked-p {q}))")), "t");
    assert!(std::fs::read_link(&lock).is_ok(), "FILELOCK_IS_REAL_INTERLOCK");
    assert_eq!(eval(&mut vm, &format!("(progn (lock-buffer) (unlock-buffer) (file-locked-p {q}))")), "nil");
    assert!(std::fs::symlink_metadata(&lock).is_err());
}
#[test]
fn foreign_stale_empty_malformed_and_regular_locks() {
    use std::os::unix::fs::symlink;
    let p = dir("owners").join("target"); let q = quoted(&p); let lock = p.with_file_name(".#target");
    let mut vm = Vm::new(); prepare(&mut vm, &lock);
    symlink("alice@foreign.invalid.23:17", &lock).unwrap();
    assert_eq!(eval(&mut vm, &format!("(equal (file-locked-p {q}) \"alice\")")), "t");
    std::fs::remove_file(&lock).unwrap();
    std::fs::write(&lock, b"bob@foreign.invalid.42").unwrap();
    assert_eq!(eval(&mut vm, &format!("(equal (file-locked-p {q}) \"bob\")")), "t");
    std::fs::write(&lock, b"garbage").unwrap();
    assert_eq!(eval(&mut vm, &format!("(condition-case e (file-locked-p {q}) (error (list (car e) (nth 1 e) (nth 2 e))))")), "(file-error \"Testing file lock\" \"Invalid argument\")");
    std::fs::write(&lock, b"").unwrap();
    assert_eq!(eval(&mut vm, &format!("(file-locked-p {q})")), "nil");
    assert!(std::fs::symlink_metadata(&lock).is_err());
    let host = eval(&mut vm, "(system-name)");
    let host: String = serde_json::from_str(&host).unwrap();
    symlink(format!("gone@{host}.-1"), &lock).unwrap();
    assert_eq!(eval(&mut vm, &format!("(file-locked-p {q})")), "nil");
    assert!(std::fs::symlink_metadata(&lock).is_err());
}
#[test]
fn conflict_callback_can_decline_or_steal() {
    use std::os::unix::fs::symlink;
    let p = dir("conflict").join("target"); let q = quoted(&p); let lock = p.with_file_name(".#target");
    let mut vm = Vm::new(); prepare(&mut vm, &lock);
    symlink("alice@foreign.invalid.23:17", &lock).unwrap();
    eval(&mut vm, "(defun ask-user-about-lock (file owner) (setq fl-owner owner) nil)");
    assert_eq!(eval(&mut vm, &format!("(progn (set-buffer-modified-p t) (lock-buffer {q}) (equal (list fl-owner (file-locked-p {q})) '(\"alice@foreign.invalid (pid 23)\" \"alice\")))")), "t");
    eval(&mut vm, "(defun ask-user-about-lock (file owner) t)");
    assert_eq!(eval(&mut vm, &format!("(progn (lock-buffer {q}) (file-locked-p {q}))")), "t");
    eval(&mut vm, &format!("(write-region \"\" nil {q} nil t)"));
    eval(&mut vm, "(set-buffer-modified-p t)");
    assert_eq!(eval(&mut vm, "(unlock-buffer)"), "nil");
    assert!(std::fs::symlink_metadata(&lock).is_err());
}
#[test]
fn indirect_buffer_edits_lock_the_base_file_as_pinned_gnu() {
    let p = dir("indirect").join("target"); let q = quoted(&p);
    let source = format!("(progn (write-region \"\" nil {q} nil t) (setq buffer-file-truename {q}) \
        (let ((r nil)) (with-current-buffer (make-indirect-buffer (current-buffer) \"fl-indirect\") \
          (insert \"x\") (push (file-locked-p {q}) r) (set-buffer-modified-p nil) (push (file-locked-p {q}) r) \
          (set-buffer-modified-p t) (push (file-locked-p {q}) r) (set-buffer-modified-p nil)) (nreverse r)))");
    let output = std::process::Command::new(pinned_gnu()).args(["--batch", "-Q", "--eval", &format!("(prin1 {source})")])
        .output().expect("pinned GNU is required");
    assert!(output.status.success(), "GNU failed: {output:?}");
    let gnu = String::from_utf8(output.stdout).unwrap();
    assert_eq!(gnu, "(t nil t)");
    let mut vm = Vm::new(); prepare(&mut vm, &p.with_file_name(".#target"));
    assert_eq!(eval(&mut vm, &source), gnu);
    assert!(std::fs::symlink_metadata(p.with_file_name(".#target")).is_err());
}
