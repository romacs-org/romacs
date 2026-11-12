;;; romacs-follow.el --- Romacs definitions that follow a GNU file's loads -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Some files of lisp/romacs/ define again a few of a GNU file's
;; definitions, for Romacs's engine (ledger/divergences/LISP-*.md).  A
;; later load of the GNU file restores GNU's definitions; `require' of
;; the Romacs file would not undo that, as its feature stays provided.
;; `romacs-follow-gnu-file' loads the Romacs file after every load of
;; the GNU file instead.

;;; Code:

(defun romacs-follow-gnu-file (library romacs-file)
  "Load ROMACS-FILE after LIBRARY: now, if it is loaded, and after every
later load of it.  LIBRARY is the feature a GNU file provides, so the
follow holds whatever name the file was loaded by (a renamed copy, a
symlink); for a GNU file that provides none, such as subr.el, it is the
file's name as `eval-after-load' takes one, and only that name follows."
  (eval-after-load library (lambda () (load romacs-file nil t))))

(provide 'romacs-follow)
;;; romacs-follow.el ends here
