//! A kill of a buffer, with window.el, against pinned GNU: a buffer a window
//! showed, or named only in its previous or next buffers or its
//! `quit-restore` parameter, is gone from all of them afterwards; advice on
//! `replace-buffer-in-windows`, or a redefinition, runs at every kill, named
//! or not. Romacs skips window.el's walk for a buffer no window named
//! (`romacs--windows-may-name-p`), which the last test counts.
#[path = "support/gnu_lisp.rs"]
mod gnu_lisp;
#[path = "support/gnu_batch.rs"]
mod gnu_batch;
use gnu_lisp::gnu_lisp;

const CASES: &[&str] = &[
    // Shown, then killed: the window shows the buffer it showed before, and
    // its history no longer holds the dead one.
    r#"(let ((a (get-buffer-create "kn-a")) (b (get-buffer-create "kn-b")))
         (set-window-buffer nil a) (set-window-buffer nil b) (kill-buffer b)
         (list (eq (window-buffer) a) (assq b (window-prev-buffers)) (memq b (window-next-buffers))))"#,
    // Named only in the previous buffers.
    r#"(let ((p (get-buffer-create "kn-p")))
         (set-window-prev-buffers nil (cons (list p 1 1) (window-prev-buffers)))
         (kill-buffer p) (assq p (window-prev-buffers)))"#,
    // Named only in the next buffers.
    r#"(let ((n (get-buffer-create "kn-n")))
         (set-window-next-buffers nil (cons n (window-next-buffers)))
         (kill-buffer n) (memq n (window-next-buffers)))"#,
    // Named only by `quit-restore' and `quit-restore-prev'.
    r#"(let ((q (get-buffer-create "kn-q")) (r (get-buffer-create "kn-r")))
         (set-window-parameter nil 'quit-restore (list 'window 'window (selected-window) q))
         (set-window-parameter nil 'quit-restore-prev (list 'same (list r 1 1 10) (selected-window) (window-buffer)))
         (kill-buffer q) (kill-buffer r)
         (list (window-parameter nil 'quit-restore) (window-parameter nil 'quit-restore-prev)))"#,
    // Advice sees every kill, named or not, temporary and inhibited ones too.
    r#"(let* ((n 0) (counter (lambda (&rest _) (setq n (1+ n)))))
         (advice-add 'replace-buffer-in-windows :before counter)
         (kill-buffer (get-buffer-create "kn-x"))
         (with-temp-buffer nil)
         (kill-buffer (get-buffer-create "kn-y" t))
         (advice-remove 'replace-buffer-in-windows counter)
         n)"#,
    // So does a redefinition.
    r#"(let* ((old (symbol-function 'replace-buffer-in-windows)) (n 0))
         (fset 'replace-buffer-in-windows (lambda (&optional b) (setq n (1+ n)) (funcall old b)))
         (kill-buffer (get-buffer-create "kn-z"))
         (fset 'replace-buffer-in-windows old)
         n)"#,
];

#[test]
fn kill_cleans_every_window_naming_as_pinned_gnu() {
    let expected = gnu_batch::outputs(&CASES.iter().map(|c| format!("(prin1 {c})")).collect::<Vec<_>>());
    let actual = std::thread::Builder::new().stack_size(256 << 20).spawn(|| {
        let mut vm = gnu_lisp(&[]);
        CASES.iter().map(|c| vm.eval_source(&format!("(prin1-to-string {c})")).and_then(romacs_vm::Local::display)
            .map_or_else(|e| format!("error: {e}"), |s| serde_json::from_str::<String>(&s).unwrap_or(s))).collect::<Vec<_>>()
    }).unwrap().join().unwrap();
    for ((case, expected), actual) in CASES.iter().zip(&expected).zip(&actual) {
        assert_eq!(actual, expected, "{case}");
    }
}

/// Romacs only (GNU walks the windows at every kill): window.el's walk, seen
/// through its `window-list-1`, runs for a buffer a window named and not for
/// one none did, including a temporary buffer; advised, it runs for both;
/// with the advice removed the skip comes back.
#[test]
fn kill_skips_windows_for_a_buffer_no_window_named() {
    let actual = std::thread::Builder::new().stack_size(256 << 20).spawn(|| {
        let mut vm = gnu_lisp(&[]);
        vm.eval_source(r#"(prin1-to-string
          (let* ((walks 0) (walk (lambda (&rest _) (setq walks (1+ walks))))
                 (count (lambda (thunk) (setq walks 0) (funcall thunk) walks))
                 (ignore (lambda (&rest _) nil)))
            (advice-add 'window-list-1 :before walk)
            (prog1
                (list (funcall count (lambda () (kill-buffer (get-buffer-create "ks-unnamed"))))
                      (funcall count (lambda () (with-temp-buffer nil)))
                      (funcall count (lambda () (let ((b (get-buffer-create "ks-prev")))
                                                  (set-window-prev-buffers nil (cons (list b 1 1) (window-prev-buffers)))
                                                  (kill-buffer b))))
                      (progn (advice-add 'replace-buffer-in-windows :before ignore)
                             (funcall count (lambda () (kill-buffer (get-buffer-create "ks-advised")))))
                      (progn (advice-remove 'replace-buffer-in-windows ignore)
                             (funcall count (lambda () (kill-buffer (get-buffer-create "ks-after"))))))
              (advice-remove 'window-list-1 walk))))"#).and_then(romacs_vm::Local::display)
            .map_or_else(|e| format!("error: {e}"), |s| serde_json::from_str::<String>(&s).unwrap_or(s))
    }).unwrap().join().unwrap();
    assert_eq!(actual, "(0 0 1 1 0)");
}
