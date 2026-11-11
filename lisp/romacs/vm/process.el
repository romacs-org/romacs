;;; process.el --- Owned background commands -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Workers exchange bytes only.  Callbacks run on the evaluator between commands,
;; never inside a recursive minibuffer read.  Callers own their destination buffers.

;;; Code:

(defvar romacs-process-handlers nil)

(defun romacs-process-start (argv input directory timeout callback)
  "Start ARGV with INPUT in DIRECTORY, bounded by TIMEOUT seconds.
CALLBACK receives (STATUS STDOUT STDERR ERROR) on evaluator-side completion."
  (let ((id (romacs--process-start argv input directory timeout)))
    (setq romacs-process-handlers (cons (cons id callback) romacs-process-handlers))
    id))

(defun romacs-process-forget (id)
  "Retire ID and its callback.  This never retries an external effect."
  (let ((entry (assq id romacs-process-handlers)))
    (setq romacs-process-handlers (delq entry romacs-process-handlers)))
  (romacs--process-forget id))

(defun romacs-process-dispatch-ready ()
  "Apply completed callbacks; return non-nil only for visible changes.
A callback's quit propagates after the other acknowledged completions settle."
  (let ((changed nil) (quit-condition nil))
    (dolist (id (romacs--process-ready))
      (let ((entry (assq id romacs-process-handlers)))
        (when entry
          (setq romacs-process-handlers (delq entry romacs-process-handlers))
          (unwind-protect
              (condition-case err
                  (save-current-buffer
                    (when (funcall (cdr entry) (romacs--process-result id))
                      (setq changed t)))
                (error (message "Background command: %s" err))
                (quit (setq quit-condition err)))
            (romacs--process-forget id)))))
    (when quit-condition (signal (car quit-condition) (cdr quit-condition)))
    changed))

(provide 'romacs-process)
;;; process.el ends here
