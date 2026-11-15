//! The bundled packages' autoloads (lisp/session/romacs-bundled-autoloads.el,
//! which scans Romacs's own files and the bundled packages as it loads) in the
//! GNU world the package ERT runner boots: each command is an autoload until
//! it is called, and calling it loads the package's own file from its
//! directory.
use romacs_vm::Vm;
use romacs_vm::host::{self, Pinned};
use serde_json::json;
use std::path::Path;
use std::time::UNIX_EPOCH;
#[path = "support/gnu_ert_vm.rs"]
mod gnu_ert_vm;

fn query(vm: &mut Vm, form: &str) -> String {
    let source = format!("(encode-coding-string (condition-case e (prin1-to-string {form}) (error (format \"signal %S\" e))) 'utf-8)");
    match vm.eval_source(&source).and_then(|v| v.bytes().map(|b| String::from_utf8_lossy(b).into_owned())) {
        Ok(text) => text,
        Err(error) => format!("VM error: {error}"),
    }
}

#[test]
fn bundled_commands_are_autoloads_that_load_their_package_when_called() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let layout: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join("lisp/bundled.json")).unwrap()).unwrap();
    let autoloads = root.join(layout["autoloads"].as_str().unwrap());
    let home = std::env::temp_dir().join(format!("romacs-bundled-autoloads-{}/", std::process::id()));
    let mut vm = gnu_ert_vm::bootstrapped();
    vm.load_file(&autoloads).unwrap();
    // The package directories it scanned, in their order, as --loadup's load-path has them.
    let root_name = format!("{:?}", root.to_string_lossy());
    assert!(query(&mut vm, &format!("(length (romacs-bundled-autoloads-directories {root_name}))")).parse::<usize>().unwrap() > 3);
    vm.eval_source(&format!("(setq load-path (append load-path (romacs-bundled-autoloads-directories {root_name})) user-emacs-directory {:?})",
        home.to_string_lossy())).unwrap();
    let state = |vm: &mut Vm, name: &str, feature: &str, file: &str| query(vm, &format!(
        "(list (if (autoloadp (symbol-function '{name})) (nth 1 (symbol-function '{name})) (functionp '{name}))
               (commandp '{name}) (featurep '{feature}) (and (assoc {:?} load-history) t))",
        root.join(file).to_string_lossy()));
    let commands = [("gnosis-dashboard", "gnosis-dashboard", "lisp/packages/gnosis/gnosis-dashboard.el"),
                    ("jabber-customize", "jabber", "lisp/packages/jabber/jabber.el")];
    let before: Vec<String> = commands.iter().map(|(name, feature, file)| state(&mut vm, name, feature, file)).collect();
    assert_eq!(before, [r#"("gnosis-dashboard" t nil nil)"#, r#"("jabber" t nil nil)"#], "autoloaded, not loaded");
    // A call first does what `autoload-do-load` does; this loads the file
    // without running the command (no network or database).
    for (name, _, _) in commands {
        assert_eq!(query(&mut vm, &format!("(functionp (autoload-do-load (symbol-function '{name}) '{name}))")), "t", "{name}");
    }
    let after: Vec<String> = commands.iter().map(|(name, feature, file)| state(&mut vm, name, feature, file)).collect();
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(after, ["(t t t t)", "(t t t t)"], "each call loads its package's own file");
}

/// The --loadup world loads the library (romacs-vm-gui's startup), so what
/// its scan reads keys the world: each bundled directory's listing and each
/// package file is a record of the build, and the scan leaves nothing uncaptured.
#[test]
fn a_world_build_records_what_the_bundled_scan_reads() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mut vm = gnu_ert_vm::bootstrapped();
    host::enter_build(Pinned { clock: UNIX_EPOCH, pid: 4242, seed: [1, 2, 3, 4] });
    let loaded = vm.load_file(&root.join("lisp/session/romacs-bundled-autoloads.el"));
    let trace = host::leave_build().expect("a build was recording");
    loaded.unwrap();
    let records = trace["records"].as_array().unwrap();
    let recorded = |kind: &str, path: &Path| records.iter().any(|r| r["kind"] == kind && r["path"] == json!(path.to_string_lossy()));
    for dir in ["lisp", "lisp/emacs-lisp", "lisp/agents", "lisp/packages/gnosis", "lisp/packages/jabber"] {
        assert!(recorded("dir-list", &root.join(dir)), "{dir} is listed");
    }
    for file in ["lisp/bundled.json", "lisp/packages/jabber/jabber.el", "lisp/packages/gnosis/gnosis-dashboard.el",
                 "lisp/agents/romacs-attach.el"] {
        assert!(recorded("file-read", &root.join(file)), "{file} is read");
    }
    assert_eq!(trace["uncaptured"], json!([]));
}

