;;; buffer_lifecycle.el --- Buffer lifecycle policy

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Run buffer query and cleanup hooks on the evaluator thread.

;;; Code:

(progn
  (defvar kill-buffer-query-functions nil)
  (defvar kill-buffer-hook nil)
  (defun romacs--kill-hook-functions (value defaults)
    (if (functionp value) (list value)
      (let ((result nil))
        (while value
          (if (eq (car value) t)
              (setq result (append result (romacs--kill-hook-functions defaults nil)))
            (setq result (append result (list (car value)))))
          (setq value (cdr value)))
        result)))
  (defun romacs--kill-buffer (buffer inhibit)
    (let ((previous (current-buffer))
          (previous-point (if enable-multibyte-characters (copy-marker (point)) (point)))
          (queries nil) (hooks nil) (allow t))
      (unwind-protect
          (progn
            (set-buffer buffer)
            (unless inhibit
              (setq queries (romacs--kill-hook-functions kill-buffer-query-functions (default-value 'kill-buffer-query-functions)))
              (while (and allow queries)
                (setq allow (funcall (car queries)))
                (setq queries (cdr queries))))
            ;; buffer.c: an interactive session (no batch, no keyboard
            ;; macro) asks before killing a modified file buffer.
            (when (and allow (buffer-live-p buffer) (null executing-kbd-macro) (not noninteractive)
                       buffer-file-name (buffer-modified-p) (fboundp 'kill-buffer--possibly-save))
              (setq allow (kill-buffer--possibly-save buffer))
              (set-buffer buffer))
            (when (and allow (buffer-live-p buffer) (not inhibit))
              (set-buffer buffer)
              (setq hooks (romacs--kill-hook-functions kill-buffer-hook (default-value 'kill-buffer-hook)))
              (while hooks
                (funcall (car hooks))
                (setq hooks (cdr hooks)))))
        (when (buffer-live-p previous)
          (set-buffer previous)
          (goto-char (if (integerp previous-point) previous-point (marker-position previous-point))))
        (unless (integerp previous-point) (set-marker previous-point nil)))
      (if allow
          (progn
            ;; kill_buffer_processes (process.c), once the hooks allowed the
            ;; kill; absent only in a VM without the process prelude.
            (if (and (buffer-live-p buffer) (fboundp 'romacs--kill-buffer-processes))
                (romacs--kill-buffer-processes buffer))
            (romacs--kill-buffer-finish buffer))
        nil))))
