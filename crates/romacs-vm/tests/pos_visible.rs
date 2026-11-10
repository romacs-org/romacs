//! pos-visible-in-window-p (window.c:1998). In batch the window is on the
//! initial frame, where pos_visible_p (xdisp.c:1703) sees nothing, so after
//! GNU's argument checks the answer is nil, whatever `noninteractive' holds:
//! compared against pinned GNU. Once a host has laid the frame out, a
//! displayed window refuses unless a host layout is attached to answer
//! (VM2-POSVIS1); romacs-vm-gui's measure.rs tests the hosted answers.
use romacs_vm::Vm;

/// One expression per line.
const BATCH: &str = r###"
(mapcar (lambda (f) (condition-case e (funcall f) (error e))) (list (lambda () (pos-visible-in-window-p)) (lambda () (pos-visible-in-window-p 1)) (lambda () (pos-visible-in-window-p t)) (lambda () (pos-visible-in-window-p 1 nil t)) (lambda () (pos-visible-in-window-p 1000)) (lambda () (pos-visible-in-window-p "x")) (lambda () (pos-visible-in-window-p 1 3)) (lambda () (pos-visible-in-window-p (make-marker))) (lambda () (pos-visible-in-window-p (point-marker) (selected-window))) (lambda () (pos-visible-in-window-p 1.5)) (lambda () (pos-visible-in-window-p 1 (minibuffer-window)))))
(list (pos-visible-in-window-p (expt 2 80)) (pos-visible-in-window-p (- (expt 2 80))) (let ((noninteractive nil)) (pos-visible-in-window-p 1)) (with-temp-buffer (insert "abc") (pos-visible-in-window-p)))
"###;

#[test]
fn batch_pos_visible_in_window_p_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    for expression in BATCH.lines().filter(|line| !line.is_empty()) {
        let source = format!("(prin1-to-string {expression})");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(princ {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let actual = vm.eval_source(&source).unwrap().display().unwrap();
        assert_eq!(actual, format!("{:?}", String::from_utf8(output.stdout).unwrap()), "{expression}");
    }
}

#[test]
fn displayed_window_refuses_after_a_host_layout() {
    let mut vm = Vm::new();
    let face = vm.display_default_face().unwrap();
    let (line, column) = romacs_layout::frame_metrics(&face).unwrap();
    vm.set_frame_geometry(column * 80, line * 24, line, column).unwrap();
    vm.eval_source(r#"(progn (set-buffer (get-buffer-create "visible")) (insert (make-string 100 ?x))
                             (set-window-buffer nil (current-buffer)) (goto-char (point-min)))"#).unwrap();
    // Before a host lays anything out the frame is like GNU's initial one.
    assert_eq!(vm.eval_source("(pos-visible-in-window-p 1)").unwrap().display().unwrap(), "nil");
    let snapshot = vm.frame_snapshot(column * 80, line * 24).unwrap();
    let (_, ends) = romacs_layout::layout_frame(&snapshot, 1).unwrap();
    vm.display_apply_layout(&ends).unwrap();
    let refused = vm.eval_source("(condition-case e (pos-visible-in-window-p 1) (error (car (cdr e))))").unwrap().display().unwrap();
    assert_eq!(refused, "\"VM2 pos-visible-in-window-p in a displayed window is unsupported\"");
    // GNU's range checks still answer nil first, and argument errors still signal.
    assert_eq!(vm.eval_source("(list (pos-visible-in-window-p 0) (pos-visible-in-window-p 1000))").unwrap().display().unwrap(), "(nil nil)");
    assert_eq!(vm.eval_source("(condition-case e (pos-visible-in-window-p \"x\") (error e))").unwrap().display().unwrap(), "(wrong-type-argument integer-or-marker-p \"x\")");
}