/// The scan defines what `loaddefs-generate' writes for the same directory,
/// in its order: files by library name (foo before foo-bar, which a listing
/// puts first), a cookie's text read as written, and nothing of a file that
/// sends its autoloads to another file (`generated-autoload-file').
#[test]
fn the_scan_yields_the_forms_loaddefs_generate_writes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let dir = std::env::temp_dir().join(format!("romacs-bundled-scan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, text) in [
        ("foo.el", ";;; foo.el --- fixture  -*- lexical-binding: t -*-\n;;;###autoload\n(defun foo-command () \"Foo.\" (interactive) 1)\n\
                    ;;;###autoload\n(defcustom foo-option nil \"Foo option.\" :type 'boolean :group 'foo)\n\
                    ;;;###autoload (put 'foo-command 'foo-fixture t)\n(provide 'foo)\n"),
        ("foo-bar.el", ";;; foo-bar.el --- fixture  -*- lexical-binding: t -*-\n;;;###autoload\n(define-minor-mode foo-bar-mode \"Foo bar.\" :global t)\n(provide 'foo-bar)\n"),
        ("other.el", ";;; other.el --- fixture  -*- lexical-binding: t -*-\n;;;###autoload\n(defun other-command () \"Other.\" (interactive))\n(provide 'other)\n\
                      ;; Local Variables:\n;; generated-autoload-file: \"other-loaddefs.el\"\n;; End:\n"),
    ] {
        std::fs::write(dir.join(name), text).unwrap();
    }
    let mut vm = gnu_ert_vm::bootstrapped();
    vm.load_file(&root.join("lisp/session/romacs-bundled-autoloads.el")).unwrap();
    let dir_name = format!("{:?}", dir.to_string_lossy());
    let found = query(&mut vm, &format!("(let* ((dir {dir_name}) (output (expand-file-name \"=loaddefs.el\" dir)))
        (loaddefs-generate dir output nil nil nil t)
        (let ((written (with-temp-buffer
                         (insert-file-contents output)
                         (let (forms) (condition-case nil (while t (push (read (current-buffer)) forms)) (end-of-file nil))
                              (seq-remove (lambda (f) (eq (car-safe f) 'provide)) (nreverse forms)))))
              (scanned (romacs-bundled-autoloads--forms dir (romacs-bundled-autoloads--files dir))))
          (list (equal scanned written) (length scanned)
                (and (memq 'foo-fixture (flatten-tree scanned)) t) (and (memq 'other-command (flatten-tree scanned)) t)
                (seq-position (flatten-tree scanned) 'foo-command) (seq-position (flatten-tree scanned) 'foo-bar-mode))))"));
    let _ = std::fs::remove_dir_all(&dir);
    let parts: Vec<&str> = found.trim_matches(['(', ')']).split(' ').collect();
    assert_eq!(parts[..4], ["t", parts[1], "t", "nil"], "equal, count, text cookie, no redirected form: {found}");
    assert!(parts[4].parse::<usize>().unwrap() < parts[5].parse::<usize>().unwrap(), "foo before foo-bar: {found}");
}

/// The scan and `tools/lisp-loaddefs` split the tree between them by one rule: the files the
/// library scans are the ones `tools/lib/lisp_layout.py` `scanned_files` leaves out of the loaddefs
/// files (the library itself aside, which scans everything but itself), so no file's autoloads are
/// in both holders or in neither.
#[test]
fn the_scan_holds_what_lisp_layout_says_it_does() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mut vm = gnu_ert_vm::bootstrapped();
    vm.load_file(&root.join("lisp/session/romacs-bundled-autoloads.el")).unwrap();
    let listed = query(&mut vm, &format!("(mapconcat #'identity (sort (apply #'append (mapcar #'cdr (romacs-bundled-autoloads--scan {:?}))) #'string<) \" \")",
        root.to_string_lossy()));
    let scanned: Vec<String> = serde_json::from_str::<String>(&listed).unwrap().split(' ').map(str::to_owned).collect();
    let python = std::process::Command::new("python3").arg("-B").arg("-c")
        .arg("import sys; sys.path.insert(0, sys.argv[1] + '/tools/lib'); import lisp_layout; from pathlib import Path\n\
              print('\\n'.join(sorted(str(p) for p in lisp_layout.scanned_files(Path(sys.argv[1])))))")
        .arg(&root).output().unwrap();
    assert!(python.status.success(), "{}", String::from_utf8_lossy(&python.stderr));
    let own = root.join("lisp/session/romacs-bundled-autoloads.el").to_string_lossy().into_owned();
    let mut expected: Vec<String> = String::from_utf8(python.stdout).unwrap().lines()
        .filter(|line| *line != own).map(str::to_owned).collect();
    expected.sort();
    let mut scanned = scanned;
    scanned.sort();
    assert_eq!(scanned, expected);
}
