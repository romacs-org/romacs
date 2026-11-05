//! fixtures/definition-files-test.el in Romacs's `--batch` session and in
//! pinned GNU's: both run every test and pass it.  Then Romacs's own
//! answers: a primitive's and a forwarded variable's Rust file, visited in
//! the tree `ROMACS_SOURCE_DIRECTORY' names, and none when it names none.
use std::process::Command;

fn batch(binary: &str, dir: &std::path::Path, args: &[&str], source: Option<&str>) -> (Option<i32>, String) {
    let mut command = Command::new(binary);
    command.args(["-Q", "--batch"]).args(args)
        .current_dir(dir).env("HOME", dir).env("TMPDIR", dir).env("XDG_CACHE_HOME", dir.join("cache")).env("ROMACS_WORLD", "off");
    if let Some(source) = source { command.env("ROMACS_SOURCE_DIRECTORY", source); }
    let output = command.output().unwrap();
    (output.status.code(), String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr))
}

fn ert(binary: &str, dir: &std::path::Path) -> (Option<i32>, String) {
    let (code, text) = batch(binary, dir, &["-l", concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/definition-files-test.el"),
                                          "-f", "ert-run-tests-batch-and-exit"], None);
    let summary = text.lines().find(|line| line.starts_with("Ran ")).unwrap_or("no ERT summary").to_owned();
    (code, summary.split(" (").next().unwrap().to_owned())
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("romacs-definition-files-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn definition_files_tests_pass_in_romacs_and_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let dir = scratch("ert");
    let expected = ert(&gnu, &dir);
    let actual = ert(env!("CARGO_BIN_EXE_romacs-vm-gui"), &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(expected, (Some(0), "Ran 3 tests, 3 results as expected, 0 unexpected".to_owned()));
    assert_eq!(actual, expected);
}

#[test]
fn rust_definitions_name_their_files_and_a_missing_tree_says_so() {
    let dir = scratch("rust");
    let form = "(progn (require 'help-fns) (require 'find-func) (prin1 (list
        (find-lisp-object-file-name 'read-string (symbol-function 'read-string))
        (find-lisp-object-file-name 'car (symbol-function 'car))
        (find-lisp-object-file-name 'fill-column 'defvar)
        (and (stringp source-directory) (file-directory-p (expand-file-name \"crates/romacs-vm/src/prims\" source-directory)))
        (let ((l (find-function-noselect 'read-string))) (with-current-buffer (car l) (line-number-at-pos (cdr l)))))))";
    let binary = env!("CARGO_BIN_EXE_romacs-vm-gui");
    // The tree named explicitly: a gate's mirror of it has no `.git', so
    // the build root alone names none there.
    let tree = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).canonicalize().unwrap();
    let (_, text) = batch(binary, &dir, &["--eval", form], Some(&tree.display().to_string()));
    let line = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../romacs-vm/src/prims/minibuf.rs")).unwrap()
        .lines().position(|l| l.contains("name = \"read-string\"")).unwrap() + 1;
    assert!(text.ends_with(&format!("(\"crates/romacs-vm/src/prims/minibuf.rs\" \"crates/romacs-vm/src/lib.rs\" \"crates/romacs-vm/src/forwarded.rs\" t {line})")), "{text}");
    // Advised, aliased, and each way Rust declares a variable: a string
    // spread over lines, a loop over names, and not a test's.
    let forms = "(progn (require 'find-func) (require 'xref) (require 'elisp-mode)
        (advice-add 'read-string :filter-return #'identity) (defalias 'definition-files-car 'car)
        (prin1 (list (mapcar (lambda (x) (xref-location-group (xref-item-location x))) (elisp--xref-find-definitions 'read-string))
                     (mapcar (lambda (x) (xref-location-group (xref-item-location x))) (elisp--xref-find-definitions 'definition-files-car))
                     (mapcar (lambda (v) (car (romacs--rust-definition v 'defvar))) '(default-text-properties most-positive-fixnum this-command global-map before-change-functions)))))";
    let (_, text) = batch(binary, &dir, &["--eval", forms], Some(&tree.display().to_string()));
    assert!(text.ends_with("((\"crates/romacs-vm/src/prims/minibuf.rs\") (\"crates/romacs-vm/src/lib.rs\") (\"crates/romacs-vm/src/lib.rs\" \"crates/romacs-vm/src/lib.rs\" \"crates/romacs-vm/src/command.rs\" \"crates/romacs-vm/src/keymap.rs\" \"crates/romacs-vm/src/forwarded.rs\"))"), "{text}");
    // A relative tree, named from the tree: absolute, so navigating from a
    // buffer visiting a file elsewhere still finds it.
    let visit = dir.join("elsewhere.el");
    std::fs::write(&visit, ";; elsewhere\n").unwrap();
    let relative = format!("(progn (require 'find-func) (find-file {:?}) (prin1 (list (file-name-absolute-p source-directory)
        (let ((l (find-function-noselect 'read-string))) (with-current-buffer (car l) (file-name-nondirectory buffer-file-name))))))", visit.display().to_string());
    let output = Command::new(binary).args(["-Q", "--batch", "--eval", &relative]).current_dir(&tree)
        .env("HOME", &dir).env("TMPDIR", &dir).env("XDG_CACHE_HOME", dir.join("cache")).env("ROMACS_WORLD", "off")
        .env("ROMACS_SOURCE_DIRECTORY", ".").output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr);
    assert!(text.ends_with("(t \"minibuf.rs\")"), "{text}");
    // No tree: a clear error, and GNU's nil from version.el's repository
    // queries rather than a signal.
    let missing = "(progn (require 'find-func) (prin1 (list source-directory (condition-case e (find-function-noselect 'read-string) (user-error (cadr e))) (emacs-repository-get-version) (emacs-repository-get-branch))))";
    let (_, text) = batch(binary, &dir, &["--eval", missing], Some(&dir.join("none").display().to_string()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(text.ends_with("(nil \"The Rust source file crates/romacs-vm/src/prims/minibuf.rs is not available: no Romacs source tree (`source-directory' is nil)\" nil nil)"), "{text}");
}
