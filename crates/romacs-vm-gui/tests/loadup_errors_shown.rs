//! A preloaded file that fails to load is said, never swallowed: the chain
//! goes on past it (`startup::drive_loadup`), and notes its error as it
//! notes one of Romacs's Lisp, at the place the drive knows as data. A
//! batch session booted from source says it on stderr, a batch world's
//! build records it (`lisp_errors`), and a session or isolate worker
//! restored from a world says it too.
use std::path::{Path, PathBuf};
use std::process::Command;

/// A file the bootstrap loads through a load site of its own.
const SITE: &str = "lisp/frame.el";
/// A file the bootstrap loads from inside one of its forms, after `simple'.
const FOLLOWED: &str = "lisp/completion/romacs-commands.el";
/// The bootstrap the chain drives.
const BOOTSTRAP: &str = "lisp/session/romacs-loadup-bootstrap.el";

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

/// TEXT appended to FILE.
fn plant(file: &Path, text: &str) {
    let mut source = std::fs::read_to_string(file).unwrap();
    source.push_str(text);
    std::fs::write(file, source).unwrap();
}

/// The romacs-vm-gui ARGS over TREE in the HOME ROOT/home, worlds and
/// compiled Lisp at their defaults (none made yet): (exit success, stdout,
/// stderr).
fn run(root: &Path, tree: &Path, args: &[&str]) -> (bool, String, String) {
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
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    (output.status.success(), text(&output.stdout), text(&output.stderr))
}

/// The errors STDERR says were noted (`romacs: ...`).
fn noted(stderr: &str) -> Vec<&str> {
    stderr.lines().filter_map(|line| line.strip_prefix("romacs: ")).filter(|line| line.contains(": line ")).collect()
}

/// The `romacs-world` event of STDERR that says a world was prepared, for
/// which the predicate holds.
fn prepared(stderr: &str, which: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
    stderr.lines().filter_map(|line| line.strip_prefix("romacs-world: "))
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| event["mode"] == "prepared" && which(event))
        .unwrap_or_else(|| panic!("no prepared world:\n{stderr}"))
}

