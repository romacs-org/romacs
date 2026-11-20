//! Compiled Lisp holds over a copy of the files its compile read the
//! attributes of: a library that asks a file's attributes as it loads is
//! compiled once, then the file is replaced by a copy with its content and
//! times (another inode, as a restore or a reflink of an installation
//! gives), and the next session takes the artifact, compiling nothing.
//! A change of the file's content or times would still recompile it.
use std::path::Path;
use std::process::Command;

/// A batch session in the fresh HOME ROOT/NAME over ROOT's store, LIB on
/// its `load-path': how many files it compiled (traced).
fn session(root: &Path, name: &str) -> usize {
    let home = root.join(name);
    std::fs::create_dir_all(home.join("tmp")).unwrap();
    let tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(["-Q", "--batch", "-L"]).arg(root.join("lib"))
        .args(["--eval", "(progn (require 't-copied) (princ t-copied-size))"])
        .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", &home).env("TMPDIR", home.join("tmp")).env("LANG", "C.UTF-8")
        .env("ROMACS_RUNTIME_ROOT", &tree)
        .env("ROMACS_COMPILED_LISP", "on").env("ROMACS_COMPILED_LISP_STORE", root.join("store"))
        .env("ROMACS_COMPILED_LISP_TRACE", "1")
        .envs(std::env::var_os("LD_LIBRARY_PATH").map(|v| ("LD_LIBRARY_PATH", v)))
        .output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{name}: {stderr}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "4", "{name}: {stderr}");
    stderr.lines().filter(|line| line.starts_with("romacs-compiled-lisp: {\"compile\"") && line.contains("t-copied")).count()
}

#[test]
fn compiled_lisp_holds_over_a_copy_of_the_files_it_stat() {
    use std::os::unix::fs::MetadataExt;
    let root = std::env::temp_dir().join(format!("romacs-copied-inputs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("lib")).unwrap();
    let data = root.join("lib/t-copied.dat");
    std::fs::write(&data, "data").unwrap();
    std::fs::write(root.join("lib/t-copied.el"), format!(";;; -*- lexical-binding: t -*-\n\
        (defvar t-copied-size (file-attribute-size (file-attributes {:?})))\n\
        (provide 't-copied)\n", data.to_str().unwrap())).unwrap();
    // A sibling `.elc`: GNU would load compiled code, so Romacs does.
    std::fs::write(root.join("lib/t-copied.elc"), "").unwrap();
    assert_eq!(session(&root, "a"), 1, "the first session compiles the library");
    // The file replaced by a copy of it, its times kept: another inode.
    let inode = std::fs::metadata(&data).unwrap().ino();
    let copied = Command::new("cp").arg("-p").arg(&data).arg(root.join("lib/t-copied.new")).status().unwrap();
    assert!(copied.success());
    std::fs::rename(root.join("lib/t-copied.new"), &data).unwrap();
    assert_ne!(std::fs::metadata(&data).unwrap().ino(), inode, "a copy");
    assert_eq!(session(&root, "b"), 0, "the session after the copy took the artifact");
    let _ = std::fs::remove_dir_all(&root);
}
