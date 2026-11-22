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
(defun romacs--auto-save-list-file ()
  "`auto-save-list-file-name' expanded, its directory made private when it
is missing (do_auto_save_make_dir, its failure ignored)."
  (let* ((file (expand-file-name auto-save-list-file-name))
         (dir (file-name-directory file)))
    (unless (file-directory-p dir)
      (condition-case nil
          (with-file-modes #o700 (make-directory dir t))
        (t nil)))
    file))

(defun romacs--auto-save-list-entry (b name)
  "Buffer B's lines in the auto-save list: its visited file's name, or
nothing, then NAME, its auto-save file's."
  (concat (or (buffer-local-value 'buffer-file-name b) "") "\n" name "\n"))

(defun romacs--auto-save-in-pass-p (b name saved-size handled current-only)
  "Whether pass HANDLED saves buffer B, NAME its auto-save file's name and
SAVED-SIZE its `buffer-saved-size': nil saves the buffers whose auto-save
file has no `write-region' handler, t the rest.  With CURRENT-ONLY, only
the current buffer is saved."
  (and (or (not current-only) (eq b (current-buffer)))
       (stringp name)
       ;; SAVE_MODIFF < MODIFF and AUTOSAVE_MODIFF < MODIFF.
       (eq (buffer-modified-p b) t)
       ;; -1 means auto-saving is off until a real save.
       (fixnump saved-size) (>= saved-size 0)
       (or handled (null (find-file-name-handler name 'write-region)))))

(defun romacs--auto-save-failed-recently-p (now)
  "Whether the current buffer's auto-save failed less than 20 minutes
before NOW, in seconds."
  (let ((failure (romacs--auto-save-failure-time)))
    (and (> failure 0) (< (- now failure) 1200))))

(defun romacs--auto-save-shrunk-p (saved-size no-message)
  "Whether the current buffer, visiting a file, shrank too much since its
SAVED-SIZE to be auto-saved over it; never with NO-MESSAGE."
  (and (not auto-save-include-big-deletions)
       (> saved-size 5000)
       (< (* 4 (buffer-size)) (* 3 saved-size))
       buffer-file-name
       (not no-message)))

(defun romacs--auto-save-buffer (b saved-size old no-message announce)
  "Auto-save buffer B, SAVED-SIZE its `buffer-saved-size', unless its last
failure is recent: `saved', `failed' (warned), `shrunk' (auto-saving off
there until a real save, B left current, as GNU leaves it) or nil.
ANNOUNCE says \"Auto-saving...\" first; OLD is current again after a save."
  (let ((before (time-convert nil 'integer)))
    (unless (with-current-buffer b (romacs--auto-save-failed-recently-p before))
      (set-buffer b)
      (if (romacs--auto-save-shrunk-p saved-size no-message)
          (progn
            (message "Buffer %s has shrunk a lot; auto save disabled in that buffer until next real save"
                     (buffer-name b))
            (setq buffer-saved-size -1)
            (sleep-for 1)
            'shrunk)
        (when announce (message "Auto-saving..."))
        (let ((outcome (condition-case err
                           (progn (romacs--auto-save-1) 'saved)
                         (t
                          ;; auto_save_error
                          (ding)
                          (display-warning 'auto-save
                                           (format "Auto-saving %s: %s" (buffer-name) (error-message-string err))
                                           :error)
                          'failed))))
          (with-current-buffer b (romacs--auto-saved-tick))
          (setq buffer-saved-size (buffer-size))
          (set-buffer old)
          ;; More than a minute: an NFS failure that timed out.
          (let ((after (time-convert nil 'integer)))
            (when (> (- after before) 60)
              (with-current-buffer b (romacs--auto-save-failure-time after))))
          outcome)))))

(defun romacs--auto-save-say-done (old-message error-occurred)
  "Say the auto-save is done, unless ERROR-OCCURRED; or give OLD-MESSAGE,
the message it replaced, back, unlogged (restore_message's message3_nolog),
once ours had time to be read."
  (if old-message
      (progn (sit-for 1) (let ((message-log-max nil)) (message "%s" old-message)))
    (unless error-occurred (message "Auto-saving...done"))))

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
      (setq listfile (romacs--auto-save-list-file) list-text ""))
    (unwind-protect
        (dolist (handled '(nil t))
          (dolist (b (buffer-list))
            (let ((name (buffer-local-value 'buffer-auto-save-file-name b))
                  (saved-size (buffer-local-value 'buffer-saved-size b)))
              (when (and (stringp name) list-text (not handled))
                (setq list-text (concat list-text (romacs--auto-save-list-entry b name))))
              (when (romacs--auto-save-in-pass-p b name saved-size handled current-only)
                (let ((outcome (romacs--auto-save-buffer b saved-size old no-message
                                                         (not (or auto-saved no-message)))))
                  (when (memq outcome '(saved failed)) (setq auto-saved t))
                  (when (eq outcome 'failed) (setq error-occurred t)))))))
      (romacs--auto-save-done)
      (when listfile (romacs--write-auto-save-list listfile list-text)))
    (setq quit-flag oquit)
    (when (and auto-saved (not no-message))
      (romacs--auto-save-say-done old-message error-occurred))
    nil))
)
