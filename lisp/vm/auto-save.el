;;; auto-save.el --- fileio.c Fdo_auto_save's loop  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; The Lisp half of the `do-auto-save' subr (prims/autosave.rs), which
;; evaluates this form once per VM on first use. It transcribes
;; fileio.c:6315-6516 over the native `romacs--auto-save-1' (auto_save_1,
;; which sets `auto_saving'), `romacs--auto-save-done' (its unwind),
;; `romacs--auto-saved-tick' (BUF_AUTOSAVE_MODIFF = BUF_MODIFF),
;; `romacs--auto-save-failure-time' and `romacs--write-auto-save-list'
;; (the stdio stream of `auto-save-list-file-name').

;;; Code:

(progn
(defun romacs--do-auto-save (no-message current-only)
  "fileio.c Fdo_auto_save."
  (let* ((old (current-buffer))
         (auto-saved nil)
         (error-occurred nil)
         (oquit quit-flag)
         (no-message (or no-message (> (minibuffer-depth) 0)))
         ;; push_message: the message to give back afterwards.
         (old-message (and (not no-message) (current-message)))
         (listfile nil)
         (list-text nil))
    (setq quit-flag nil)
    (romacs--safe-run-hooks 'auto-save-hook)
    (when (stringp auto-save-list-file-name)
      (setq listfile (expand-file-name auto-save-list-file-name))
      (let ((dir (file-name-directory listfile)))
        ;; do_auto_save_make_dir: private, and its failure ignored.
        (unless (file-directory-p dir)
          (condition-case nil
              (with-file-modes #o700 (make-directory dir t))
            (t nil))))
      (setq list-text ""))
    (unwind-protect
        ;; First the buffers whose auto-save file has no handler, then
        ;; those whose has.
        (dolist (handled '(nil t))
          (dolist (b (buffer-list))
            (let ((name (buffer-local-value 'buffer-auto-save-file-name b))
                  (saved-size (buffer-local-value 'buffer-saved-size b)))
              (when (and (stringp name) list-text (not handled))
                (setq list-text
                      (concat list-text
                              (or (buffer-local-value 'buffer-file-name b) "")
                              "\n" name "\n")))
              (when (and (or (not current-only) (eq b (current-buffer)))
                         (stringp name)
                         ;; SAVE_MODIFF < MODIFF and AUTOSAVE_MODIFF < MODIFF.
                         (eq (buffer-modified-p b) t)
                         ;; -1 means auto-saving is off until a real save.
                         (fixnump saved-size) (>= saved-size 0)
                         (or handled (null (find-file-name-handler name 'write-region))))
                (let ((before (time-convert nil 'integer))
                      (failure (with-current-buffer b (romacs--auto-save-failure-time))))
                  ;; After a failure, not again for 20 minutes.
                  (unless (and (> failure 0) (< (- before failure) 1200))
                    (set-buffer b)
                    (if (and (not auto-save-include-big-deletions)
                             (> saved-size 5000)
                             (< (* 4 (buffer-size)) (* 3 saved-size))
                             buffer-file-name
                             (not no-message))
                        ;; It has shrunk too much: no more auto-saving
                        ;; here, or warnings, until a real save.
                        (progn
                          (message "Buffer %s has shrunk a lot; auto save disabled in that buffer until next real save"
                                   (buffer-name b))
                          (setq buffer-saved-size -1)
                          (sleep-for 1))
                      (unless (or auto-saved no-message) (message "Auto-saving..."))
                      (condition-case err
                          (romacs--auto-save-1)
                        (t
                         ;; auto_save_error
                         (setq error-occurred t)
                         (ding)
                         (display-warning 'auto-save
                                          (format "Auto-saving %s: %s" (buffer-name)
                                                  (error-message-string err))
                                          :error)))
                      (setq auto-saved t)
                      (with-current-buffer b (romacs--auto-saved-tick))
                      (setq buffer-saved-size (buffer-size))
                      (set-buffer old)
                      ;; More than a minute: an NFS failure that timed out.
                      (let ((after (time-convert nil 'integer)))
                        (when (> (- after before) 60)
                          (with-current-buffer b (romacs--auto-save-failure-time after)))))))))))
      (romacs--auto-save-done)
      (when listfile (romacs--write-auto-save-list listfile list-text)))
    (setq quit-flag oquit)
    (when (and auto-saved (not no-message))
      (if old-message
          ;; Give time to read ours before the old message comes back,
          ;; unlogged (restore_message's message3_nolog).
          (progn (sit-for 1) (let ((message-log-max nil)) (message "%s" old-message)))
        (unless error-occurred (message "Auto-saving...done"))))
    nil))
)
