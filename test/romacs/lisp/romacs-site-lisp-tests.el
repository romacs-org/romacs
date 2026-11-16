;;; romacs-site-lisp-tests.el --- The system's site-lisp on load-path -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-site-lisp)

(ert-deftest romacs-site-lisp-tests-present-directories-lead-with-their-subdirs ()
  (let* ((site (make-temp-file "romacs-site-lisp-" t))
         (missing (expand-file-name "missing" site))
         (load-path (list "/elsewhere/lisp" site)))
    (unwind-protect
        (progn
          (make-directory (expand-file-name "notmuch" site))
          (with-temp-file (expand-file-name "subdirs.el" site)
            (insert "(normal-top-level-add-subdirs-to-load-path)\n"))
          (romacs-site-lisp-start (list site missing))
          ;; The site directory first, once; its subdirectory after it;
          ;; a missing one nowhere; what was there kept, after them.
          (should (equal load-path (list site (expand-file-name "notmuch" site) "/elsewhere/lisp"))))
      (delete-directory site t))))

;;; romacs-site-lisp-tests.el ends here
