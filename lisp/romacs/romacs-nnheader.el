;;; romacs-nnheader.el --- Gnus finds its shipped data in data-directory  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; GNU's `nnheader-find-etc-directory' (lisp/gnus/nnheader.el) guesses where
;; etc/ is: beside each `load-path' directory.  That finds GNU's own etc/
;; only where the Lisp directory and etc/ share a parent.  Here they do not,
;; and the shipped etc/ is known: it is `data-directory'.  So the function
;; walks a `load-path' whose entry for GNU's Lisp directory is replaced by
;; one beside `data-directory'; everything else is GNU's.  VM2-ETC1.

;;; Code:

(require 'nnheader)

(defun romacs-nnheader--find-etc (find &rest arguments)
  "Around FIND, `nnheader-find-etc-directory', called with ARGUMENTS.
GNU's Lisp directory stands in `load-path' as a sibling of `data-directory',
so the shipped data is found at that directory's place in the search."
  (let ((load-path (mapcar (lambda (directory)
                             (if (and directory
                                      (equal (file-name-as-directory directory) lisp-directory))
                                 (expand-file-name "../lisp" data-directory)
                               directory))
                           load-path)))
    (apply find arguments)))

(advice-add 'nnheader-find-etc-directory :around #'romacs-nnheader--find-etc)

(provide 'romacs-nnheader)
;;; romacs-nnheader.el ends here
