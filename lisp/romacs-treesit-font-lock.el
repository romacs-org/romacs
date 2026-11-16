;;; romacs-treesit-font-lock.el --- treesit.el's font-lock, written for Romacs -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Loaded after every load of GNU's treesit.el (`romacs-follow-gnu-file'),
;; whose `treesit-font-lock-fontify-region' and
;; `treesit--font-lock-fontify-region-1' this replaces
;; (ledger/divergences/LISP-TREESIT-FONTIFY.md).
;;
;; GNU's runs each font-lock setting's query over the region in turn, asks
;; `treesit-query-capture' for a list of its captures, and walks that list
;; in Lisp: a face per capture, through `treesit-fontify-with-override', or
;; a call to the capture's function.  Here the enabled settings of a
;; language, while no setting of another language comes between them, are
;; one query, made once and kept until the settings change; the region is
;; walked once for all of them, and `romacs--treesit-fontify' applies the
;; captures setting by setting, in GNU's order, with no list made and no
;; Lisp loop.

;;; Code:

(defvar treesit--font-lock-query-expand-range)
(defvar treesit--font-lock-fast-mode)
(defvar treesit--font-lock-verbose)
(defvar jit-lock-chunk-size)
(declare-function romacs--treesit-fontify "treesit.rs")
(declare-function romacs--treesit-query-combine "treesit.rs")
(declare-function romacs--treesit-subtree-exceeds "treesit.rs")
(declare-function treesit-update-ranges "treesit")
(declare-function treesit-local-parsers-on "treesit")
(declare-function treesit--children-covering-range-recurse "treesit")
(declare-function font-lock-unfontify-region "font-lock")

(defvar-local romacs-treesit--font-lock-runs nil
  "This buffer's enabled font-lock settings as combined queries.
\(SETTINGS FLAGS . RUNS): `treesit-font-lock-settings' and each setting's
enable flag when they were made, and for each run of consecutive enabled
settings of one language, (LANGUAGE QUERY . SPANS) as
`romacs--treesit-query-combine' gives QUERY and SPANS.")

(defun romacs-treesit--font-lock-runs ()
  "The runs of `romacs-treesit--font-lock-runs', made again when the
settings or their enable flags changed."
  (let ((settings treesit-font-lock-settings)
        (flags (mapcar #'treesit-font-lock-setting-enable treesit-font-lock-settings)))
    (unless (and (eq (car romacs-treesit--font-lock-runs) settings)
                 (equal (cadr romacs-treesit--font-lock-runs) flags))
      (let (runs)
        (dolist (setting settings)
          (when (eq t (treesit-font-lock-setting-enable setting))
            (let ((language (treesit-font-lock-setting-language setting)))
              (unless (eq language (caar runs))
                (push (list language) runs))
              (push setting (cdar runs)))))
        (setq romacs-treesit--font-lock-runs
              (cons settings
                    (cons flags
                          (mapcar (lambda (run)
                                    (let ((members (reverse (cdr run))))
                                      (cons (car run)
                                            (romacs--treesit-query-combine
                                             (car run)
                                             (mapcar #'treesit-font-lock-setting-query members)
                                             (mapcar #'treesit-font-lock-setting-override members)))))
                                  (nreverse runs)))))))
    (cddr romacs-treesit--font-lock-runs)))

(defun treesit-font-lock-fontify-region (start end &optional loudly)
  "Fontify the region between START and END.
If LOUDLY is non-nil, display some debugging information."
  (when (or loudly treesit--font-lock-verbose)
    (message "Fontifying region: %s-%s" start end))
  (treesit-update-ranges start end)
  (font-lock-unfontify-region start end)
  (let* ((root-nodes (mapcar #'treesit-parser-root-node
                             (append (treesit-local-parsers-on start end) (treesit-parser-list))))
         (expand treesit--font-lock-query-expand-range)
         (query-start (max (- start (car expand)) (point-min)))
         (query-end (min (+ end (cdr expand)) (point-max)))
         (region (cons start end)))
    ;; GNU's deterministic choice of "fast mode" (bug#60691, bug#60223),
    ;; its tree walk stopped as soon as it decides.
    (when (and treesit-font-lock-settings (eq treesit--font-lock-fast-mode 'unspecified))
      (setq treesit--font-lock-fast-mode
            (romacs--treesit-subtree-exceeds (treesit-parser-root-node treesit-primary-parser) 100 4000)))
    (with-silent-modifications
      (pcase-dolist (`(,language . ,combined) (romacs-treesit--font-lock-runs))
        (dolist (root root-nodes)
          (when (eq (treesit-node-language root) language)
            (dolist (node (if (eq t treesit--font-lock-fast-mode)
                              (treesit--children-covering-range-recurse
                               root start end (* 4 jit-lock-chunk-size))
                            (list root)))
              (romacs--treesit-fontify node combined query-start query-end region)))))))
  `(jit-lock-bounds ,start . ,end))

(defun treesit--font-lock-fontify-region-1 (node query start end override _loudly)
  "Fontify the region between START and END by querying NODE with QUERY.
Each capture named by a face gives that face to its node's part of the
region, as OVERRIDE says (see `treesit-font-lock-rules'); one named by a
function calls it with the node, OVERRIDE, START and END.  The query runs
over the region widened by `treesit--font-lock-query-expand-range'.
GNU's prints a message per capture when LOUDLY; this prints none."
  (let ((expand treesit--font-lock-query-expand-range))
    (with-silent-modifications
      (romacs--treesit-fontify node (cons query (vector (cons 0 override)))
                               (max (- start (car expand)) (point-min))
                               (min (+ end (cdr expand)) (point-max))
                               (cons start end)))))

(provide 'romacs-treesit-font-lock)
;;; romacs-treesit-font-lock.el ends here
