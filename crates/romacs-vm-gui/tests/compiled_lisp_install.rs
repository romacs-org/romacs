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
//! so, took no artifact and compiled every file it loaded again. The
//! session's own count says so (`romacs-compiled-lisp-stats`).
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
    // that holds is taken, and no load compiles (`romacs-compiled-lisp-stats',
    // written to a file in its HOME). A load that reads source does so only
    // for a file whose compile refused it (`not-compiled'), as the tree's
    // compile found. With no compile identity, every load compiled again.
    let form = "(progn (require 'dired-x)
      (let ((s (romacs-compiled-lisp-stats)))
        (with-temp-file (expand-file-name \"stats\" (getenv \"HOME\"))
          (prin1 (list (featurep 'dired-x) (plist-get s :taken) (plist-get s :compiled)
                       (mapcar #'cdr (plist-get s :misses)))
                 (current-buffer)))))";
    let mut session = Command::new(&exe);
    user(session.args(["--batch", "--eval", form]));
    let output = session.env("ROMACS_COMPILED_LISP", "on").env("ROMACS_COMPILED_LISP_STORE", bin.join("romacs-vm-gui.lisp"))
        .output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "the session: {}\n{stderr}", output.status);
    let written = std::fs::read_to_string(home.join("stats")).unwrap_or_else(|e| panic!("the session wrote no stats ({e}): {stderr}"));
    let inner = written.trim().strip_prefix('(').and_then(|text| text.strip_suffix(')')).unwrap_or_else(|| panic!("{written}"));
    let mut fields = inner.splitn(4, ' ');
    let (loaded, taken, compiled, misses) = (fields.next(), fields.next(), fields.next(), fields.next().unwrap_or("nil"));
    assert_eq!(loaded, Some("t"), "the session did not load dired-x: {written}");
    assert_eq!(compiled, Some("0"), "the session compiled what the tree did: {written}");
    assert!(taken.and_then(|n| n.parse::<u64>().ok()).is_some_and(|n| n > 0), "the session took no artifact: {written}");
    let reasons = misses.trim_start_matches('(').trim_end_matches(')');
    assert!(reasons == "nil" || reasons.split_whitespace().all(|reason| reason == "not-compiled"),
            "a load read source for want of an artifact: {written}");
    let _ = std::fs::remove_dir_all(&root);
}
