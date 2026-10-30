;;; romacs-cl-generic.el --- GNU generic functions for Romacs -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; Load after the GNU core prerequisites.  The representation bridge must be
;; installed before GNU builds dispatchers that inspect ordinary closures.

;;; Code:

(require 'romacs-oclosure)
(require 'cl-generic)

;; cl-lib.el defines `cl-deftype' dispatch, its "derived-types"
;; `cl-generic-generalizers' method, only when cl-generic is already
;; loaded.  GNU dumps cl-generic and loads cl-lib.elc later, so the method
;; exists once cl-lib does.  Loading cl-generic.el from source requires
;; cl-lib first (its `eval-when-compile'), so that guard was false here:
;; evaluate cl-lib.el's own form now that cl-generic is defined.
(when (featurep 'cl-lib)
  (with-temp-buffer
    (insert-file-contents (locate-file-internal "emacs-lisp/cl-lib.el" load-path))
    (goto-char (point-min))
    (search-forward "\n(when (fboundp 'cl-generic-define-method)")
    (goto-char (1+ (match-beginning 0)))
    (eval (read (current-buffer)) t)))

(provide 'romacs-cl-generic)
;;; romacs-cl-generic.el ends here
