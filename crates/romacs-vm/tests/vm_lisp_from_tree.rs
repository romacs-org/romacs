//! The VM's own Lisp (lisp/vm/) is its runtime tree's: an edited file there
//! is what a VM runs, its preludes when they are installed and its Lisp
//! halves on their first use. No copy is compiled into the executable.
#[path = "support/own_test.rs"]
mod own_test;

use romacs_vm::Vm;
use std::path::{Path, PathBuf};

/// This checkout's lisp/vm/.
fn checkout_vm_lisp() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lisp/vm")
}

/// A runtime tree of links to this checkout's lisp/vm/ and etc/, but for
/// FILES (under lisp/) of its own.
fn tree(name: &str, files: &[(&str, String)]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("romacs-vm-lisp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("lisp/vm")).unwrap();
    std::os::unix::fs::symlink(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../etc"), root.join("etc")).unwrap();
    for entry in std::fs::read_dir(checkout_vm_lisp()).unwrap() {
        let path = entry.unwrap().path();
        std::os::unix::fs::symlink(&path, root.join("lisp/vm").join(path.file_name().unwrap())).unwrap();
    }
    for (file, text) in files {
        let path = root.join("lisp").join(file);
        std::fs::remove_file(&path).unwrap();
        std::fs::write(path, text).unwrap();
    }
    root
}

/// A tree whose prelude.el ends in one more definition and whose
/// message.el's `(progn ...)` begins with one: a VM made over it has the
/// first once its prelude is installed, and the second once `message`
/// logs, which installs that half.
#[test]
fn a_vm_runs_the_lisp_its_tree_holds_now() {
    if std::env::var_os("ROMACS_VM_LISP_TREE_CHILD").is_some() {
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let ran = vm.eval_source("(progn (message \"%s\" \"logged\")
                                         (list (boundp 'romacs-test-tree-prelude) (boundp 'romacs-test-tree-half)))")
            .unwrap().display().unwrap();
        assert_eq!(ran, "(t t)", "the VM ran another lisp/vm/ than its tree's");
        return;
    }
    let read = |file: &str| std::fs::read_to_string(checkout_vm_lisp().join(file)).unwrap();
    let prelude = read("prelude.el") + "\n(defvar romacs-test-tree-prelude t)\n";
    let message = read("message.el").replacen("(progn", "(progn (defvar romacs-test-tree-half t)", 1);
    let root = tree("edited", &[("vm/prelude.el", prelude), ("vm/message.el", message)]);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &own_test::name(module_path!(), "a_vm_runs_the_lisp_its_tree_holds_now"), "--nocapture"])
        .env("ROMACS_VM_LISP_TREE_CHILD", "1").env("ROMACS_RUNTIME_ROOT", &root)
        .output().unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert!(output.status.success(), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

/// A process with no runtime tree gets the reason when its VM's prelude
/// is installed, and goes on: none ends the process or runs a copy.
#[test]
fn a_vm_without_a_tree_is_told_why() {
    if std::env::var_os("ROMACS_VM_LISP_TREE_CHILD").is_some() {
        let mut vm = Vm::new();
        let error = vm.install_lisp_prelude().map(|_| ()).unwrap_err();
        assert!(error.contains("ROMACS_RUNTIME_ROOT"), "{error}");
        return;
    }
    let missing = std::env::temp_dir().join(format!("romacs-vm-lisp-none-{}", std::process::id()));
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &own_test::name(module_path!(), "a_vm_without_a_tree_is_told_why"), "--nocapture"])
        .env("ROMACS_VM_LISP_TREE_CHILD", "1").env("ROMACS_RUNTIME_ROOT", &missing)
        .output().unwrap();
    assert!(output.status.success(), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

/// A prelude that signals fails the install with an error naming its file.
#[test]
fn a_prelude_that_signals_names_its_file() {
    if std::env::var_os("ROMACS_VM_LISP_TREE_CHILD").is_some() {
        let mut vm = Vm::new();
        let error = vm.install_lisp_prelude().map(|_| ()).unwrap_err();
        assert!(error.contains("lisp/vm/prelude.el: ") && error.contains("broken on purpose"), "{error}");
        return;
    }
    let prelude = std::fs::read_to_string(checkout_vm_lisp().join("prelude.el")).unwrap() + "\n(error \"broken on purpose\")\n";
    let root = tree("broken", &[("vm/prelude.el", prelude)]);
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &own_test::name(module_path!(), "a_prelude_that_signals_names_its_file"), "--nocapture"])
        .env("ROMACS_VM_LISP_TREE_CHILD", "1").env("ROMACS_RUNTIME_ROOT", &root)
        .output().unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert!(output.status.success(), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}
