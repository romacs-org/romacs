//! Strict source-only thunk bootstrap: every dependency must finish in a fresh VM.
//! GNU's Lisp owns the macros; native Custom/faces are deliberately not replaced.
use romacs_vm::Vm;
use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

const MIXED_DISPATCH: &str = "(defvar dispatch-binding nil)
(defun dispatch-step (n) (1+ n))
(let ((n 0))
  (while (< n 200000)
    (let ((dispatch-binding n))
      (setq n (dispatch-step dispatch-binding))))
  n)";

#[test]
fn mixed_cold_and_speculative_dispatch_does_not_retain_native_frames() {
    // Exercise the shipped CLI, not only the differently linked test library.
    let mut child = Command::new(env!("CARGO_BIN_EXE_romacs-vm"))
        .arg("--source")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(MIXED_DISPATCH.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "MIXED_DISPATCH_MUST_FINISH: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "200000");
}

#[test]
fn real_thunk_bootstrap_completes_without_custom_or_failed_prefix_reuse() {
    // Pin the ordinary CLI-sized stack rather than hiding dispatch retention
    // behind a larger test-thread stack. Stack/depth limits remain unchanged.
    std::thread::Builder::new().stack_size(8 * 1024 * 1024).spawn(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lisp/gnu");
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_load_path(&[root.clone(), root.join("emacs-lisp")]).unwrap();
        for name in [
            "emacs-lisp/debug-early", "emacs-lisp/byte-run", "emacs-lisp/backquote",
            "subr", "keymap", "emacs-lisp/pcase", "emacs-lisp/gv", "emacs-lisp/inline",
            "bindings", "emacs-lisp/easy-mmode", "emacs-lisp/thunk",
        ] {
            let result = vm.load_file(&root.join(format!("{name}.el"))).map(|_| ());
            assert!(result.is_ok(), "STRICT_THUNK_OWNER_MUST_COMPLETE {name}: {result:?}");
        }
        assert_eq!(vm.eval_source("(list (featurep 'cl-lib) (featurep 'thunk) (featurep 'custom) (car (symbol-function 'thunk-delay)))").unwrap().display().unwrap(), "(t t nil macro)");
        let report = vm.load_report();
        let steps = report["steps"].as_array().unwrap();
        assert!(steps.iter().all(|step| step["status"] == "ok"), "NO_FAILED_PREFIX");
        for (file, forms) in [("/cl-lib.el", 97), ("/thunk.el", 7)] {
            assert_eq!(steps.iter().filter(|step| step["file"].as_str().unwrap().ends_with(file)).count(), forms, "COMPLETE_GNU_OWNER {file}");
        }
    }).unwrap().join().unwrap();
}
