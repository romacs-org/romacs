use romacs_vm::Vm;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

fn eval(vm: &mut Vm, text: &str) -> String {
    vm.eval_source(text)
        .map(|v| v.display().unwrap())
        .unwrap_or_else(|e| format!("ERROR {e}"))
}
fn q(p: &Path) -> String {
    serde_json::to_string(p.to_str().unwrap()).unwrap()
}
struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("romacs-filesave-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn filesave_writable_and_modes() {
    let f = Fixture::new("modes");
    let file = f.path("file");
    let link = f.path("link");
    std::fs::write(&file, "contents").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
    std::os::unix::fs::symlink(&file, &link).unwrap();
    let mut vm = Vm::new();
    assert_eq!(
        eval(&mut vm, &format!("(file-writable-p {})", q(&file))),
        "t",
        "WRITABLE_EXISTING"
    );
    assert_eq!(
        eval(
            &mut vm,
            &format!("(file-writable-p {})", q(&f.path("missing")))
        ),
        "t",
        "WRITABLE_CREATABLE"
    );
    assert_eq!(
        eval(
            &mut vm,
            &format!(
                "(file-writable-p {})",
                serde_json::to_string(&format!("{}/", f.path("missing").display())).unwrap()
            )
        ),
        "nil",
        "MISSING_TRAILING_SLASH_NOT_CREATABLE"
    );
    assert_eq!(
        eval(
            &mut vm,
            &format!("(file-writable-p {})", q(&f.path("absent/child")))
        ),
        "nil"
    );
    assert_eq!(eval(&mut vm, &format!("(file-modes {})", q(&file))), "416");
    assert_eq!(eval(&mut vm, &format!("(file-modes {})", q(&link))), "416");
    assert_eq!(
        eval(&mut vm, &format!("(file-modes {} 'anything)", q(&link))),
        "511",
        "NOFOLLOW_ANY_NON_NIL"
    );
    assert_eq!(
        eval(&mut vm, &format!("(file-modes {})", q(&f.path("missing")))),
        "nil"
    );
    assert_eq!(
        eval(&mut vm, &format!("(set-file-modes {} 8576)", q(&file))),
        "nil"
    );
    assert_eq!(
        std::fs::metadata(&file).unwrap().mode() & 0o7777,
        0o600,
        "MODE_LOW_TWELVE_BITS"
    );
    assert_eq!(
        eval(
            &mut vm,
            &format!(
                "(condition-case e (set-file-modes {} 511 t) (file-error (car e)))",
                q(&link)
            )
        ),
        "file-error",
        "NOFOLLOW_SYMLINK_CHMOD_REFUSES"
    );
    assert_eq!(std::fs::metadata(&file).unwrap().mode() & 0o7777, 0o600);
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o400)).unwrap();
    // This host's pinned GNU oracle runs as non-root; do not pretend mode bits
    // alone determine access when the suite is deliberately run as root.
    if unsafe { libc::geteuid() } != 0 {
        assert_eq!(
            eval(&mut vm, &format!("(file-writable-p {})", q(&file))),
            "nil",
            "READ_ONLY_ACCESS"
        );
    }
}

#[test]
fn filesave_rename_never_overwrites_without_permission() {
    let f = Fixture::new("rename");
    let a = f.path("a");
    let b = f.path("b");
    std::fs::write(&a, "source").unwrap();
    std::fs::write(&b, "target").unwrap();
    let mut vm = Vm::new();
    assert_eq!(
        eval(
            &mut vm,
            &format!(
                "(condition-case e (rename-file {} {}) (error (car e)))",
                q(&a),
                q(&b)
            )
        ),
        "file-already-exists",
        "RENAME_NO_REPLACE"
    );
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "source");
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "target");
    assert_eq!(
        eval(&mut vm, &format!("(rename-file {} {} t)", q(&a), q(&b))),
        "nil"
    );
    assert!(!a.exists());
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "source");
    let dir = f.path("dir");
    std::fs::create_dir(&dir).unwrap();
    assert_eq!(
        eval(
            &mut vm,
            &format!(
                "(rename-file {} {})",
                q(&b),
                serde_json::to_string(&format!("{}/", dir.display())).unwrap()
            )
        ),
        "nil",
        "TRAILING_SLASH_TARGET"
    );
    assert_eq!(std::fs::read_to_string(dir.join("b")).unwrap(), "source");
    assert_eq!(
        eval(
            &mut vm,
            &format!(
                "(condition-case e (rename-file {} {}) (error (car e)))",
                q(&a),
                q(&b)
            )
        ),
        "file-missing"
    );
}

