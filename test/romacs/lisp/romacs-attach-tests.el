;;; romacs-attach-tests.el --- Attachment helper preparation -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'ert)
(require 'cl-lib)
(require 'romacs-attach)

(ert-deftest romacs-attach-prepare-settles-a-failed-spawn ()
  (let* ((romacs-attach-jobs 2)
         (plan '(((:path "/tmp/first") :kind text) ((:path "/tmp/second") :kind text)))
         (spawn (symbol-function 'make-process))
         (outputs nil) (first nil) (results nil) (handle nil))
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-attach--program) (lambda () "sh"))
                  ((symbol-function 'romacs-attach--arguments) (lambda (&rest _) '("-c" "exec sleep 60")))
                  ((symbol-function 'make-process)
                   (lambda (&rest args)
                     (push (plist-get args :buffer) outputs)
                     (if first (error "Cannot start second helper")
                       (setq first (apply spawn args))))))
          (setq handle (romacs-attach-prepare plan temporary-file-directory
                                              (lambda (result) (push result results))))
          (accept-process-output nil 0.01)
          (should (equal results '((:error "Cannot start second helper"))))
          (should (plist-get handle :done))
          (should (zerop (plist-get handle :running)))
          (should-not (process-live-p first))
          (should-not (seq-some #'buffer-live-p outputs)))
      (when (process-live-p first) (delete-process first))
      (mapc (lambda (buffer) (when (buffer-live-p buffer) (kill-buffer buffer))) outputs))))

;;; romacs-attach-tests.el ends here
