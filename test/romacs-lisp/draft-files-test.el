;;; draft-files-test.el --- Disposable draft filesystem boundary -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; scripts/test-elisp supplies a private HOME, removed after the run.

(defun draft-files-test-path (name)
  (expand-file-name (concat "~/" name)))

(romacs-deftest draft-files-single-directory-and-private-modes ()
  (let* ((dir (draft-files-test-path "draft-files-private/"))
         (file (concat dir "message.draft")))
    (romacs-should-not (make-directory-internal dir))
    (set-file-modes dir #o700)
    (romacs-should (= (file-modes dir) #o700))
    (romacs-should-error (make-directory-internal dir) :type 'file-already-exists)
    (with-temp-buffer
      (insert "Subject: fixture\n\nDraft λ\n")
      (write-region (point-min) (point-max) file nil 'silent))
    (set-file-modes file #o600)
    (romacs-should (= (file-modes file) #o600))
    (romacs-should (equal (with-temp-buffer (insert-file-contents file) (buffer-string))
                         "Subject: fixture\n\nDraft λ\n"))
    (romacs-should-not (delete-file file nil))
    (romacs-should-not (file-attributes file))
    (romacs-should-not (delete-file file))
    (romacs-should-error (delete-file dir) :type 'file-error)
    (romacs-should (file-directory-p dir))))

(romacs-deftest draft-files-does-not-create-parents ()
  (let ((parent (draft-files-test-path "draft-files-missing-parent/")))
    (romacs-should-error (make-directory-internal (concat parent "/child")) :type 'file-missing)
    (romacs-should-not (file-attributes parent))))

(defun draft-files-test-handler (&rest args) args)
(put 'draft-files-test-handler 'operations '(delete-file))

(romacs-deftest draft-files-delete-handler-preserves-arguments ()
  (let* ((path (draft-files-test-path "draft-files-handled/"))
         (file-name-handler-alist '(("draft-files-handled" . draft-files-test-handler))))
    (romacs-should (equal (delete-file path nil) (list 'delete-file path nil)))
    (romacs-should (equal (delete-file path t) (list 'delete-file path t)))))

(romacs-deftest draft-files-native-trash-is-explicitly-unsupported ()
  ;; GNU supports trash: this is deliberately a Romacs-only refusal contract.
  (unless (featurep 'emacs)
    (let ((file (draft-files-test-path "draft-files-trash.draft")))
      (with-temp-buffer (insert "retained") (write-region (point-min) (point-max) file nil 'silent))
      (let ((condition (condition-case e (delete-file file t) (error e))))
        (romacs-should (string-match "delete-file trash.*unsupported" (car (cdr condition)))))
      (romacs-should (file-attributes file))
      (delete-file file nil))))

(provide 'draft-files-test)
