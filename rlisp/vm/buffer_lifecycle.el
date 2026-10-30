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
            ;; A base buffer's indirect buffers die before it, and one that
            ;; survives its kill keeps the base alive (buffer.c:2019).
            (if (and (buffer-live-p buffer) (not (buffer-base-buffer buffer)))
                (let ((tail (buffer-list)))
                  (while tail
                    (let ((other (car tail)))
                      (if (eq (buffer-base-buffer other) buffer)
                          (progn
                            (kill-buffer other)
                            (if (buffer-live-p other)
                                (error "Unable to kill buffer whose indirect buffer `%s' cannot be killed"
                                       (buffer-name other))))))
                    (setq tail (cdr tail)))))
            ;; unlock_buffer: a modified buffer's file is unlocked.
            (if (and (buffer-live-p buffer) (buffer-modified-p buffer)
                     (stringp (buffer-local-value 'buffer-file-truename buffer)))
                (unlock-file (buffer-local-value 'buffer-file-truename buffer)))
            ;; kill_buffer_processes (process.c), once the hooks allowed the
            ;; kill; absent only in a VM without the process prelude.
            (if (and (buffer-live-p buffer) (fboundp 'romacs--kill-buffer-processes))
                (romacs--kill-buffer-processes buffer))
            (romacs--kill-buffer-finish buffer))
        nil)))
  (defun romacs--indirect-buffer-made (buffer base clone inhibit)
    ;; Fmake_indirect_buffer after its buffer exists (buffer.c:938): a
    ;; clone forgets its base's save query and file number and runs its
    ;; hook in BUFFER; then the buffer list's hook runs.
    (if clone
        (save-current-buffer
          (set-buffer buffer)
          (set 'buffer-save-without-query nil)
          (set 'buffer-file-number nil)
          (if (local-variable-p 'buffer-stale-function base)
              (kill-local-variable 'buffer-stale-function))
          (run-hooks 'clone-indirect-buffer-hook)))
    (or inhibit (run-hooks 'buffer-list-update-hook))
    buffer))
