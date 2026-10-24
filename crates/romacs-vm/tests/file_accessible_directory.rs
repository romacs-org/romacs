use romacs_vm::Vm;
use std::os::unix::fs::{PermissionsExt, symlink};

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap_or_else(|e| e.to_string())
}

struct Fixture(std::path::PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        for name in ["searchable", "unsearchable", "blocked"] {
            let _ = std::fs::set_permissions(self.0.join(name), std::fs::Permissions::from_mode(0o700));
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn file_accessible_directory_requires_directory_and_search_permission() {
    let root = std::env::temp_dir().join(format!("romacs-accessible-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let _fixture = Fixture(root.clone());
    for (name, mode) in [("searchable", 0o100), ("unsearchable", 0o400), ("blocked", 0o000)] {
        let path = root.join(name);
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    std::fs::write(root.join("file"), "").unwrap();
    std::fs::set_permissions(root.join("file"), std::fs::Permissions::from_mode(0o700)).unwrap();
    symlink(root.join("searchable"), root.join("dir-link")).unwrap();
    symlink(root.join("file"), root.join("file-link")).unwrap();
    symlink(root.join("absent"), root.join("dangling")).unwrap();
    let mut vm = Vm::new();
    eval(&mut vm, &format!("(setq default-directory {:?})", format!("{}/", root.display())));
    for (path, expected) in [("", "t"), (".", "t"), ("searchable", "t"), ("searchable/", "t"), ("./searchable/../searchable", "t"), ("dir-link", "t"), ("file", "nil"), ("file/", "nil"), ("file-link", "nil"), ("dangling", "nil"), ("absent", "nil")] {
        let source = format!("(file-accessible-directory-p {path:?})");
        assert_eq!(eval(&mut vm, &source), expected, "accessible-directory directory check: {path}");
        let absolute = root.join(path).display().to_string();
        assert_eq!(eval(&mut vm, &format!("(funcall 'file-accessible-directory-p {absolute:?})")), expected, "accessible-directory absolute/funcall: {path}");
    }
    // Unlike readdir, a lookup of `DIR/./` needs search but not read access.
    // Refuse a privileged fixture instead of silently losing permission coverage.
    assert!(std::fs::metadata(root.join("unsearchable/./")).is_err(), "permission fixture requires an unprivileged process");
    for name in ["unsearchable", "blocked"] {
        assert_eq!(eval(&mut vm, &format!("(file-accessible-directory-p {name:?})")), "nil", "accessible-directory search permission: {name}");
    }
    for name in ["searchable", "unsearchable", "blocked"] {
        std::fs::set_permissions(root.join(name), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn file_accessible_directory_callable_and_type_errors() {
    let mut vm = Vm::new();
    assert_eq!(eval(&mut vm, "(list (fboundp 'file-accessible-directory-p) (functionp 'file-accessible-directory-p))"), "(t t)", "accessible-directory callable registration");
    for value in ["nil", "t", "42", "'symbol", "'(1)", "[1]"] {
        let source = format!("(condition-case e (file-accessible-directory-p {value}) (error e))");
        assert_eq!(eval(&mut vm, &source), format!("(wrong-type-argument stringp {})", eval(&mut vm, value)), "accessible-directory string type: {value}");
    }
}