#[test]
fn filesave_temp_name_does_not_create_or_expand() {
    let f = Fixture::new("temp");
    let prefix = f.path("prefix-");
    let mut vm = Vm::new();
    assert_eq!(
        eval(
            &mut vm,
            &format!(
                "(progn (setq filesave-temp (make-temp-name {})) (list (equal (substring filesave-temp 0 (length {})) {}) (- (length filesave-temp) (length {}))))",
                q(&prefix),
                q(&prefix),
                q(&prefix),
                q(&prefix)
            )
        ),
        "(t 6)",
        "TEMP_NAME_ONLY"
    );
    assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 0);
    assert_eq!(
        eval(
            &mut vm,
            "(let ((file-name-handler-alist '((\".*\" . ignored)))) (equal (substring (make-temp-name \"relative-\") 0 9) \"relative-\"))"
        ),
        "t",
        "TEMP_NO_HANDLER_OR_EXPANSION"
    );
}

#[test]
fn filesave_faults_and_handler_reentry() {
    let mut vm = Vm::new();
    for form in [
        "(file-writable-p 1)",
        "(make-temp-name nil)",
        "(file-modes 1)",
        "(rename-file 1 2)",
    ] {
        assert_eq!(
            eval(&mut vm, &format!("(condition-case e {form} (error e))")),
            "(wrong-type-argument stringp 1)"
                .replace(" 1)", if form.contains("nil") { " nil)" } else { " 1)" })
        );
    }
    assert_eq!(
        eval(
            &mut vm,
            "(condition-case e (set-file-modes 1 nil) (error e))"
        ),
        "(wrong-type-argument fixnump nil)"
    );
    assert_eq!(
        eval(
            &mut vm,
            "(condition-case e (file-modes \"a\\0b\") (error (car e)))"
        ),
        "wrong-type-argument"
    );
    assert_eq!(
        eval(
            &mut vm,
            "(progn (defun filesave-handler (op &rest args) (cons op args)) (put 'filesave-handler 'operations '(file-writable-p file-modes set-file-modes rename-file)) (setq file-name-handler-alist '((\"/filesave-magic/\" . filesave-handler))) (file-writable-p \"/filesave-magic/a\"))"
        ),
        "(file-writable-p \"/filesave-magic/a\")"
    );
    assert_eq!(
        eval(&mut vm, "(file-modes \"/filesave-magic/a\" 'nofollow)"),
        "(file-modes \"/filesave-magic/a\" nofollow)"
    );
    assert_eq!(
        eval(&mut vm, "(set-file-modes \"/filesave-magic/a\" 384 t)"),
        "(set-file-modes \"/filesave-magic/a\" 384 t)"
    );
    assert_eq!(
        eval(
            &mut vm,
            "(rename-file \"/filesave-magic/a\" \"/filesave-magic/b\" t)"
        ),
        "(rename-file \"/filesave-magic/a\" \"/filesave-magic/b\" t)"
    );
    assert_eq!(
        eval(&mut vm, "(funcall 'file-modes \"/filesave-magic/a\")"),
        "(file-modes \"/filesave-magic/a\" nil)"
    );
    assert_eq!(
        eval(
            &mut vm,
            "(rename-file \"/ordinary-source\" \"/filesave-magic/b\")"
        ),
        "(rename-file \"/ordinary-source\" \"/filesave-magic/b\" nil)",
        "RENAME_DESTINATION_HANDLER"
    );
    let f = Fixture::new("handler");
    let file = f.path("file");
    std::fs::write(&file, "handler").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(
        eval(
            &mut vm,
            "(progn (setq file-name-handler-alist nil) (defun filesave-reenter (op name flag) (let ((inhibit-file-name-operation op) (inhibit-file-name-handlers '(filesave-reenter))) (funcall op name flag))) (put 'filesave-reenter 'operations '(file-modes)) (setq file-name-handler-alist '((\".*\" . filesave-reenter))) t)"
        ),
        "t"
    );
    assert_eq!(
        eval(&mut vm, &format!("(file-modes {})", q(&file))),
        "416",
        "HANDLER_REENTRY_INHIBITED"
    );
}
