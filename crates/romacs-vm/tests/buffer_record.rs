//! `select-window' records its buffer at the front of `buffer-list' unless
//! NORECORD (window.c select_window, buffer.c record_buffer), and
//! `window-discard-buffer-from-window' drops a buffer from a window's
//! previous and next buffers (window.c:3445); both as pinned GNU answers.
use romacs_vm::Vm;

const EXPRESSIONS: &[&str] = &[
    r#"(let ((a (get-buffer-create "rec-a")) (b (get-buffer-create "rec-b")))
         (set-window-buffer nil a) (select-window (selected-window))
         (let ((first (eq (car (buffer-list)) a)))
           (set-window-buffer nil b) (select-window (selected-window) t)
           (list first (eq (car (buffer-list)) a)
                 (progn (select-window (selected-window)) (eq (car (buffer-list)) b)))))"#,
    r#"(let ((a (get-buffer-create "dis-a")) (b (get-buffer-create "dis-b")) (w (selected-window)))
         (set-window-prev-buffers w (list (list a 1 1) (list b 1 1)))
         (set-window-next-buffers w (list a b))
         (window-discard-buffer-from-window a w)
         (list (length (window-prev-buffers w)) (eq (car (car (window-prev-buffers w))) b)
               (length (window-next-buffers w)) (eq (car (window-next-buffers w)) b)))"#,
    r#"(let ((a (get-buffer-create "dis-q")) (w (selected-window)))
         (set-window-parameter w 'quit-restore (list 'window 'window nil a))
         (set-window-parameter w 'quit-restore-prev (list 'other 'x))
         (window-discard-buffer-from-window a w)
         (let ((kept (window-parameter w 'quit-restore)))
           (window-discard-buffer-from-window a w t)
           (list (and kept t) (window-parameter w 'quit-restore) (window-parameter w 'quit-restore-prev))))"#,
    r#"(condition-case err (window-discard-buffer-from-window (let ((b (get-buffer-create "dis-dead"))) (kill-buffer b) b) (selected-window)) (error err))"#,
];

#[test]
fn buffer_recording_matches_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = Vm::new();
    let mut failures = Vec::new();
    for expression in EXPRESSIONS {
        let source = format!("(prin1-to-string {expression})");
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(princ {source})")])
            .output()
            .unwrap();
        assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display()).map(|printed| {
            serde_json::from_str::<String>(&printed).unwrap_or(printed)
        }).unwrap_or_else(|e| format!("ERR {e}"));
        if actual != expected {
            failures.push(format!("{expression}\n  GNU    {expected}\n  Romacs {actual}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
