;;; romacs-ram-files.el --- Files in memory-only directories stay off the disk  -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A file under /dev/shm or $XDG_RUNTIME_DIR lives in memory only: pass
;; decrypts an entry there for editing so that it never reaches a disk.
;; A buffer visiting one makes no backup and no auto-save, and warm
;; restart never writes it into a session image: unmodified, it is left
;; out (the image would hold its undo); modified, a restart refuses until
;; it is saved or killed.  `romacs-defaults-mode' turns this on.

;;; Code:

(defvar warm-restart-blocker-functions)
(defvar warm-restart-regenerable)

(defun romacs-ram-files-directories ()
  "The memory-only directories: /dev/shm and $XDG_RUNTIME_DIR, as true names."
  (mapcar (lambda (directory) (file-name-as-directory (file-truename directory)))
          (seq-filter #'file-directory-p
                      (delq nil (list "/dev/shm" (getenv "XDG_RUNTIME_DIR"))))))

(defun romacs-ram-files-p (file &optional directories)
  "Non-nil when FILE's true name is under one of DIRECTORIES.
DIRECTORIES defaults to `romacs-ram-files-directories'."
  (let ((true (file-truename file)))
    (seq-some (lambda (directory) (string-prefix-p directory true))
              (or directories (romacs-ram-files-directories)))))

(defun romacs-ram-files--restart-blocker ()
  "Refuse a warm restart that would write this buffer's unsaved text to disk."
  (and (buffer-modified-p)
       "unsaved text of a memory-only file; save or kill it first"))

(defun romacs-ram-files-keep-off-disk ()
  "In `find-file-hook' and `after-change-major-mode-hook', since a mode
clears the buffer's locals: keep a memory-only file's buffer off the disk."
  (when (and buffer-file-name (romacs-ram-files-p buffer-file-name))
    (setq-local backup-inhibited t)
    (auto-save-mode -1)
    (setq-local warm-restart-regenerable "a memory-only file, kept off the disk")
    (add-hook 'warm-restart-blocker-functions #'romacs-ram-files--restart-blocker nil t)))

(provide 'romacs-ram-files)
;;; romacs-ram-files.el ends here
