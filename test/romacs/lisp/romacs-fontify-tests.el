;;; romacs-fontify-tests.el --- Fontification worker capacity tests  -*- lexical-binding: t; -*-

(require 'ert)
(require 'cl-lib)
(require 'romacs-fontify)

(defun romacs-fontify-tests--stop-workers ()
  (isolate-kill-all)
  (let ((deadline (+ (float-time) 10)))
    (while (and (isolate-list) (< (float-time) deadline))
      (romacs--isolate-pump 0.1))
    (should-not (isolate-list))))

(ert-deftest romacs-fontify-retries-only-when-a-worker-will-free-room ()
  (let ((isolate-boot 'bare)
        (isolate-zygotes nil)
        (isolate-pool-size 1))
    (with-temp-buffer
      (cl-letf (((symbol-function 'romacs-fontify--wanted-p) (lambda () t)))
        (unwind-protect
            (progn
              (let ((isolate-queue-limit 0))
                (romacs-fontify--start)
                (should-not romacs-fontify--timer)
                (should-not (isolate-list)))
              (let ((isolate-process-limit 0))
                (romacs-fontify--start)
                (should-not romacs-fontify--timer))
              (let ((job (isolate-spawn 'list)))
                (romacs-fontify--start)
                (should (timerp romacs-fontify--timer))
                (should-not romacs-fontify--state)
                (should-not (isolate-await job 10))))
          (romacs-fontify--drop)
          (romacs-fontify-tests--stop-workers))))))

(ert-deftest romacs-fontify-retries-after-preparation-frees-the-only-queue-slot ()
  (let ((isolate-boot 'source)
        (isolate-zygotes nil)
        (isolate-queue-limit 1)
        (romacs-fontify-isolate-delay 60))
    (with-temp-buffer
      (insert "(defun f () t)")
      (emacs-lisp-mode)
      (font-lock-set-defaults)
      (cl-letf (((symbol-function 'romacs-fontify--wanted-p) (lambda () t)))
        (unwind-protect
            (progn
              (romacs-fontify--start)
              (should (timerp romacs-fontify--timer))
              (let* ((worker (car (isolate-list)))
                     (deadline (+ (float-time) 20)))
                (while (and (> (isolate-pending worker) 0) (< (float-time) deadline))
                  (romacs--isolate-pump 0.1))
                (should (zerop (isolate-pending worker)))
                (cancel-timer romacs-fontify--timer)
                (romacs-fontify--start)
                (should-not romacs-fontify--timer)
                (let ((future (aref romacs-fontify--state 2)))
                  (should (eq worker (isolate-future-isolate future)))
                  (isolate-await future 20)
                  (should (isolate-future-done-p future)))))
          (romacs-fontify--drop)
          (romacs-fontify-tests--stop-workers))))))

(provide 'romacs-fontify-tests)
;;; romacs-fontify-tests.el ends here
