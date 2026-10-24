;; Typing latency in a real TTY session, designed to be MEASURABLE.
;;
;; The previous fixture was unmeasurable for two reasons, both fixed here:
;;   1. the edit loop (~0.26B cycles) was swamped by startup (~1.4B), so the
;;      subtraction was mostly noise -> do far more iterations;
;;   2. it scrolled with `forward-line', forcing full-screen redraws that
;;      saturated the pty, which throttles the app rather than measuring it
;;      -> stay on one screen and edit in place.
;;
;; Editing in place is also what typing actually is: one line changes, and
;; redisplay updates that line.
;;
;; The fixture records the iteration count it ACTUALLY completed, and the
;; runner exits non-zero if the sentinel never appears. Both exist because a
;; run that stops early still burns a believable number of instructions: a
;; baseline that quietly finished only part of its loop once read as a +16%
;; regression in the code under test, consistent across repeats because it
;; failed the same way each time. Always check the reported iters.
(run-at-time 0 nil (lambda ()
  (let* ((root (expand-file-name default-directory))
         (log  (expand-file-name "tmp/reval/typing.out" root))
         (done (expand-file-name "tmp/reval/typing.done" root))
         (file (expand-file-name "lisp/emacs-lisp/seq.el" root))
         (buf (find-file-noselect file))
         (n (string-to-number (or (getenv "TYPE_ITERS") "3000"))))
    (set-window-buffer (selected-window) buf)
    (with-current-buffer buf
      (emacs-lisp-mode)
      (font-lock-set-defaults)
      (goto-char (point-min))
      (forward-line 40)               ; settle mid-screen; no scrolling below
      (redisplay t)
      (let ((t0 (car (current-cpu-time))))
        (dotimes (_ n)
          (insert "x")
          (redisplay t)
          (delete-char -1)
          (redisplay t))
        (write-region (format "typing_us=%d iters=%d per_keystroke_us=%.1f\n"
                              (- (car (current-cpu-time)) t0) n
                              (/ (float (- (car (current-cpu-time)) t0)) (max 1 n)))
                      nil log t 'silent)))
    (write-region "done\n" nil done nil 'silent)
    (kill-emacs 0))))
