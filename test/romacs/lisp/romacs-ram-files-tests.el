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
            ;; Left out whatever its state, and nothing holds the restart.
            (should (eq (romacs-ram-files-tests--strategy secret) 'dropped))
            (insert "password")
            (should (eq (romacs-ram-files-tests--strategy secret) 'dropped))
            (save-buffer)
            (should (eq (romacs-ram-files-tests--strategy secret) 'dropped))
            ;; A major mode clears the buffer's locals; the protection holds.
            (text-mode)
            (insert "more")
            (should (eq (romacs-ram-files-tests--strategy secret) 'dropped))
            (set-buffer-modified-p nil))
          (with-current-buffer plain
            (should (eq (romacs-ram-files-tests--strategy plain) 'file)))
          (kill-buffer secret)
          (kill-buffer plain))
      (delete-directory runtime t)
      (delete-directory elsewhere t))))

(ert-deftest romacs-ram-files-tests-a-restore-names-unsaved-text-it-did-not-carry ()
  (let ((shown nil))
    (cl-letf (((symbol-function 'message) (lambda (format &rest args) (setq shown (apply #'format format args)))))
      (warm-restart--announce
       (list :buffers 3 :records 3 :seconds 0.1
             :lost (list (list :buffer "entry.txt" :reason "a memory-only file, kept off the disk" :unsaved t)
                         (list :buffer "*Help*" :reason "generated buffer")))))
    (should (string-match-p "unsaved text not carried: entry\\.txt" shown))
    (should-not (string-match-p "\\*Help\\*" shown))
    ;; An incomplete restore warns, and names it too.
    (cl-letf (((symbol-function 'display-warning) (lambda (_type text &rest _) (setq shown text))))
      (warm-restart--announce
       (list :buffers 2 :records 3 :seconds 0.1 :file "/tmp/x.image"
             :lost (list (list :buffer "entry.txt" :reason "a memory-only file, kept off the disk" :unsaved t)))))
    (should (string-match-p "incomplete.*unsaved text not carried: entry\\.txt" shown))))

(ert-deftest romacs-ram-files-tests-an-image-leaves-out-unsaved-ram-text-and-says-so ()
  (let* ((runtime (make-temp-file "romacs-ram-files-" t))
         (process-environment (cons (concat "XDG_RUNTIME_DIR=" runtime) process-environment))
         (image (expand-file-name "session.image" (make-temp-file "romacs-ram-files-image-" t)))
         (secret (find-file-noselect (expand-file-name "entry.txt" runtime))))
    (unwind-protect
        (progn
          (with-current-buffer secret
            (romacs-ram-files-keep-off-disk)
            (insert "secret-text"))
          (warm-restart-write nil image)
          (should (equal (seq-filter (lambda (entry) (equal (plist-get entry :buffer) "entry.txt"))
                                     (plist-get (car (warm-restart--read image)) :lost))
                         '((:buffer "entry.txt" :reason "a memory-only file, kept off the disk" :unsaved t))))
          (with-temp-buffer
            (insert-file-contents-literally image)
            (should-not (search-forward "secret-text" nil t))))
      (with-current-buffer secret (set-buffer-modified-p nil))
      (kill-buffer secret)
      (delete-directory runtime t)
      (delete-directory (file-name-directory image) t))))

;;; romacs-ram-files-tests.el ends here
