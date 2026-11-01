//! window.c's window change functions (rlisp/vm/window-change.el, run by
//! each redisplay's frame snapshot). Against pinned GNU, which runs no
//! redisplay in batch: `run-window-configuration-change-hook' runs each
//! window's buffer-local functions with that window selected and its
//! buffer current, then the default ones. Of the redisplay's own run: a
//! buffer shown again in a window runs the buffer-change, size-change,
//! state-change and configuration-change functions, buffer-locally with
//! the window and by default with the frame, then records the state the
//! next run and `window-old-buffer' compare with; an unchanged frame runs
//! none (the GUI parity harnesses compare the whole cycle with GNU's).
use romacs_vm::Vm;
use std::path::Path;

fn bootstrapped() -> Vm {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    let lisp = root.join("gnu-lisp");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('='))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    paths.push(root.join("rlisp"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("rlisp/romacs-loadup-bootstrap.el")).unwrap();
    vm
}

const CONFIGURATION: &str = r#"(let ((log nil) (a (get-buffer-create "wc-a")) (b (get-buffer-create "wc-b")))
  (delete-other-windows)
  (switch-to-buffer a)
  (let ((other (split-window)))
    (set-window-buffer other b)
    (with-current-buffer b
      (add-hook 'window-configuration-change-hook
                (lambda () (push (list 'local (buffer-name) (buffer-name (window-buffer (selected-window)))) log)) nil t))
    (add-hook 'window-configuration-change-hook
              (lambda () (push (list 'global (buffer-name) (buffer-name (window-buffer (selected-window)))) log)))
    (run-window-configuration-change-hook)
    (prog1 (list (reverse log) (buffer-name) (buffer-name (window-buffer (selected-window))))
      (setq window-configuration-change-hook nil)
      (delete-window other))))"#;

#[test]
fn configuration_change_hook_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&binary)
        .args(["-Q", "--batch", "--eval", &format!("(prin1 {CONFIGURATION})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let mut vm = bootstrapped();
    vm.eval_source(include_str!("../../../rlisp/vm/window-change.el")).unwrap();
    assert_eq!(vm.eval_source(CONFIGURATION).unwrap().display().unwrap(), expected);
}

#[test]
fn redisplay_runs_window_change_functions() {
    let mut vm = bootstrapped();
    vm.eval_source(include_str!("../../../rlisp/vm/window-change.el")).unwrap();
    let source = r#"(let ((log nil) (a (get-buffer-create "wc-a")) (b (get-buffer-create "wc-b")))
  (delete-other-windows)
  (switch-to-buffer a)
  (romacs--run-window-change-functions)
  (add-hook 'window-buffer-change-functions (lambda (f) (push (list 'buffer (framep f)) log)))
  (add-hook 'window-size-change-functions (lambda (f) (push (list 'size (framep f)) log)))
  (add-hook 'window-state-change-functions (lambda (f) (push (list 'state (framep f)) log)))
  (add-hook 'window-configuration-change-hook (lambda () (push 'configuration log)))
  (with-current-buffer b
    (add-hook 'window-buffer-change-functions (lambda (w) (push (list 'local (windowp w) (buffer-name)) log)) nil t))
  (romacs--run-window-change-functions)
  (push 'quiet log)
  (switch-to-buffer b)
  (romacs--run-window-change-functions)
  (prog1 (list (reverse log) (buffer-name (window-old-buffer)) (eq (old-selected-window) (selected-window)))
    (setq window-buffer-change-functions nil window-size-change-functions nil
          window-state-change-functions nil window-configuration-change-hook nil)))"#;
    assert_eq!(vm.eval_source(source).unwrap().display().unwrap(),
               "((quiet (local t \"wc-b\") (buffer t) (size t) (state t) configuration) \"wc-b\" t)");
}

/// The redisplay's gate before the Lisp: a frame snapshot that follows a
/// change only body sizes see (the mode line hidden) runs the size-change
/// functions, a buffer switch across a collection runs the buffer-change
/// functions, and a snapshot of an unchanged frame runs none.
#[test]
fn frame_snapshots_run_window_change_functions_on_changes() {
    let mut vm = bootstrapped();
    vm.eval_source(include_str!("../../../rlisp/vm/window-change.el")).unwrap();
    vm.eval_source(r#"(progn (delete-other-windows) (switch-to-buffer (get-buffer-create "wc-a")) (setq wc-log nil))"#).unwrap();
    // The first snapshot lays the frame out at its size, a size change.
    vm.frame_snapshot(800, 600).unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    vm.eval_source(r#"(progn
  (add-hook 'window-size-change-functions (lambda (_) (push 'size wc-log)))
  (add-hook 'window-buffer-change-functions (lambda (_) (push 'buffer wc-log))))"#).unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    vm.eval_source(r#"(push 'unchanged wc-log)"#).unwrap();
    vm.eval_source(r#"(with-current-buffer "wc-a" (setq-local mode-line-format nil))"#).unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    vm.eval_source(r#"(push 'mode-line wc-log)"#).unwrap();
    vm.eval_source(r#"(progn (switch-to-buffer (get-buffer-create "wc-b")) (kill-buffer "wc-a") (garbage-collect) (switch-to-buffer (get-buffer-create "wc-c")) (kill-buffer "wc-b"))"#).unwrap();
    vm.frame_snapshot(800, 600).unwrap();
    assert_eq!(vm.eval_source("(prog1 (reverse wc-log) (setq window-size-change-functions nil window-buffer-change-functions nil))").unwrap().display().unwrap(),
               "(unchanged size mode-line buffer size)");
}
