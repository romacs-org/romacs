//! window_wants_header_line (window.c:6067) and the heights it feeds:
//! `window-header-line-height', `window-body-height' and
//! `window-text-pixel-size' MODE-LINES, against pinned GNU as relations to
//! the frame's line, since VM2's batch frame keeps its pixel backend
//! geometry (DIVERGENCES.md VM2-WIN5). romacs-vm-gui's measure.rs tests the
//! painted header line.
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(let ((header-line-format "h")) (list (= (window-header-line-height) (frame-char-height)) (- (window-total-height) (window-body-height) (/ (window-mode-line-height) (frame-char-height))) (- (window-pixel-height) (window-body-height nil t) (window-mode-line-height) (window-header-line-height)) (window-header-line-height (minibuffer-window))))
(list (window-header-line-height) (progn (set-window-parameter nil 'header-line-format "p") (prog1 (= (window-header-line-height) (frame-char-height)) (set-window-parameter nil 'header-line-format nil))))
(let ((header-line-format "h")) (set-window-parameter nil 'header-line-format 'none) (prog1 (window-header-line-height) (set-window-parameter nil 'header-line-format nil)))
(let ((b (get-buffer-create "hl-other")) (w (selected-window))) (with-current-buffer b (setq header-line-format "o")) (with-temp-buffer (list (window-header-line-height w) (progn (set-window-buffer w b) (= (window-header-line-height w) (frame-char-height))))))
(let ((header-line-format "h")) (list (/ (- (cdr (window-text-pixel-size nil nil nil nil nil 'header-line)) (cdr (window-text-pixel-size))) (frame-char-height)) (/ (- (cdr (window-text-pixel-size nil nil nil nil nil t)) (cdr (window-text-pixel-size nil nil nil nil nil 'mode-line))) (frame-char-height))))
(let ((b (get-buffer-create "ml-other")) (w (selected-window))) (with-current-buffer b (setq mode-line-format nil)) (list (with-current-buffer b (= (window-mode-line-height w) (frame-char-height))) (progn (set-window-buffer w b) (list (window-mode-line-height w) (- (window-total-height w) (window-body-height w))))))
(condition-case e (window-header-line-height 'x) (error e))
"###;

#[test]
fn header_line_heights_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        let source = format!("(prin1-to-string (condition-case e {expression} (error (list 'signal e))))");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(princ {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display()).map(|printed| {
            serde_json::from_str::<String>(&printed).unwrap_or(printed)
        }).unwrap_or_else(|e| format!("ERR {e}"));
        if actual != expected {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
