//! A preloaded file that fails to load is said, never swallowed: the chain
//! goes on past it (`startup::drive_loadup`), and notes its error as it
//! notes one of Romacs's Lisp. A batch session booted from source says it
//! on stderr, a batch world's build records it (`lisp_errors`), and a
//! session restored from that world says it too.
use std::path::{Path, PathBuf};
use std::process::Command;

/// A preloaded file: the bootstrap loads it after `simple'.
const PRELOADED: &str = "lisp/completion/romacs-commands.el";

/// The Lisp and data of this checkout, copied (reflinked where the file
/// system can) into ROOT/tree, by its true name.
fn tree_copy(root: &Path) -> PathBuf {
    let tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let copy = root.join("tree");
    std::fs::create_dir_all(&copy).unwrap();
    let copied = Command::new("cp").args(["-a", "--reflink=auto"]).arg(tree.join("lisp")).arg(tree.join("etc")).arg(&copy)
        .status().unwrap();
    assert!(copied.success());
    copy.canonicalize().unwrap()
}

/// The romacs-vm-gui ARGS over TREE in the HOME ROOT/home, worlds and
/// compiled Lisp at their defaults (none made yet): (exit success, stderr).
fn run(root: &Path, tree: &Path, args: &[&str]) -> (bool, String) {
    let home = root.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .args(args)
        .current_dir(&home)
        .env_clear().env("PATH", "/usr/bin:/bin").env("LANG", "C.UTF-8").env("HOME", &home).env("TMPDIR", &home)
        .env("ROMACS_LISP_DIR", tree.join("lisp")).env("ROMACS_RUNTIME_ROOT", tree).env("ROMACS_DIAGNOSTICS", "1")
        .envs(["XDG_CACHE_HOME", "XDG_STATE_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR"]
              .map(|var| (var, home.join(var.to_lowercase()))))
        .envs(std::env::var_os("LD_LIBRARY_PATH").map(|v| ("LD_LIBRARY_PATH", v)))
        .stdin(std::process::Stdio::null())
        .output().unwrap();
    (output.status.success(), String::from_utf8_lossy(&output.stderr).into_owned())
}

/// The lines of STDERR that say FILE's planted error, as a noted error of
/// Romacs's Lisp is said.
fn said(stderr: &str, file: &Path) -> usize {
    let head = format!("romacs: {}: line ", file.display());
    stderr.lines().filter(|line| line.starts_with(&head) && line.contains("planted on purpose")).count()
}

#[test]
fn a_preloaded_file_that_fails_to_load_is_said_from_source_and_from_its_world() {
    let root = std::env::temp_dir().join(format!("romacs-loadup-errors-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let tree = tree_copy(&root);
    let file = tree.join(PRELOADED);
    let mut source = std::fs::read_to_string(&file).unwrap();
    source.push_str("\n(error \"planted on purpose\")\n");
    std::fs::write(&file, source).unwrap();
    let session = ["--batch", "--eval", "(princ 1)"];

    // No world yet: the session boots from source.
    let (ok, stderr) = run(&root, &tree, &session);
    assert!(ok, "{stderr}");
    assert_eq!(said(&stderr, &file), 1, "the source boot swallowed the error:\n{stderr}");

    // The batch world's build records it.
    let store = root.join("worlds");
    let store = store.to_str().unwrap();
    let (ok, stderr) = run(&root, &tree, &["--world-store", store, "--world-build", "--batch"]);
    assert!(ok, "{stderr}");
    let prepared = stderr.lines().filter_map(|line| line.strip_prefix("romacs-world: "))
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| event["mode"] == "prepared" && event["launch"] == "batch")
        .unwrap_or_else(|| panic!("no prepared batch world:\n{stderr}"));
    let errors = prepared["lisp_errors"].as_array().unwrap_or_else(|| panic!("{prepared}"));
    assert_eq!(errors.len(), 1, "{prepared}");
    assert_eq!(errors[0]["file"], file.to_string_lossy().as_ref(), "{prepared}");

    // A session restored from it says it.
    let (ok, stderr) = run(&root, &tree, &["--world-store", store, "--batch", "--eval", "(princ 1)"]);
    assert!(ok, "{stderr}");
    assert!(stderr.contains(r#""mode":"restored""#), "not restored:\n{stderr}");
    assert_eq!(said(&stderr, &file), 1, "the restored session swallowed the error:\n{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}
