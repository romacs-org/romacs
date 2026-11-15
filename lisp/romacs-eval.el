;;; romacs-eval.el --- GNU's eval.c primitives in Lisp -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; `funcall-with-delayed-message', which GNU writes in C over an atimer,
;; here over a timer: the message waits for FUNCTION to wait (process
;; output, `sit-for'), as a timer does, so a pure CPU loop never shows it.
;; A display-only safepoint action would lift that (romacs#245).

;;; Code:

(defun funcall-with-delayed-message (timeout message function)
  "Like `funcall', but display MESSAGE if FUNCTION takes longer than TIMEOUT.
TIMEOUT is a number of seconds, and can be an integer or a floating
point number.

If FUNCTION takes less time to execute than TIMEOUT seconds, MESSAGE
is not displayed.

In Romacs the message is shown when FUNCTION waits (for process output,
`sit-for', `sleep-for'), as a timer runs; during a computation that never
waits it is not shown."
  (unless (numberp timeout) (signal 'wrong-type-argument (list 'numberp timeout)))
  (unless (stringp message) (signal 'wrong-type-argument (list 'stringp message)))
  (let ((timer (run-with-timer timeout nil (lambda () (message "%s" message)))))
    (unwind-protect (funcall function)
      (cancel-timer timer))))

(put 'funcall-with-delayed-message 'romacs-lint-hint
     "its MESSAGE shows only while FUNCTION waits (process output, sit-for), not during a CPU loop")

(provide 'romacs-eval)
;;; romacs-eval.el ends here
