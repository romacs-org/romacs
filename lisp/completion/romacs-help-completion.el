;;; romacs-help-completion.el --- Help's completion of symbols, loading no file  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; Copyright (C) 1985-1986, 1993-2026 Free Software Foundation, Inc.
;; Adapted from GNU help-fns.el: `help--symbol-completion-table', kept as
;; the GPL requires.

;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; GNU's `help--symbol-completion-table' (C-h f, C-h v, C-h o) loads, as
;; the input grows, every file whose `definition-prefixes' match it, and
;; lists the whole obarray under the prompt's predicate on every call:
;; `symbol-help' asks `basic', `shorthand' and `substring' first, so each
;; key ran the predicate over every symbol, about a second a key in a
;; session with a large obarray, and typing ran files.  Here the table
;; lists the names its predicate accepts once a prompt, until a function
;; is defined or a file loaded, and answers each key from that set; it
;; loads no file.  An exact name is asked of the predicate as it is now.
;; A definition in a library not loaded yet is not offered until the
;; library loads (ledger/divergences/VM2-HELP-NO-LOAD.md).

;;; Code:

(declare-function romacs-command-epoch "prims/completion.rs" ())
(declare-function help--symbol-completion-table-affixation "help-fns" (completions))

(defvar-local romacs-help--candidates nil
  "Help's candidates in this minibuffer, as (KEY HISTORY NAMES), KEY as
`romacs-help--key' makes it, HISTORY the head of `load-history' then and
NAMES a hash table of their names.  Local to the minibuffer, so each
prompt lists them anew.")

(defun romacs-help--names (pred)
  "A hash table of the names of the symbols PRED accepts."
  (let ((names (make-hash-table :test 'equal :size 100000)))
    (mapatoms (lambda (symbol)
                (when (or (null pred) (funcall pred symbol))
                  (puthash (symbol-name symbol) t names))))
    names))

(defun romacs-help--key (pred)
  "What decides help's candidates for PRED within a prompt, besides the
files loaded: a function defined meanwhile lists them again."
  (list pred (romacs-command-epoch)))

(defun romacs-help--list (pred)
  "Help's candidates for PRED, kept in a minibuffer while the prompt lasts
and no function is defined or file loaded (`romacs-help--candidates')."
  (if (not (minibufferp)) (romacs-help--names pred)
    (let ((key (romacs-help--key pred)) (kept romacs-help--candidates))
      (unless (and (equal key (car kept)) (eq (car load-history) (nth 1 kept)))
        (setq romacs-help--candidates (list key (car load-history) (romacs-help--names pred))))
      (nth 2 romacs-help--candidates))))

(defun romacs-help--accepts-p (name pred)
  "Whether NAME names a symbol PRED accepts now."
  (let ((symbol (intern-soft name)))
    (and (or symbol (equal name "nil")) (or (null pred) (funcall pred symbol)))))

(defun romacs-help--exact-p (string pred)
  "Whether STRING names a symbol PRED accepts now, and matches
`completion-regexp-list'.  Under `completion-ignore-case', a listed name
that differs from STRING in case alone answers for it."
  (and (let ((case-fold-search completion-ignore-case))
         (seq-every-p (lambda (regexp) (string-match-p regexp string)) completion-regexp-list))
       (or (romacs-help--accepts-p string pred)
           (and completion-ignore-case
                (seq-some (lambda (name)
                            (and (= (length name) (length string))
                                 (romacs-help--accepts-p name pred)))
                          (all-completions string (romacs-help--list pred)))))
       t))

(defun help--symbol-completion-table (string pred action)
  "Complete STRING to the names of the symbols PRED accepts, per ACTION.
The names are listed once a prompt (`romacs-help--list') and no file is
loaded; an exact name is asked of PRED as it is now."
  (cond
   ((eq action 'metadata)
    `(metadata
      ,@(when completions-detailed
          '((affixation-function . help--symbol-completion-table-affixation)))
      (category . symbol-help)))
   ((eq action 'lambda) (romacs-help--exact-p string pred))
   ((eq (car-safe action) 'boundaries) nil)
   (t (complete-with-action action (romacs-help--list pred) string nil))))

(provide 'romacs-help-completion)
;;; romacs-help-completion.el ends here
