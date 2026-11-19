//! Compiled Lisp is keyed by its compile world, and that world by the Lisp
//! its build read, checked again when a process first asks
//! (`World::lisp_files_digest`): a library compiled against a macro that
//! world preloads is not run once the macro's file has changed, in a
//! session that boots from source as in a tree's compile, though nothing
//! the library's own compile read has changed.
use std::path::Path;
use std::process::Command;

const USER: &str = ";;; -*- lexical-binding: t -*-\n(defun t-key-f () (t-key-macro))\n(provide 't-key-user)\n";

/// A runtime tree under ROOT that is the repository's but for its
/// `lisp/subr.el`, a copy that ends with `t-key-macro` expanding to VALUE.
fn tree(root: &Path, value: u8) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let lisp = root.join("tree/lisp");
    if !lisp.exists() {
        std::fs::create_dir_all(&lisp).unwrap();
        for entry in std::fs::read_dir(&repo).unwrap().flatten() {
            if entry.file_name() != "lisp" && entry.file_name() != "target" {
                std::os::unix::fs::symlink(entry.path(), root.join("tree").join(entry.file_name())).unwrap();
            }
        }
        for entry in std::fs::read_dir(repo.join("lisp")).unwrap().flatten() {
            if entry.file_name() != "subr.el" {
                std::os::unix::fs::symlink(entry.path(), lisp.join(entry.file_name())).unwrap();
            }
        }
    }
    let subr = std::fs::read_to_string(repo.join("lisp/subr.el")).unwrap();
    std::fs::write(lisp.join("subr.el"), format!("{subr}\n(defmacro t-key-macro () {value})\n")).unwrap();
}

/// A batch session in the fresh HOME ROOT/NAME over ROOT's tree and
/// store: what `(t-key-f)' gives.
fn session(root: &Path, name: &str) -> String {
    let home = root.join(name);
    for dir in ["config", "cache", "tmp"] {
        std::fs::create_dir_all(home.join(dir)).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(["-Q", "--batch", "-L"]).arg(root.join("lib"))
        .args(["--eval", "(progn (require 't-key-user) (princ (t-key-f)))"])
        .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", &home).env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_CACHE_HOME", home.join("cache")).env("TMPDIR", home.join("tmp")).env("LANG", "C.UTF-8")
        .env("ROMACS_RUNTIME_ROOT", root.join("tree"))
        .env("ROMACS_COMPILED_LISP", "on").env("ROMACS_COMPILED_LISP_STORE", root.join("store"))
        .envs(std::env::var_os("LD_LIBRARY_PATH").map(|v| ("LD_LIBRARY_PATH", v)))
        .output().unwrap();
    assert!(output.status.success(), "{name}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The lines of ROOT's store's use logs that name the library.
fn uses(root: &Path) -> usize {
    std::fs::read_dir(root.join("store")).into_iter().flatten().flatten()
        .filter_map(|g| std::fs::read_to_string(g.path().join("use.log")).ok())
        .map(|log| log.lines().filter(|l| l.ends_with("/lib/t-key-user.el")).count())
        .sum()
}

#[test]
fn a_changed_preloaded_macro_is_not_run_from_a_stale_world_key() {
    let root = std::env::temp_dir().join(format!("romacs-world-key-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("lib")).unwrap();
    std::fs::write(root.join("lib/t-key-user.el"), USER).unwrap();
    // A sibling `.elc`: GNU would load compiled code, so Romacs does.
    std::fs::write(root.join("lib/t-key-user.elc"), "").unwrap();
    tree(&root, 1);
    assert_eq!(session(&root, "a"), "1");
    let before = uses(&root);
    assert_eq!(session(&root, "b"), "1");
    assert!(uses(&root) > before, "the second session did not load the library's artifact");
    // The macro the compile world preloaded changes; the library does not.
    tree(&root, 2);
    assert_eq!(session(&root, "c"), "2", "a session ran the expansion of a superseded world");
    let _ = std::fs::remove_dir_all(&root);
}
