//! The plain kill (`plain_kill`): `kill-buffer` of an inhibited buffer the
//! Lisp half would only make not current, done without that half. Against
//! pinned GNU, the state `with-temp-buffer` scopes leave; against the Lisp
//! path, the buffer made current when the killed one was; and which kills
//! take the half, one condition broken at a time.
#[path = "support/gnu_lisp.rs"]
mod gnu_lisp;
#[path = "support/gnu_batch.rs"]
mod gnu_batch;
use gnu_lisp::gnu_lisp;

fn printed(source: &str) -> String {
    let source = source.to_owned();
    std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let mut vm = gnu_lisp(&[]);
        vm.eval_source(&format!("(prin1-to-string {source})")).and_then(romacs_vm::Local::display)
            .map_or_else(|e| format!("error: {e}"), |s| serde_json::from_str::<String>(&s).unwrap_or(s))
    }).unwrap().join().unwrap()
}

const CASES: &[&str] = &[
    // Markers and an overlay made in the scope.
    r#"(let (m o b) (with-temp-buffer (insert "hello world") (setq b (current-buffer) m (copy-marker 3) o (make-overlay 2 5)))
         (list (buffer-live-p b) (buffer-name b) (marker-buffer m) (marker-position m) (overlay-buffer o) (overlay-start o)))"#,
    // Locals, and the buffer gone from the list.
    r#"(let (b) (with-temp-buffer (setq b (current-buffer)) (setq-local fill-column 3) (setq-local kp-local 1))
         (list (buffer-live-p b) (memq b (buffer-list)) (eq (current-buffer) (window-buffer))))"#,
    // A nested scope, then the outer one.
    r#"(let (outer inner seen) (with-temp-buffer (setq outer (current-buffer)) (with-temp-buffer (setq inner (current-buffer)))
         (setq seen (list (eq (current-buffer) outer) (buffer-live-p inner))))
         (append seen (list (buffer-live-p outer) (eq (current-buffer) (window-buffer)))))"#,
    // A body that leaves another buffer current: the kill is of a buffer not current.
    r#"(let ((start (current-buffer)) b) (with-temp-buffer (setq b (current-buffer)) (set-buffer (get-buffer-create "kp-other")))
         (list (eq (current-buffer) start) (buffer-live-p b) (buffer-live-p (get-buffer "kp-other"))))"#,
    // A temporary buffer killed while current, in its own right.
    r#"(let ((start (current-buffer)) (b (generate-new-buffer " *kp*" t)))
         (set-buffer b) (list (kill-buffer b) (buffer-live-p b) (null (memq b (buffer-list))) (buffer-live-p (current-buffer))))"#,
];

#[test]
fn plain_kill_leaves_the_state_gnu_leaves() {
    let expected = gnu_batch::outputs(&CASES.iter().map(|c| format!("(prin1 {c})")).collect::<Vec<_>>());
    for (case, expected) in CASES.iter().zip(&expected) {
        assert_eq!(&printed(case), expected, "{case}");
    }
}

