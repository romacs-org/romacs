//! The bundled packages' generated autoloads (LISP-BUNDLED.json) in the GNU
//! world the package ERT runner boots: each command is an autoload until it
//! is called, and calling it loads the package's own file from its directory.
use romacs_vm::Vm;
use std::path::Path;
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
    let layout: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join("LISP-BUNDLED.json")).unwrap()).unwrap();
    let autoloads = root.join(layout["autoloads"].as_str().unwrap());
    let text = std::fs::read_to_string(&autoloads).unwrap();
    // The package directories the artifact says it scanned, in its order.
    let directories: Vec<String> = text.lines().filter_map(|l| l.strip_prefix(";;; Directory "))
        .map(|d| format!("{:?}", root.join(d).to_string_lossy())).collect();
    assert!(directories.len() > 3, "{directories:?}");
    let home = std::env::temp_dir().join(format!("romacs-bundled-autoloads-{}/", std::process::id()));
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    vm.eval_source(&format!("(setq load-path (append load-path (list {})) user-emacs-directory {:?})",
        directories.join(" "), home.to_string_lossy())).unwrap();
    vm.load_file(&autoloads).unwrap();
    let state = |vm: &mut Vm, name: &str, file: &str| query(vm, &format!(
        "(list (if (autoloadp (symbol-function '{name})) (nth 1 (symbol-function '{name})) (functionp '{name}))
               (commandp '{name}) (featurep '{name}) (and (assoc {:?} load-history) t))",
        root.join(file).to_string_lossy()));
    let commands = [("gnosis-dashboard", "rlisp/gnosis/gnosis-dashboard.el"), ("hermes", "rlisp/hermes/hermes.el")];
    let before: Vec<String> = commands.iter().map(|(name, file)| state(&mut vm, name, file)).collect();
    assert_eq!(before, [r#"("gnosis-dashboard" t nil nil)"#, r#"("hermes" t nil nil)"#], "autoloaded, not loaded");
    // A call first does what `autoload-do-load` does; this loads the file
    // without running the command (no network or database).
    for (name, _) in commands {
        assert_eq!(query(&mut vm, &format!("(functionp (autoload-do-load (symbol-function '{name}) '{name}))")), "t", "{name}");
    }
    let after: Vec<String> = commands.iter().map(|(name, file)| state(&mut vm, name, file)).collect();
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(after, ["(t t t t)", "(t t t t)"], "each call loads its package's own file");
}
