(progn
;;; treesit.el --- VM2: the Lisp half of tree-sitter's primitives  -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; Evaluated once per VM by prims/treesit.rs, the first time a primitive
;; needs one of these: the calls GNU's treesit.c makes into Lisp that need
;; a handler or a buffer of their own.

;;; Code:

(defun romacs--treesit-notify (functions ranges parser)
  "Call each of FUNCTIONS with RANGES and PARSER after PARSER reparsed.
As treesit.c's safe_calln: a signal from one is dropped and the next runs."
  (while functions
    (condition-case nil
        (funcall (car functions) ranges parser)
      (t nil))
    (setq functions (cdr functions))))

(defun romacs--treesit-parse-string (string language)
  "treesit.c's treesit-parse-string: STRING parsed for LANGUAGE.
The parser lives in a new buffer that does not list it, and the root node
is returned."
  (let ((buffer (get-buffer-create
                 (generate-new-buffer-name " *treesit-parse-string*"))))
    (save-current-buffer
      (set-buffer buffer)
      (insert string))
    (let ((parser (treesit-parser-create language buffer t)))
      (romacs--treesit-detach parser)
      (treesit-parser-root-node parser))))
)