/// A load site's file that the reader cannot finish (an unfinished
/// `defun` after comments) is said at that file and the line the form
/// begins on, by the drive's own place for it, from source, in the batch
/// world's manifest, and from a session restored from it: the reader's
/// text never names the place.
#[test]
fn an_unfinished_defun_in_a_site_file_is_said_at_that_file() {
    let root = std::env::temp_dir().join(format!("romacs-loadup-errors-site-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let tree = tree_copy(&root);
    let file = tree.join(SITE);
    let before = format!("{}\n;; a comment\n\n;; another\n", std::fs::read_to_string(&file).unwrap());
    let line = before.matches('\n').count() + 1;
    std::fs::write(&file, format!("{before}(defun planted-broken (\n")).unwrap();
    let head = format!("{}: line {line} form ", file.display());

    let (ok, _, stderr) = run(&root, &tree, &["--batch", "--eval", "(princ 1)"]);
    assert!(ok, "{stderr}");
    let said = noted(&stderr);
    assert_eq!(said.len(), 1, "{stderr}");
    assert!(said[0].starts_with(&head) && said[0].contains("end-of-file"), "{stderr}");

    let store = root.join("worlds");
    let store = store.to_str().unwrap();
    let (ok, _, stderr) = run(&root, &tree, &["--world-store", store, "--world-build", "--batch"]);
    assert!(ok, "{stderr}");
    let event = prepared(&stderr, |event| event["launch"] == "batch");
    let errors = event["lisp_errors"].as_array().unwrap_or_else(|| panic!("{event}"));
    assert_eq!(errors.len(), 1, "{event}");
    assert_eq!(errors[0]["file"], file.to_string_lossy().as_ref(), "{event}");
    assert_eq!(errors[0]["line"], line, "{event}");

    let (ok, _, stderr) = run(&root, &tree, &["--world-store", store, "--batch", "--eval", "(princ 1)"]);
    assert!(ok, "{stderr}");
    assert!(stderr.contains(r#""mode":"restored""#), "not restored:\n{stderr}");
    assert_eq!(noted(&stderr), said, "the restored session says what the source boot said");
    let _ = std::fs::remove_dir_all(&root);
}

/// A site whose file loaded whole but whose after-load hook failed has no
/// form of its own that failed: it is said at the site's call in the
/// bootstrap.
#[test]
fn a_failed_after_load_hook_is_said_at_the_sites_call() {
    let root = std::env::temp_dir().join(format!("romacs-loadup-errors-hook-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let tree = tree_copy(&root);
    plant(&tree.join(SITE), "\n(eval-after-load \"frame\" '(error \"planted after-load\"))\n");
    let bootstrap = tree.join(BOOTSTRAP);
    let text = std::fs::read_to_string(&bootstrap).unwrap();
    let call = text.lines().position(|l| l.starts_with("(load \"frame.el\"")).expect("the bootstrap's frame.el site") + 1;
    let (ok, _, stderr) = run(&root, &tree, &["--batch", "--eval", "(princ 1)"]);
    assert!(ok, "{stderr}");
    let said = noted(&stderr);
    assert_eq!(said.len(), 1, "{stderr}");
    assert!(said[0].starts_with(&format!("{}: line {call} form ", bootstrap.display())), "{stderr}");
    assert!(said[0].contains("planted after-load"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// A form that caught a failed load and then signalled something else is
/// said with the error that escaped it, at the bootstrap's form that ran
/// it, never with the one it caught.
#[test]
fn the_error_that_escaped_is_said_not_one_caught_before_it() {
    let root = std::env::temp_dir().join(format!("romacs-loadup-errors-caught-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let tree = tree_copy(&root);
    let dependency = tree.join("lisp/completion/romacs-planted-dependency.el");
    std::fs::write(&dependency, "(error \"the caught failure\")\n").unwrap();
    plant(&tree.join(FOLLOWED), &format!("\n(condition-case nil (load {:?} nil t) (error nil))\n\
                                         (error \"the escaped failure\")\n", dependency.to_str().unwrap()));
    let (ok, _, stderr) = run(&root, &tree, &["--batch", "--eval", "(princ 1)"]);
    assert!(ok, "{stderr}");
    let said = noted(&stderr);
    assert_eq!(said.len(), 1, "{stderr}");
    assert!(said[0].starts_with(&format!("{}: line ", tree.join(BOOTSTRAP).display())), "{stderr}");
    assert!(said[0].contains("the escaped failure") && !said[0].contains("the caught failure"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}

/// An isolate worker restored from a world whose build noted an error says
/// it, as a worker booted from source says the errors its boot meets.
#[test]
fn an_isolate_worker_restored_from_a_world_says_its_noted_errors() {
    let root = std::env::temp_dir().join(format!("romacs-loadup-errors-isolate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let tree = tree_copy(&root);
    // Romacs's Lisp of the GUI's world, loaded after the chain.
    let file = tree.join("lisp/ui/romacs-popup.el");
    plant(&file, "\n(error \"planted on purpose\")\n");
    let store = root.join("worlds");
    let store = store.to_str().unwrap();
    let (ok, _, stderr) = run(&root, &tree, &["--loadup", "--world-store", store, "--world-build"]);
    assert!(ok, "{stderr}");
    let event = prepared(&stderr, |event| event["recipe"][0] == "consumer romacs-vm-gui");
    assert_eq!(event["lisp_errors"].as_array().map(Vec::len), Some(1), "{event}");
    let key = event["key"].as_str().unwrap();
    let script = format!("(progn (setq isolate-boot 'world isolate-world (cons {store:?} {key:?}) isolate-pool-size 1)
                            (prin1 (isolate-await (isolate-spawn '+ 1 2) 60)) (isolate-kill-all))");
    let (ok, stdout, stderr) = run(&root, &tree, &["-Q", "--batch", "--eval", &script]);
    assert!(ok, "{stderr}");
    assert_eq!(stdout.trim(), "3", "{stderr}");
    let head = format!("{}: line ", file.display());
    let said: Vec<&str> = noted(&stderr).into_iter().filter(|line| line.starts_with(&head)).collect();
    assert_eq!(said.len(), 1, "the worker said its world's error:\n{stderr}");
    assert!(said[0].contains("planted on purpose"), "{stderr}");
    let _ = std::fs::remove_dir_all(&root);
}
