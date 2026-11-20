//! An installer's `--compile-lisp-tree`, as forge's install runs it: the
//! executable in an installation's `bin/` with its fresh compiled-Lisp
//! store beside it, the runtime tree in `share/romacs`, and the installer's
//! own environment, a user's: the test's own without its ROMACS_ variables
//! but the tree's two, a private HOME, XDG directories and TMPDIR, and
//! LOGNAME and USER set, which no compile process has (src/batch.rs
//! `compile_environment`). The tree's process keys what its compile
//! processes made by the compile world they built, asked as they ask it: a
//! world it judged in its own environment was stale for it, and the run
//! ended after its first compile; and a session of that user, judging it
//! so, took no artifact and compiled every file it loaded again.
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
    // forge's `isolated_environment`: this process's environment without
    // its ROMACS_ variables, a user's LOGNAME and USER.
    let user = |command: &mut Command| {
        command.current_dir(&root)
            .env_clear().envs(std::env::vars_os().filter(|(name, _)| !name.to_string_lossy().starts_with("ROMACS_")))
            .env("LOGNAME", "romacs-installer").env("USER", "romacs-installer")
            .env("HOME", &home).env("XDG_CONFIG_HOME", home.join("config")).env("XDG_CACHE_HOME", home.join("cache"))
            .env("XDG_DATA_HOME", home.join("data")).env("XDG_STATE_HOME", home.join("state"))
            .env("XDG_RUNTIME_DIR", home.join("runtime")).env("TMPDIR", home.join("tmp"))
            .env("ROMACS_RUNTIME_ROOT", root.join("share/romacs")).env("ROMACS_LISP_DIR", root.join("share/romacs/lisp"));
    };
    let mut tree_compile = Command::new(&exe);
    user(tree_compile.args(["--compile-lisp-tree", "--jobs", "4"]));
    let output = tree_compile.output().unwrap();
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
    // A session of the same user loads what the tree compiled: an artifact
    // that holds is taken, and no load compiles (a compile of a load is
    // traced). With no compile identity, every load compiled again.
    let mut session = Command::new(&exe);
    user(session.args(["--batch", "--eval", "(progn (require 'dired-x) (princ (featurep 'dired-x)))"]));
    let output = session.env("ROMACS_COMPILED_LISP", "on").env("ROMACS_COMPILED_LISP_STORE", bin.join("romacs-vm-gui.lisp"))
        .env("ROMACS_COMPILED_LISP_TRACE", "1").output().unwrap();
    let (stdout, stderr) = (String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success() && stdout.trim() == "t", "the session: {}\n{stdout}\n{stderr}", output.status);
    let compiles: Vec<&str> = stderr.lines().filter(|line| line.starts_with("romacs-compiled-lisp: {\"compile\"")).collect();
    assert!(compiles.is_empty(), "the session compiled what the tree did:\n{}", compiles.join("\n"));
    let _ = std::fs::remove_dir_all(&root);
}
