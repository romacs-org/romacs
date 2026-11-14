;;; romacs-timer-pass-tests.el --- The timer pass lets pending input go first  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; lisp/romacs/vm/subprocess.el `romacs--timer-check': past
;; `romacs-timer-pass-budget', with input pending, the ripe timers left
;; wait for the next pass, in order.

(require 'ert)

(defun romacs-timer-pass-tests--busy (seconds)
  "Work for SECONDS."
  (let ((end (+ (float-time) seconds)))
    (while (< (float-time) end))))

(ert-deftest timer-pass-yields-to-pending-input ()
  "Five ripe 5 ms timers: with input pending the pass stops past 8 ms and
answers 0; the next pass runs the rest in order."
  (let ((timer-list nil) (timer-idle-list nil) (ran nil)
        (romacs-timer-pass-budget 0.008)
        (unread-command-events (list ?a)))
    (dotimes (i 5)
      (run-at-time 0 nil (lambda () (romacs-timer-pass-tests--busy 0.005) (push i ran))))
    (should (eql 0 (romacs--timer-check)))
    (should (<= 2 (length ran) 3))
    (setq unread-command-events nil)
    (romacs--timer-check)
    (should (equal '(0 1 2 3 4) (reverse ran)))))

(ert-deftest timer-pass-runs-all-without-input ()
  "Without pending input, or with no budget, every ripe timer runs."
  (dolist (budget '(0.008 nil))
    (let ((timer-list nil) (timer-idle-list nil) (ran nil)
          (romacs-timer-pass-budget budget)
          (unread-command-events (and (null budget) (list ?a))))
      (dotimes (i 5)
        (run-at-time 0 nil (lambda () (romacs-timer-pass-tests--busy 0.005) (push i ran))))
      (romacs--timer-check)
      (should (equal '(0 1 2 3 4) (reverse ran))))))

(provide 'romacs-timer-pass-tests)
;;; romacs-timer-pass-tests.el ends here
