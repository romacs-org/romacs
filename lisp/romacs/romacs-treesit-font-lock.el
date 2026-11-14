;;; romacs-treesit-font-lock.el --- treesit.el's font-lock, written for Romacs -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Loaded after every load of GNU's treesit.el (`romacs-follow-gnu-file'),
;; whose `treesit--font-lock-fontify-region-1' this replaces
;; (ledger/divergences/LISP-TREESIT-FONTIFY.md).
;;
;; GNU's asks `treesit-query-capture' for a list of every capture in the
;; region, then walks it in Lisp: a face per capture, through
;; `treesit-fontify-with-override', or a call to the capture's function.
;; `romacs--treesit-fontify' runs the same captures and predicates and
;; does the same, in the same order, with no list made and no Lisp loop.

;;; Code:

(defvar treesit--font-lock-query-expand-range)
(declare-function romacs--treesit-fontify "treesit.rs")

(defun treesit--font-lock-fontify-region-1 (node query start end override _loudly)
  "Fontify the region between START and END by querying NODE with QUERY.
Each capture named by a face gives that face to its node's part of the
region, as OVERRIDE says (see `treesit-font-lock-rules'); one named by a
function calls it with the node, OVERRIDE, START and END.  The query runs
over the region widened by `treesit--font-lock-query-expand-range'.
GNU's prints a message per capture when LOUDLY; this prints none."
  (let ((expand treesit--font-lock-query-expand-range))
    (with-silent-modifications
      (romacs--treesit-fontify node query
                               (max (- start (car expand)) (point-min))
                               (min (+ end (cdr expand)) (point-max))
                               (cons start end) override))))

(provide 'romacs-treesit-font-lock)
;;; romacs-treesit-font-lock.el ends here
