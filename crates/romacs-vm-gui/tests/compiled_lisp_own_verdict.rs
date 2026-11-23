//! `--compile-lisp FILE` gives FILE's own verdict, even for a file the
//! process's boot loads (a preloaded one) and in an empty store: the boot's
//! load of FILE reads source instead of compiling FILE in a compile process
//! of its own (FILE is on its compile's chain), so the store holds no
//! artifact of FILE the explicit compile then finds held, its diagnostic
//! left in the store's compile log. Run as Forge's compile stage runs it:
//! a private HOME, an empty store, no `--world-store`.
use std::path::{Path, PathBuf};
use std::process::Command;

/// A preloaded file: the bootstrap loads it after `simple'.
const PRELOADED: &str = "lisp/completion/romacs-commands.el";

/// `--compile-lisp TREE/PRELOADED` in the fresh HOME ROOT/NAME, over an
/// empty store, compile traces on: (exit success, standard output,
/// standard error).
fn compile(root: &Path, name: &str, tree: &Path) -> (bool, String, String) {
    let home = root.join(name);
    std::fs::create_dir_all(&home).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_romacs-vm-gui"))
        .arg("--compile-lisp").arg(tree.join(PRELOADED))
        .current_dir(&home)
        .env_clear().env("PATH", "/usr/bin:/bin").env("LANG", "C.UTF-8").env("HOME", &home).env("TMPDIR", &home)
        .env("ROMACS_LISP_DIR", tree.join("lisp")).env("ROMACS_RUNTIME_ROOT", tree)
        .env("ROMACS_COMPILED_LISP", "on").env("ROMACS_COMPILED_LISP_STORE", home.join("store"))
        .env("ROMACS_COMPILED_LISP_TRACE", "1")
        .envs(["XDG_CACHE_HOME", "XDG_STATE_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR"]
              .map(|var| (var, home.join(var.to_lowercase()))))
        .envs(std::env::var_os("LD_LIBRARY_PATH").map(|v| ("LD_LIBRARY_PATH", v)))
        .stdin(std::process::Stdio::null())
        .output().unwrap();
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    (output.status.success(), text(&output.stdout), text(&output.stderr))
}

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

/// Whether the process traced a compile of FILE it started itself.
fn compiled_nested(stderr: &str, file: &Path) -> bool {
    stderr.lines().any(|line| line.starts_with("romacs-compiled-lisp: {\"compile\"") && line.contains(&*file.to_string_lossy()))
}

#[test]
fn a_preloaded_file_with_a_planted_error_gets_its_own_verdict_in_an_empty_store() {
    let root = std::env::temp_dir().join(format!("romacs-compile-own-verdict-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let tree = tree_copy(&root);
    let file = tree.join(PRELOADED);
    let mut source = std::fs::read_to_string(&file).unwrap();
    source.push_str("\n(defun planted-broken (\n");
    std::fs::write(&file, source).unwrap();

    let (_, stdout, stderr) = compile(&root, "planted", &tree);
    let said = format!("{stdout}{stderr}");
    assert!(!compiled_nested(&stderr, &file), "the boot compiled FILE in a process of its own:\n{said}");
    let diagnostic = format!("romacs-compile: {}: ", file.display());
    assert!(stderr.lines().any(|line| line.starts_with(&diagnostic) && line.contains("end-of-file")),
            "a planted error passed silently:\n{said}");
    assert!(stdout.lines().last().is_some_and(|line| line.starts_with("romacs-compile: compiled ")),
            "FILE's own compile, not a held artifact of another process's:\n{said}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_clean_preloaded_file_compiles_in_an_empty_store() {
    let root = std::env::temp_dir().join(format!("romacs-compile-own-clean-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let (ok, stdout, stderr) = compile(&root, "clean", &tree);
    let said = format!("{stdout}{stderr}");
    assert!(ok, "{said}");
    assert!(!compiled_nested(&stderr, &tree.join(PRELOADED)), "the boot compiled FILE in a process of its own:\n{said}");
    assert!(stdout.lines().last().is_some_and(|line| line.starts_with("romacs-compile: compiled ")), "{said}");
    let _ = std::fs::remove_dir_all(&root);
}
