//! GNU inotify.c from Lisp (rlisp/vm/inotify.el, prims/inotify.rs): watch
//! descriptors shared by one file's watches and told apart by id, aspect
//! and descriptor errors, events read by batch `read-event' (create,
//! a rename's moved-from and moved-to with one cookie, delete, and the
//! IN_IGNORED that ends a deleted file's watch), `sleep-for' leaving them
//! queued, and removal. The expected value is pinned GNU 32.0.50's answer
//! to the same forms on the same steps.
use romacs_vm::Vm;
#[path = "support/gnu_ert_vm.rs"]
mod gnu_ert_vm;

const CASE: &str = r###"
(defun file-notify-handle-event (object)
  "filenotify.el's handler, which a VM without GNU's Lisp lacks."
  (interactive "e")
  (funcall (nth 2 object) (nth 1 object)))
(defun cadr (x) (car (cdr x)))
(defun case-wait (pred) (let ((end (+ (float-time) 3))) (while (and (not (funcall pred)) (< (float-time) end)) (read-event nil nil 0.05))))
(defun case-run (dir)
  (let* ((create-lockfiles nil) (log nil) (cookies nil)
         (file (expand-file-name "a.txt" dir))
         (other (expand-file-name "b.txt" dir))
         (cb (lambda (ev) (when (memq (car (cadr ev)) '(moved-from moved-to)) (push (nth 3 ev) cookies)) (push (list (cdr (car ev)) (cadr ev) (and (stringp (nth 2 ev)) (file-name-nondirectory (nth 2 ev))) (integerp (nth 3 ev))) log)))
         (d (inotify-add-watch dir '(create moved-from moved-to delete) cb))
         (d2 (inotify-add-watch dir 'create cb))
         (r (list (featurep 'inotify) (inotify-valid-p 0) (inotify-valid-p nil) (inotify-valid-p '(0 . 0))
                  (equal (car d) (car d2)) (cdr d) (cdr d2) (inotify-valid-p d)
                  (condition-case e (inotify-add-watch dir 'bogus cb) (file-notify-error (list (car e) (nth 1 e) (nth 3 e))))
                  (condition-case e (inotify-add-watch dir '(create . bogus) cb) (error (car e)))
                  (condition-case e (inotify-add-watch (expand-file-name "missing/x" dir) t cb) (file-notify-error (list (car e) (nth 1 e))))
                  (condition-case e (inotify-rm-watch 'x) (file-notify-error (list (car e) (nth 1 e)))))))
    (write-region "x" nil file nil 'silent)
    ;; sleep-for leaves events queued for the keyboard, as GNU's does.
    (sleep-for 0.2)
    (push (list 'after-sleep (length log)) r)
    (case-wait (lambda () (>= (length log) 2)))
    (rename-file file other)
    (case-wait (lambda () (>= (length log) 4)))
    (inotify-rm-watch d2)
    (push (list (inotify-valid-p d) (inotify-valid-p d2)) r)
    (delete-file other)
    (case-wait (lambda () (>= (length log) 5)))
    (inotify-rm-watch d)
    (push (inotify-valid-p d) r)
    (let ((f2 (expand-file-name "c.txt" dir)) (self nil))
      (write-region "y" nil f2 nil 'silent)
      (let ((w (inotify-add-watch f2 '(modify) (lambda (ev) (when (memq (car (cadr ev)) '(moved-from moved-to)) (push (nth 3 ev) cookies)) (push (cadr ev) self)))))
        (delete-file f2)
        (case-wait (lambda () (memq 'ignored (apply #'append self))))
        (push (list (reverse self) (inotify-valid-p w)) r)))
    (list (nreverse r) (reverse log) (and (= (length cookies) 2) (equal (car cookies) (cadr cookies)) (/= 0 (car cookies))))))
"###;

const GNU: &str = r###"(((file-notify-error "Invalid descriptor ") (file-notify-error "Could not add watch for file") wrong-type-argument (file-notify-error "Unknown aspect" bogus) t 1 0 t nil nil nil t (after-sleep 0) (t nil) nil (((ignored)) nil)) ((0 (create) "a.txt" t) (1 (create) "a.txt" t) (0 (moved-from) "a.txt" t) (0 (moved-to) "b.txt" t) (0 (delete) "b.txt" t)) t)"###;

#[test]
fn inotify_watches_and_events_match_gnu() {
    let dir = std::env::temp_dir().join(format!("romacs-inotify-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source(&format!("(progn {CASE})")).unwrap();
    // prin1, as GNU printed its answer.
    let got = vm.eval_source(&format!("(string-to-unibyte (prin1-to-string (case-run {:?})))", dir.display().to_string()))
        .and_then(|v| v.bytes().map(|b| String::from_utf8_lossy(b).into_owned())).unwrap_or_else(|e| format!("ERROR {e}"));
    std::fs::remove_dir(&dir).unwrap();
    assert_eq!(got, GNU);
}

/// Without a live watch, a batch read with no time limit still ends at end
/// of input, as before file notification.
#[test]
fn batch_read_without_a_watch_ends_at_end_of_input() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let got = vm.eval_source("(condition-case e (read-event) (end-of-file (car e)))").and_then(|v| v.display()).unwrap();
    assert_eq!(got, "end-of-file");
}

/// keyboard.c insert-special-event: GNU's kind check, then the event waits
/// in the special-event queue for read_char, which runs its binding.
#[test]
fn insert_special_event_matches_gnu() {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let got = vm.eval_source(r#"(string-to-unibyte (prin1-to-string (list (condition-case e (insert-special-event '(bogus)) (error e))
        (condition-case e (insert-special-event 1) (error e))
        (let ((got nil))
          (current-global-map)
          (define-key special-event-map [file-notify] (lambda (ev) (interactive "e") (setq got ev)))
          (insert-special-event (list 'file-notify (list 1 2) 'cb))
          (read-event nil nil 0.1)
          got))))"#)
        .and_then(|v| v.bytes().map(|b| String::from_utf8_lossy(b).into_owned())).unwrap_or_else(|e| format!("ERROR {e}"));
    assert_eq!(got, r#"((error "Invalid special event kind" bogus) (wrong-type-argument consp 1) (file-notify (1 2) cb))"#);
}

/// A batch read with no time limit waits only while a watch is live: a
/// timer that removes the last one ends it at end of input, not in a poll
/// of the closed instance. The guard timer tells a hang from the end.
#[test]
fn batch_read_ends_when_a_timer_removes_the_last_watch() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let dir = std::env::temp_dir().join(format!("romacs-inotify-timer-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let got = vm.eval_source(&format!(r#"(let ((w (inotify-add-watch {:?} 'create #'ignore)))
          (run-with-timer 0.2 nil (lambda () (inotify-rm-watch w)))
          (catch 'guard
            (let ((guard (run-with-timer 3 nil (lambda () (throw 'guard 'hung)))))
              (prog1 (condition-case nil (read-event) (end-of-file 'end-of-input))
                (cancel-timer guard)))))"#, dir.display().to_string()))
        .and_then(|v| v.display()).unwrap_or_else(|e| format!("ERROR {e}"));
    std::fs::remove_dir(&dir).unwrap();
    assert_eq!(got, "end-of-input");
}
