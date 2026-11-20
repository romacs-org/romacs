//! An installation never names the checkout it was built from, which its
//! installer deletes: with the executable in an installation's `bin/` and
//! its runtime tree a copy of the checkout's, `source-directory` is nil
//! (the tree has no `source` link), though the build's checkout is still
//! there, and neither the compile world nor the compiled Lisp the session
//! makes records a path of it. A world or artifact that did went stale for
//! every process once the checkout was gone, and the installation lost its
//! compiled Lisp.
use std::path::Path;
use std::process::Command;

#[test]
fn an_installation_never_names_the_checkout_it_was_built_from() {
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("installed-source-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (bin, share, lib, home, store) = (root.join("bin"), root.join("share/romacs"), root.join("lib"), root.join("home"), root.join("store"));
    for dir in [&bin, &share, &lib, &home.join("tmp")] {
        std::fs::create_dir_all(dir).unwrap();
    }
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&store).unwrap();
    }
    // The runtime tree, a copy of the checkout's (hard links): another tree.
    for dir in ["lisp", "etc"] {
        let copied = Command::new("cp").arg("-al").arg(checkout.join(dir)).arg(&share).status().unwrap();
        assert!(copied.success(), "copying {dir}");
    }
    let exe = bin.join("romacs-vm-gui");
    let source = Path::new(env!("CARGO_BIN_EXE_romacs-vm-gui"));
    std::fs::hard_link(source, &exe).or_else(|_| std::fs::copy(source, &exe).map(drop)).unwrap();
    // A library that asks where the sources are as it loads, as find-func does.
    std::fs::write(lib.join("t-source.el"), ";;; -*- lexical-binding: t -*-\n\
        (defvar t-source-c (and source-directory (file-directory-p (expand-file-name \"src\" source-directory))))\n\
        (provide 't-source)\n").unwrap();
    // A sibling `.elc`: GNU would load compiled code, so Romacs does.
    std::fs::write(lib.join("t-source.elc"), "").unwrap();
    let output = Command::new(&exe)
        .args(["-Q", "--batch", "-L"]).arg(&lib)
        .args(["--eval", "(progn (require 't-source) (princ (format \"%S\" source-directory)))"])
        .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", &home).env("TMPDIR", home.join("tmp")).env("LANG", "C.UTF-8")
        .env("ROMACS_RUNTIME_ROOT", &share).env("ROMACS_LISP_DIR", share.join("lisp"))
        .env("ROMACS_COMPILED_LISP", "on").env("ROMACS_COMPILED_LISP_STORE", &store)
        .envs(std::env::var_os("LD_LIBRARY_PATH").map(|v| ("LD_LIBRARY_PATH", v)))
        .output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "nil", "the session's source-directory: {stderr}");
    // Nothing the store holds, the compile world's manifest and the
    // library's artifact among them, names the checkout.
    // Its sources by name (the target directory may lie in it).
    let names: Vec<Vec<u8>> = ["crates", ".git", "src"].iter()
        .map(|name| format!("{}/{name}", checkout.display()).into_bytes()).collect();
    let mut naming = Vec::new();
    let mut held = 0;
    let mut walk = vec![store.clone()];
    while let Some(dir) = walk.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                walk.push(path);
            } else if matches!(path.extension().and_then(|e| e.to_str()), Some("json" | "rlc")) {
                held += 1;
                let bytes = std::fs::read(&path).unwrap();
                if names.iter().any(|name| bytes.windows(name.len()).any(|w| w == name.as_slice())) {
                    naming.push(path.display().to_string());
                }
            }
        }
    }
    assert!(held >= 2, "a compile world and an artifact: {held} files");
    assert!(naming.is_empty(), "files naming the checkout {}: {naming:?}", checkout.display());
    let _ = std::fs::remove_dir_all(&root);
}
