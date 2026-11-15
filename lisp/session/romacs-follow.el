;;; romacs-follow.el --- Romacs definitions that follow a GNU file's loads -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Some of Romacs's own files define again a few of a GNU file's
;; definitions, for Romacs's engine (ledger/divergences/LISP-*.md).  A
;; later load of the GNU file restores GNU's definitions; `require' of
;; the Romacs file would not undo that, as its feature stays provided.
;; `romacs-follow-gnu-file' loads the Romacs file after every load of
;; the GNU file instead.

;;; Code:

(defconst romacs-follow--directory
  (expand-file-name "../" (file-name-directory (or load-file-name (locate-library "romacs-follow"))))
  "The Lisp tree, lisp/, the following files' names are relative to.")

(defun romacs-follow-gnu-file (library romacs-file)
  "Load ROMACS-FILE after LIBRARY: now, if it is loaded, and after every
later load of it.  LIBRARY is the feature a GNU file provides, so the
follow holds whatever name the file was loaded by (a renamed copy, a
symlink); for a GNU file that provides none, such as subr.el, it is the
file's name as `eval-after-load' takes one, and only that name follows.
ROMACS-FILE is a file of Romacs's own Lisp, named relative to lisp/
(\"international/romacs-mule\"), loaded from there whatever `load-path'
holds then: a load of the GNU file under a
`load-path' without that directory still gets Romacs's definition, and
cannot fail for want of it."
  (let ((file (expand-file-name romacs-file romacs-follow--directory)))
    (eval-after-load library (lambda () (load file nil t)))))

(provide 'romacs-follow)
;;; romacs-follow.el ends here
