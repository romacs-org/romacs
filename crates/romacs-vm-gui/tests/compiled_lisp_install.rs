//! An installer's `--compile-lisp-tree`, as forge's install runs it: the
//! executable in an installation's `bin/` with its fresh compiled-Lisp
//! store beside it, the runtime tree in `share/romacs`, and the installer's
//! own environment, a user's: a private HOME, XDG directories and TMPDIR,
//! no ROMACS_ variable but the tree's two, and LOGNAME and USER set, which
//! no compile process has (src/batch.rs `compile_environment`). The tree's
//! process keys what its compile processes made by the compile world they
//! built, asked as they ask it: a world it judged in its own environment
//! was stale for it, and the run ended after its first compile.
use std::path::Path;
use std::process::Command;

#[test]
fn an_installers_tree_compile_keys_by_the_world_its_first_compile_built() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("compile-install-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (bin, home) = (root.join("bin"), root.join("home"));
    for dir in ["config", "cache", "data", "state", "runtime", "tmp"] {
        std::fs::create_dir_all(home.join(dir)).unwrap();
    }
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(root.join("share")).unwrap();
    let exe = bin.join("romacs-vm-gui");
    let source = Path::new(env!("CARGO_BIN_EXE_romacs-vm-gui"));
    std::fs::hard_link(source, &exe).or_else(|_| std::fs::copy(source, &exe).map(drop)).unwrap();
    let tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    std::os::unix::fs::symlink(&tree, root.join("share/romacs")).unwrap();
    let output = Command::new(&exe)
        .args(["--compile-lisp-tree", "--jobs", "4"])
        .current_dir(&root)
        .env_clear().env("PATH", "/usr/bin:/bin").env("LANG", "C.UTF-8")
        .env("LOGNAME", "romacs-installer").env("USER", "romacs-installer")
        .env("HOME", &home).env("XDG_CONFIG_HOME", home.join("config")).env("XDG_CACHE_HOME", home.join("cache"))
        .env("XDG_DATA_HOME", home.join("data")).env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_RUNTIME_DIR", home.join("runtime")).env("TMPDIR", home.join("tmp"))
        .env("ROMACS_RUNTIME_ROOT", root.join("share/romacs")).env("ROMACS_LISP_DIR", root.join("share/romacs/lisp"))
        .envs(std::env::var_os("LD_LIBRARY_PATH").map(|v| ("LD_LIBRARY_PATH", v)))
        .output().unwrap();
    let (stdout, stderr) = (String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    // Status 2: only files that failed to compile, which the summary lists.
    assert!(output.status.success() || output.status.code() == Some(2),
            "--compile-lisp-tree: {}\n{stdout}\n{stderr}", output.status);
    let line = stdout.lines().rev().find(|line| line.starts_with('{')).unwrap_or_else(|| panic!("no summary: {stdout}\n{stderr}"));
    let summary: serde_json::Value = serde_json::from_str(line).unwrap();
    assert!(summary["store"].as_str().is_some_and(|store| Path::new(store) == bin.join("romacs-vm-gui.lisp")),
            "the installation's store: {summary}");
    let empty = |field: &str| summary[field].as_array().is_none_or(Vec::is_empty);
    assert!(empty("failed") && empty("lost"), "files without a usable artifact: {summary}");
    assert!(summary["files"].as_u64().is_some_and(|n| n > 1000) && summary["compiled"].as_u64().is_some_and(|n| n > 1000),
            "the run did not compile the tree: {summary}");
    let _ = std::fs::remove_dir_all(&root);
}
