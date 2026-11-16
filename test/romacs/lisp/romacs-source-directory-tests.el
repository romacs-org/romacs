;;; romacs-source-directory-tests.el --- Help reads source-directory per visit -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-help)

(ert-deftest romacs-source-directory-names-the-tree-help-visits ()
  ;; `source-directory' is each process's own (its ROMACS_SOURCE_DIRECTORY,
  ;; else its runtime tree's `source' link, else the build's checkout) and
  ;; an init may set it: help reads it at each visit to a built-in's Rust,
  ;; so the value in effect is the one it uses.  Without a tree, the visit
  ;; is a `user-error' naming the file.
  (let* ((site (romacs-help--rust-definition 'car 'defun))
         (file (car site))
         (tree (file-name-as-directory (make-temp-file "romacs-source-" t)))
         (path (expand-file-name file tree)))
    (unwind-protect
        (progn
          (make-directory (file-name-directory path) t)
          (with-temp-file path (insert "// before\n" (nth 2 site) "\n"))
          (let* ((source-directory tree)
                 (found (romacs-help--rust-source 'car nil file)))
            (unwind-protect
                (with-current-buffer (car found)
                  (should (file-equal-p buffer-file-name path))
                  (goto-char (cdr found))
                  (should (search-forward (nth 2 site) (line-end-position) t)))
              (kill-buffer (car found))))
          (let ((source-directory nil))
            (should (string-search file (cadr (should-error (romacs-help--rust-source 'car nil file)
                                                            :type 'user-error))))))
      (delete-directory tree t))))

;;; romacs-source-directory-tests.el ends here
