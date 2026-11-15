;;; romacs-ram-files-tests.el --- Memory-only files stay off the disk -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'warm-restart)
(require 'romacs-ram-files)

(ert-deftest romacs-ram-files-tests-a-runtime-directory-file-makes-no-backup-auto-save-or-image ()
  (let* ((runtime (make-temp-file "romacs-ram-files-" t))
         (elsewhere (make-temp-file "romacs-ram-files-disk-" t))
         (process-environment (cons (concat "XDG_RUNTIME_DIR=" runtime) process-environment))
         (auto-save-default t))
    (unwind-protect
        (let ((secret (find-file-noselect (expand-file-name "entry.txt" runtime)))
              (plain (find-file-noselect (expand-file-name "notes.txt" elsewhere))))
          (with-current-buffer secret
            (romacs-ram-files-keep-off-disk)
            (should backup-inhibited)
            (should-not buffer-auto-save-file-name)
            (should warm-restart-regenerable)
            ;; Unmodified, nothing stops a restart, which leaves it out;
            ;; modified, the restart refuses.
            (should-not (run-hook-with-args-until-success 'warm-restart-blocker-functions))
            (insert "password")
            (should (run-hook-with-args-until-success 'warm-restart-blocker-functions))
            (set-buffer-modified-p nil))
          ;; A file elsewhere is left as it was (GNU inhibits backups of
          ;; one under the temporary directory itself, whatever this does).
          (with-current-buffer plain
            (romacs-ram-files-keep-off-disk)
            (should-not warm-restart-regenerable)
            (should-not (memq #'romacs-ram-files--restart-blocker warm-restart-blocker-functions)))
          (kill-buffer secret)
          (kill-buffer plain))
      (delete-directory runtime t)
      (delete-directory elsewhere t))))

;;; romacs-ram-files-tests.el ends here
