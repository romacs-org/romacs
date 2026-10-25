;;; romacs-ert-batch.el --- GNU ERT for Romacs batch processes
;; Copyright (C) 2007-2026 Free Software Foundation, Inc.
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; Adapted from GNU ert.el's test context and condition formatter at the pin.
;; GNU permits removing its precautionary window excursion if expensive.
;; Batch execution retains the temporary buffer and test-body cleanup, while
;; automatic window-layout isolation and UI fixtures require native support.
;; Batch conditions use the native readable printer without editor indentation.

;;; Code:

(require 'ert)

(unless noninteractive
  (error "Romacs ERT batch adapter requires a batch process"))

(defmacro romacs-ert--batch-context (&rest body)
  "Execute BODY in a batch process without a window-layout sandbox."
  (declare (indent 0) (debug t))
  `(progn
     (unless noninteractive
       (error "Romacs ERT batch adapter requires a batch process"))
     ,@body))

(defun ert--run-test-internal (test-execution-info)
  "Low-level function to run a test according to TEST-EXECUTION-INFO.

This mainly sets up debugger-related bindings."
  (setf (ert--test-execution-info-ert-debug-on-error test-execution-info)
        ert-debug-on-error)
  (catch 'ert--pass
    (with-temp-buffer
      (romacs-ert--batch-context
        (let ((lexical-binding t)
              (ert--infos '()))
          (letrec ((debugfun (lambda (err)
                              (ert--run-test-debugger test-execution-info
                                                      err debugfun))))
            (handler-bind ((t debugfun))
              (funcall (ert-test-body (ert--test-execution-info-test
                                       test-execution-info))))))))
    (ert-pass))
  (setf (ert--test-execution-info-result test-execution-info)
        (make-ert-test-passed))
  nil)

(defun ert--pp-with-indentation-and-newline (object)
  "Print OBJECT in the batch buffer, followed by a newline."
  (let ((print-escape-newlines t)
        (print-escape-control-characters t))
    (prin1 object (current-buffer))
    (insert "\n")))

(provide 'romacs-ert-batch)
;;; romacs-ert-batch.el ends here
