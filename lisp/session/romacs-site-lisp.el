;;; romacs-site-lisp.el --- The system's site-lisp on load-path, as GNU's  -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; GNU Emacs puts its installation's site-lisp directories before its
;; Lisp on `load-path' (lread.c load_path_default, PATH_SITELOADSEARCH),
;; unless -Q or --no-site-lisp, and its startup loads each directory's
;; subdirs.el (startup.el normal-top-level), so what a distribution
;; installs there, such as notmuch, loads by name.  Romacs has no
;; installation prefix of its own: its site-lisp directories are the
;; system's, where a distribution installs Lisp for GNU Emacs.  The
;; versioned ones (share/emacs/VERSION/site-lisp) belong to that
;; version of GNU Emacs, and are left out.

;;; Code:

(defconst romacs-site-lisp-directories
  '("/usr/local/share/emacs/site-lisp" "/usr/share/emacs/site-lisp")
  "The site-lisp directories a session puts on `load-path', first first.")

(defun romacs-site-lisp-start (&optional directories)
  "Put DIRECTORIES that exist before `load-path', with their subdirs.el.
DIRECTORIES defaults to `romacs-site-lisp-directories'.  Each
directory's subdirs.el adds its subdirectories after it, with
`default-directory' bound to the directory, as GNU's startup loads one."
  (let ((present (seq-filter #'file-directory-p
                             (or directories romacs-site-lisp-directories))))
    (setq load-path (append present (seq-difference load-path present)))
    (dolist (directory present)
      (let ((default-directory (file-name-as-directory directory)))
        (load (expand-file-name "subdirs.el" directory) t t t)))))

(provide 'romacs-site-lisp)
;;; romacs-site-lisp.el ends here