/// Killing the current temporary buffer in a session where one buffer is
/// shown and another buried: the plain kill makes current the buffer the
/// Lisp half makes current (`other-buffer''s choice) past a shown buffer,
/// twice. The half is forced by advice on `replace-buffer-in-windows`.
#[test]
fn plain_kill_makes_current_what_the_lisp_half_does() {
    let scenario = r#"(lambda (force)
        (let ((ignore (lambda (&rest _) nil)) (names nil))
          (when force (advice-add 'replace-buffer-in-windows :before ignore))
          (let ((a (get-buffer-create "kp-a")) (b (get-buffer-create "kp-b")) (c (get-buffer-create "kp-c")))
            (dolist (x (buffer-list)) (unless (string-prefix-p "kp-" (buffer-name x)) (bury-buffer-internal x)))
            (set-window-buffer nil a) (bury-buffer-internal b)
            (let ((tmp (generate-new-buffer " *kp-tmp*" t)))
              (set-buffer tmp) (kill-buffer tmp) (push (buffer-name (current-buffer)) names))
            (set-window-buffer nil c)
            (let ((tmp (generate-new-buffer " *kp-tmp*" t)))
              (set-buffer tmp) (kill-buffer tmp) (push (buffer-name (current-buffer)) names)))
          (when force (advice-remove 'replace-buffer-in-windows ignore))
          (nreverse names)))"#;
    let lisp = printed(&format!("(funcall {scenario} t)"));
    let plain = printed(&format!("(funcall {scenario} nil)"));
    assert_eq!(plain, lisp, "the plain kill's successor differs from the Lisp half's");
    // kp-a shown, so kp-c; then kp-c shown, so kp-a, kp-b being buried.
    assert_eq!(lisp, "(\"kp-c\" \"kp-a\")");
}

/// Each condition of `plain_kill`, broken alone, takes the Lisp half: advice
/// counts the half's calls. Plain kills first, then: hooks not inhibited, a
/// file name, a file truename, an indirect buffer made on it, a process, a
/// window history naming it, advice on `replace-buffer-in-windows`, its
/// redefinition, a frame `buffer-predicate` with the buffer current, and
/// the minibuffer's buffer current.
#[test]
fn plain_kill_eligibility() {
    assert_eq!(printed(r#"(let* ((n 0) (count (lambda (&rest _) (setq n (1+ n))))
             (half (lambda (thunk) (setq n 0) (funcall thunk) n))
             (fresh (lambda () (generate-new-buffer " *kp*" t)))
             (ignore (lambda (&rest _) nil)))
        (kill-buffer (get-buffer-create "kp-define-the-half"))
        (advice-add 'romacs--kill-buffer :before count)
        (prog1
            (list
             (funcall half (lambda () (with-temp-buffer nil)))
             (funcall half (lambda () (kill-buffer (funcall fresh))))
             (funcall half (lambda () (kill-buffer (generate-new-buffer " *kp*"))))
             (funcall half (lambda () (let ((b (funcall fresh))) (with-current-buffer b (setq buffer-file-name "/nonexistent/kp")) (kill-buffer b))))
             (funcall half (lambda () (let ((b (funcall fresh))) (with-current-buffer b (setq buffer-file-truename "/nonexistent/kp")) (kill-buffer b))))
             (funcall half (lambda () (let ((b (funcall fresh))) (make-indirect-buffer b " *kp-indirect*" nil t) (kill-buffer b))))
             (funcall half (lambda () (let ((b (funcall fresh))) (make-pipe-process :name "kp" :buffer b :noquery t) (kill-buffer b))))
             (funcall half (lambda () (let ((b (funcall fresh))) (set-window-prev-buffers nil (cons (list b 1 1) (window-prev-buffers))) (kill-buffer b))))
             (funcall half (lambda () (advice-add 'replace-buffer-in-windows :before ignore)
                                      (kill-buffer (funcall fresh))
                                      (advice-remove 'replace-buffer-in-windows ignore)))
             (funcall half (lambda () (let ((old (symbol-function 'replace-buffer-in-windows)))
                                        (fset 'replace-buffer-in-windows (lambda (&optional b) (funcall old b)))
                                        (kill-buffer (funcall fresh))
                                        (fset 'replace-buffer-in-windows old))))
             (funcall half (lambda () (let ((b (funcall fresh)))
                                        (modify-frame-parameters nil (list (cons 'buffer-predicate (lambda (_) t))))
                                        (set-buffer b) (kill-buffer b)
                                        (modify-frame-parameters nil (list (cons 'buffer-predicate nil))))))
             (funcall half (lambda () (let ((b (funcall fresh)))
                                        (with-current-buffer (window-buffer (minibuffer-window)) (kill-buffer b)))))
             (funcall half (lambda () (kill-buffer (funcall fresh)))))
          (advice-remove 'romacs--kill-buffer count)))"#),
        "(0 0 1 1 1 1 1 1 1 1 1 1 0)");
}
