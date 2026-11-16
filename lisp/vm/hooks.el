(progn
;;; hooks.el --- keyboard.c safe_run_hooks  -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; Loaded by both the prelude and the keyboard half (prims/keyboard.rs), which
;; a bare VM installs without the prelude.  One form, as each half is.

;;; Code:

(defun romacs--safe-run-hooks (hook)
  "keyboard.c safe_run_hooks: run HOOK, each function under a handler that
reports an error and removes that function from HOOK."
  (when (and (boundp hook) (symbol-value hook))
    (let ((inhibit-quit t))
      (run-hook-wrapped hook #'romacs--safe-run-hook-function hook))))

(defun romacs--safe-run-hook-function (function hook)
  "keyboard.c safe_run_hook_funcall and safe_run_hooks_error."
  (condition-case err
      (funcall function)
    (t (message "Error in %s (%S): %S" hook function err)
       (let ((without (lambda (list)
                        (let ((kept nil))
                          (while (consp list)
                            (unless (eq (car list) function) (push (car list) kept))
                            (setq list (cdr list)))
                          (nreverse kept)))))
         (cond ((and (consp (symbol-value hook)) (memq function (symbol-value hook)))
                (set hook (funcall without (symbol-value hook))))
               ((and (default-boundp hook) (consp (default-value hook))
                     (memq function (default-value hook)))
                (set-default hook (funcall without (default-value hook))))))))
  nil)

)
;;; hooks.el ends here
