;;; romacs-fzf.el --- The fzf completion style, matched natively  -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; The `fzf' completion style: fzf's extended search syntax and its v2
;; scoring, matched in Rust (src/fzf.rs, a port of fzf commit 26ff936d
;; under fzf's MIT license).  It is opt-in:
;;
;;   (setq completion-styles '(fzf basic))
;;
;; A query is terms separated by spaces, all of which must match.  A
;; term matches fuzzily (its characters in order); 'term matches
;; exactly, 'term' exactly at word boundaries, ^term at the start,
;; term$ at the end and ^term$ the whole candidate; !term must not
;; match (exactly; !'term not fuzzily), and a | b matches when either
;; does.  A backslash before a space puts the space in the term.
;;
;; With `romacs-fzf-smart-case' a term with an upper-case letter
;; respects case and any other ignores it; without it,
;; `completion-ignore-case' decides.  Case folds through the buffer's
;; case table.  Matches sort best first: by fzf's score, then shorter,
;; then in the table's order, through the metadata's sort functions as
;; GNU's `flex' sorts (`completion--adjust-metadata').  Highlighting
;; follows `completion-lazy-hilit'.  DIVERGENCES.md VM2-FZF.

;;; Code:

(require 'romacs-completion-candidates)

(defgroup romacs-fzf nil
  "The fzf completion style."
  :group 'minibuffer)

(defcustom romacs-fzf-smart-case t
  "Whether a term's own case decides whether it ignores case.
Non-nil: a term with an upper-case letter respects case and any other
ignores it.  Nil: `completion-ignore-case' decides for every term."
  :type 'boolean)

(defcustom romacs-fzf-parallel-threshold 2000
  "Candidate count from which scoring is split across threads.
Nil always scores on one thread.  The order is the same either way."
  :type '(choice (const :tag "Never" nil) natnum))

(defface romacs-fzf-match '((t :inherit completions-common-part))
  "Face for the characters an fzf query matches.")

(defvar romacs-fzf--last nil
  "(QUERY CASE TABLE) of the last `romacs-fzf-all-completions'.
`romacs-fzf--adjust-metadata' ranks by it.")

(defun romacs-fzf--case ()
  "How the fzf primitives fold case: `smart', t (ignore it) or nil."
  (cond (romacs-fzf-smart-case 'smart) (completion-ignore-case t)))

(defun romacs-fzf--split (string table pred)
  "STRING as (PREFIX . QUERY), PREFIX what precedes TABLE's boundary."
  (let ((limit (car (completion-boundaries string table pred ""))))
    (cons (substring string 0 limit) (substring string limit))))

(defun romacs-fzf--query (query table)
  "QUERY as TABLE's entries match it.
A file name table's entries match by name, unquoted, so the query is
too: a doubled `$' is one, a single one being the boundary of an
environment variable's name."
  (if (eq table 'read-file-name-internal) (string-replace "$$" "$" query) query))

(defun romacs-fzf--filter (prefix query case table pred)
  "TABLE's candidates for PREFIX and PRED that QUERY matches, best first.
QUERY is as `romacs-fzf--query' gives it, CASE as `romacs-fzf--case'."
  (let* ((completion-ignore-case (if (eq case 'smart) (string= query (downcase query)) case))
         (candidates (romacs-completion-list prefix table pred)))
    (romacs--fzf-filter (car candidates) query case romacs-fzf-parallel-threshold
                        (cdr candidates))))

(defun romacs-fzf--highlight (strings query case table copy)
  "STRINGS with what QUERY matches in their TABLE names highlighted."
  (romacs--fzf-highlight strings query case 'romacs-fzf-match copy
                         (romacs-completion-names strings table)))

;;;###autoload
(defun romacs-fzf-all-completions (string table pred _point)
  "TABLE's entries satisfying PRED that the fzf query in STRING matches.
They come best first, with a nonzero base size in the last cdr; the matched
characters are highlighted, lazily where `completion-lazy-hilit-p'.
This function is part of the `fzf' completion style."
  (pcase-let* ((`(,prefix . ,query) (romacs-fzf--split string table pred))
               (query (romacs-fzf--query query table))
               (case (romacs-fzf--case))
               (completions (romacs-fzf--filter prefix query case table pred)))
    (setq romacs-fzf--last (list query case table))
    (when completions
      (if (completion-lazy-hilit-p)
          (setq completion-lazy-hilit-fn
                (lambda (str) (car (romacs-fzf--highlight (list str) query case table nil))))
        (setq completions (romacs-fzf--highlight completions query case table t)))
      ;; As `flex': no base size where nothing precedes the boundary.
      (if (string= prefix "") completions (nconc completions (length prefix))))))

;;;###autoload
(defun romacs-fzf-try-completion (string table pred point)
  "Complete STRING to the one entry of TABLE its fzf query matches.
STRING's prefix completion comes first, as `orderless' expands by
default; otherwise one match completes to it, several leave STRING
and POINT as they are, and none is nil.
This function is part of the `fzf' completion style."
  (or (completion-emacs21-try-completion string table pred point)
      (pcase-let* ((`(,prefix . ,query) (romacs-fzf--split string table pred))
                   (matches (romacs-fzf--filter prefix (romacs-fzf--query query table)
                                                (romacs-fzf--case) table pred))
                   (one (car matches)))
        (cond ((null matches) nil)
              ((seq-some (lambda (m) (not (equal m one))) (cdr matches)) (cons string point))
              (t (setq one (concat prefix one))
                 (or (equal string one) (cons one (length one))))))))

(defun romacs-fzf--adjust-metadata (metadata)
  "METADATA ranking by the last fzf query, where it filtered.
Its sort functions apply first and fzf's rank then, as GNU's
`completion--flex-adjust-metadata' composes flex's; without one the
completions already come ranked."
  (pcase-let ((`(,query ,case ,table) romacs-fzf--last))
    (if (string-blank-p (or query "")) metadata
      (let ((rank (lambda (existing)
                    (if (null existing) #'identity
                      (lambda (completions)
                        (let ((completions (funcall existing completions)))
                          (romacs--fzf-sort completions query case romacs-fzf-parallel-threshold
                                            (romacs-completion-names completions table))))))))
        `(metadata
          (display-sort-function
           . ,(funcall rank (completion-metadata-get metadata 'display-sort-function)))
          (cycle-sort-function
           . ,(funcall rank (completion-metadata-get metadata 'cycle-sort-function)))
          ,@(cdr metadata))))))

(put 'fzf 'completion--adjust-metadata #'romacs-fzf--adjust-metadata)

;;;###autoload
(add-to-list 'completion-styles-alist
             '(fzf romacs-fzf-try-completion romacs-fzf-all-completions
               "fzf's extended search: terms in any order, best match first."))

(provide 'romacs-fzf)
;;; romacs-fzf.el ends here
