//! window.c `recenter', `scroll-up', `scroll-down' and
//! `move-to-window-line' against pinned GNU in batch, where GNU takes their
//! line-based paths (Frecenter's terminal branch, window_scroll_line_based):
//! a 100-line buffer in the selected window of GNU's 80x25 batch frame.
//! romacs-vm-gui's measure.rs checks them against the host's paint.
use romacs_vm::Vm;

const SETUP: &str = r#"(progn (set-default 'mode-line-format "%b") (set-buffer (get-buffer-create "scroll"))
  (set-window-buffer nil (current-buffer)) (dotimes (i 100) (insert (format "line %d\n" i)))
  (goto-char 1) (set-window-start nil 1))"#;

const CASES: &[&str] = &[
    "(progn (goto-char (point-max)) (recenter -3) (list (window-start) (point)))",
    "(progn (goto-char 500) (list (progn (recenter 0) (window-start)) (progn (recenter) (window-start)) (progn (recenter '(4)) (window-start)) (progn (recenter 5) (window-start)) (progn (recenter -1) (window-start)) (progn (recenter 100) (window-start))))",
    "(list (progn (scroll-up) (list (window-start) (point))) (progn (scroll-up 5) (list (window-start) (point))) (progn (scroll-down 2) (list (window-start) (point))) (progn (scroll-down) (list (window-start) (point))))",
    "(list (condition-case e (scroll-down) (error e)) (window-start) (point))",
    "(progn (goto-char (point-max)) (set-window-start nil (- (point-max) 20)) (list (condition-case e (scroll-up) (error e)) (window-start) (point)))",
    "(progn (set-window-start nil 50) (list (move-to-window-line 0) (point) (move-to-window-line -1) (point) (move-to-window-line nil) (point) (move-to-window-line 3) (point) (move-to-window-line 200) (point)))",
    "(let ((scroll-preserve-screen-position t)) (goto-char 30) (list (progn (scroll-up 4) (list (window-start) (point))) (progn (scroll-down 4) (list (window-start) (point)))))",
    "(let ((scroll-margin 3)) (list (progn (scroll-up 10) (list (window-start) (point))) (progn (goto-char 600) (recenter 0) (window-start))))",
    "(with-temp-buffer (list (condition-case e (recenter) (error e)) (condition-case e (move-to-window-line 0) (error e))))",
    "(list (let ((w (selected-window))) (scroll-up 3) (window-start w)) (condition-case e (scroll-up \"x\") (error e)))",
    "(mapcar (lambda (f) (interactive-form f)) '(recenter scroll-up scroll-down move-to-window-line))",
];

#[test]
fn window_scrolling_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES {
        let source = format!("(progn {SETUP} (prin1-to-string (condition-case e {expression} (error (list 'signal e)))))");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(princ {source})")])
            .output()
            .unwrap();
        assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.set_frame_geometry(80, 25, 1, 1).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display()).map(|printed| {
            serde_json::from_str::<String>(&printed).unwrap_or(printed)
        }).unwrap_or_else(|e| format!("ERR {e}"));
        if actual != expected {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
