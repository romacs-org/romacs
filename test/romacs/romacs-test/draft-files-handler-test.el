;;; draft-files-handler-test.el --- delete-file through a file name handler -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; The handler path only: no file content, so it runs on the bare VM too.
;; tools/test-elisp supplies a private HOME, removed after the run.

(defun draft-files-handler-test--handler (&rest args) args)
(put 'draft-files-handler-test--handler 'operations '(delete-file))

(romacs-deftest draft-files-delete-handler-preserves-arguments ()
  (let* ((path (expand-file-name "~/draft-files-handled/"))
         (file-name-handler-alist '(("draft-files-handled" . draft-files-handler-test--handler))))
    (romacs-should (equal (delete-file path nil) (list 'delete-file path nil)))
    (romacs-should (equal (delete-file path t) (list 'delete-file path t)))))

(provide 'draft-files-handler-test)
