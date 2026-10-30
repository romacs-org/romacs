//! window.c `previous-window' against pinned GNU in batch: the cyclic
//! order `next-window' walks, backwards, over three windows split from
//! GNU's 80x25 batch frame with window.c's own `split-window-internal'
//! (A over B, B beside C), with and without the minibuffer.  Windows are
//! named by letter, never printed.
use romacs_vm::Vm;

const SETUP: &str = "(let* ((a (selected-window)) (h (window-pixel-height a))
  (b (progn (set-window-new-pixel a (- h (/ h 2))) (split-window-internal a (/ h 2) nil 0.5)))
  (w (window-pixel-width b))
  (c (progn (set-window-new-pixel b (- w (/ w 2))) (split-window-internal b (/ w 2) t 0.5)))
  (name (lambda (w) (cond ((eq w a) 'a) ((eq w b) 'b) ((eq w c) 'c) ((eq w (minibuffer-window)) 'm) (t w)))))";

const CASES: &[&str] = &[
    "(mapcar name (list (previous-window) (previous-window a) (previous-window b) (previous-window c)))",
    "(mapcar name (list (previous-window a t) (previous-window (minibuffer-window)) (previous-window (minibuffer-window) t)))",
    "(mapcar name (list (previous-window a 'lambda) (previous-window b nil t) (previous-window c nil 'visible) (previous-window a nil 0)))",
    "(mapcar name (list (next-window (previous-window b)) (previous-window (next-window c))))",
    "(condition-case e (previous-window 'x) (error (car e)))",
];

#[test]
fn previous_window_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES {
        let source = format!("{SETUP} (prin1-to-string (condition-case e {expression} (error (list 'signal e)))))");
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
