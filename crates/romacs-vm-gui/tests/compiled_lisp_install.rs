//! An installer's `--compile-lisp-tree`, as forge's install runs it: the
//! executable in an installation's `bin/` with its fresh compiled-Lisp
//! store beside it, the runtime tree in `share/romacs` (a copy of the
//! checkout's lisp/ and etc/, as an installation has), and the installer's
//! own environment, a user's: the test's own without its ROMACS_ variables
//! but the tree's two, a private HOME, XDG directories and TMPDIR, and
//! LOGNAME and USER set, which no compile process has (src/batch.rs
//! `compile_environment`). The tree's process keys what its compile
//! processes made by the compile world they built, asked as they ask it: a
//! world it judged in its own environment was stale for it, and the run
//! ended after its first compile; and a session of that user, judging it
//! so, took no artifact and compiled every file it loaded again. The
//! session's own count says so (`romacs-compiled-lisp-stats`).
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The `.el` files under DIR, by their true names, as the tree's compile
/// walks it: names starting with a dot left out, at every level.
fn lisp_files(dir: &Path, found: &mut HashSet<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let (path, name) = (entry.path(), entry.file_name());
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        if path.is_dir() {
            lisp_files(&path, found);
        } else if path.extension().is_some_and(|e| e == "el") {
            found.insert(path.canonicalize().unwrap());
        }
    }
}

