;;; romacs-ram-files-tests.el --- Memory-only files stay off the disk -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'warm-restart)
(require 'romacs-ram-files)

(defun romacs-ram-files-tests--strategy (buffer)
  "BUFFER's strategy in the plan a warm restart would make now."
  (plist-get (seq-find (lambda (entry) (eq (plist-get entry :buffer) buffer))
                       (plist-get (warm-restart-plan) :buffers))
             :strategy))

(ert-deftest romacs-ram-files-tests-a-runtime-directory-file-makes-no-backup-auto-save-or-image ()
  (let* ((runtime (make-temp-file "romacs-ram-files-" t))
         (elsewhere (make-temp-file "romacs-ram-files-disk-" t))
         (process-environment (cons (concat "XDG_RUNTIME_DIR=" runtime) process-environment))
         (auto-save-default t)
         ;; As `romacs-defaults-mode' hooks it.
         (find-file-hook (list #'romacs-ram-files-keep-off-disk))
         (after-change-major-mode-hook (list #'romacs-ram-files-keep-off-disk)))
    (unwind-protect
        (let ((secret (find-file-noselect (expand-file-name "entry.txt" runtime)))
              (plain (find-file-noselect (expand-file-name "notes.txt" elsewhere))))
          (with-current-buffer secret
            (should backup-inhibited)
            (should-not buffer-auto-save-file-name)
            ;; Unmodified it is left out; modified the restart refuses;
            ;; saved it is left out again, undo and all.
            (should (eq (romacs-ram-files-tests--strategy secret) 'dropped))
            (insert "password")
            (should (eq (romacs-ram-files-tests--strategy secret) 'blocked))
            (save-buffer)
            (should (eq (romacs-ram-files-tests--strategy secret) 'dropped))
            ;; A major mode clears the buffer's locals; the protection holds.
            (text-mode)
            (insert "more")
            (should (eq (romacs-ram-files-tests--strategy secret) 'blocked))
            (set-buffer-modified-p nil))
          (with-current-buffer plain
            (should (eq (romacs-ram-files-tests--strategy plain) 'file)))
          (kill-buffer secret)
          (kill-buffer plain))
      (delete-directory runtime t)
      (delete-directory elsewhere t))))

;;; romacs-ram-files-tests.el ends here