#[test]
fn an_installers_tree_compile_keys_by_the_world_its_first_compile_built() {
    // Outside the checkout, as an installation is: a compile's `normal-mode'
    // reads the .dir-locals.el of every directory above a file.
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("romacs-compile-install-{}", std::process::id()));
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
    // The runtime tree, a copy of the checkout's lisp/ and etc/ (shared
    // extents where the file system has them), as an installation's is:
    // nothing of the checkout around it (its .dir-locals.el files, its
    // sources) is in reach.
    let (checkout, share) = (Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap(), root.join("share/romacs"));
    assert!(!root.starts_with(&checkout), "the installation {} is inside the checkout: set TMPDIR elsewhere", root.display());
    std::fs::create_dir_all(&share).unwrap();
    for dir in ["lisp", "etc"] {
        let copied = Command::new("cp").args(["-a", "--reflink=auto"]).arg(checkout.join(dir)).arg(&share).status().unwrap();
        assert!(copied.success(), "copying {dir}");
    }
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
    let empty = |field: &str| summary[field].as_array().is_some_and(Vec::is_empty);
    assert!(empty("failed") && empty("lost"), "files without a usable artifact: {summary}");
    // Every file of the runtime trees, lisp/ and the installation's themes,
    // was reached, and each has an outcome.
    let mut inventory = HashSet::new();
    lisp_files(&share.join("lisp"), &mut inventory);
    lisp_files(&share.join("etc/themes"), &mut inventory);
    // The counters are numbers and the lists lists, or the summary is no tree compile's.
    let count = |field: &str| match field {
        "files" | "compiled" | "compiled_before" => summary[field].as_u64(),
        _ => summary[field].as_array().map(|list| list.len() as u64),
    }.unwrap_or_else(|| panic!("{field} malformed: {summary}"));
    assert_eq!(count("files"), inventory.len() as u64, "the run did not reach every file of the tree: {summary}");
    assert_eq!(["compiled", "compiled_before", "source_only", "errors", "failed"].iter().map(|field| count(field)).sum::<u64>(),
               count("files"), "a file without an outcome: {summary}");
    assert!(count("compiled") > 1000, "the run did not compile the tree: {summary}");
    // The files whose compile refused them or signalled: a session reads them as source.
    let source_only: HashSet<String> = ["source_only", "errors"].iter()
        .flat_map(|field| summary[field].as_array().cloned().unwrap_or_default())
        .filter_map(|entry| entry["file"].as_str().map(str::to_owned)).collect();
    // A session of the same user loads what the tree compiled, the stock
    // theme and dired-x: an artifact that holds is taken, and no load
    // compiles (its compile trace, and `romacs-compiled-lisp-stats' written
    // to a file in its HOME). A load that reads source does so for a file
    // the tree's compile found refused (`not-compiled'). -Q: the system's
    // site-lisp, which no installation holds, stays out. With no compile
    // identity, every load compiled again.
    let form = "(progn (load (expand-file-name \"themes/romacs-theme-rum\" data-directory) nil t)
      (require 'dired-x)
      (let ((s (romacs-compiled-lisp-stats)))
        (with-temp-file (expand-file-name \"stats\" (getenv \"HOME\"))
          (prin1 (list (featurep 'dired-x) (plist-get s :taken) (plist-get s :compiled)
                       (plist-get s :source-on-miss) (plist-get s :misses))
                 (current-buffer)))))";
    // The store's use log names each load's source on an artifact hit: the
    // stock theme and dired-x must each have one from the session.
    let store = bin.join("romacs-vm-gui.lisp");
    let uses = |name: &str| -> usize {
        std::fs::read_dir(&store).into_iter().flatten().flatten()
            .filter_map(|generation| std::fs::read_to_string(generation.path().join("use.log")).ok())
            .map(|log| log.lines().filter(|line| line.ends_with(name)).count()).sum()
    };
    let (theme, dired) = ("/etc/themes/romacs-theme-rum.el", "/lisp/dired-x.el");
    let before = (uses(theme), uses(dired));
    let mut session = Command::new(&exe);
    user(session.args(["-Q", "--batch", "--eval", form]));
    let output = session.env("ROMACS_COMPILED_LISP", "on").env("ROMACS_COMPILED_LISP_STORE", bin.join("romacs-vm-gui.lisp"))
        .env("ROMACS_COMPILED_LISP_TRACE", "1").output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "the session: {}\n{stderr}", output.status);
    let compiles: Vec<&str> = stderr.lines().filter(|line| line.starts_with("romacs-compiled-lisp: {\"compile\"")).collect();
    assert!(compiles.is_empty(), "the session compiled what the tree did:\n{}", compiles.join("\n"));
    let written = std::fs::read_to_string(home.join("stats")).unwrap_or_else(|e| panic!("the session wrote no stats ({e}): {stderr}"));
    let inner = written.trim().strip_prefix('(').and_then(|text| text.strip_suffix(')')).unwrap_or_else(|| panic!("{written}"));
    let mut fields = inner.splitn(5, ' ');
    let loaded = fields.next() == Some("t");
    let mut number = || fields.next().and_then(|n| n.parse::<u64>().ok());
    let (taken, compiled, on_miss) = (number(), number(), number());
    let misses = fields.next().unwrap_or("nil");
    assert!(loaded, "the session did not load dired-x: {written}");
    assert_eq!(compiled, Some(0), "the session compiled what the tree did: {written}");
    assert!(taken.is_some_and(|n| n > 0), "the session took no artifact: {written}");
    assert!(uses(theme) > before.0, "the session's stock theme ran no artifact: {written}");
    assert!(uses(dired) > before.1, "the session's dired-x ran no artifact: {written}");
    // Each miss, (FILE . REASON), is a file the tree's compile refused; all
    // are listed (at most 32 are kept).
    let pairs: Vec<(&str, &str)> = misses.split("(\"").skip(1)
        .filter_map(|pair| pair.split_once("\" . ")).map(|(file, rest)| (file, rest.trim_end_matches([')', ' ']))).collect();
    assert_eq!(on_miss, Some(pairs.len() as u64), "misses not all listed: {written}");
    for (file, reason) in &pairs {
        assert!(*reason == "not-compiled" && source_only.contains(*file), "a load read {file} as source ({reason}): {written}");
    }
    let _ = std::fs::remove_dir_all(&root);
}
