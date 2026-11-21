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
  "Help's candidates in this minibuffer, as (KEY NAMES), KEY as
`romacs-help--key' makes it and NAMES a hash table of their names.
Local to the minibuffer, so each prompt lists them anew.")

(defun romacs-help--names (pred)
  "A hash table of the names of the symbols PRED accepts."
  (let ((names (make-hash-table :test 'equal :size 100000)))
    (mapatoms (lambda (symbol)
                (when (or (null pred) (funcall pred symbol))
                  (puthash (symbol-name symbol) t names))))
    names))

(defun romacs-help--key (pred)
  "What decides help's candidates for PRED within a prompt: a function
defined or a file loaded meanwhile lists them again."
  (list pred (romacs-command-epoch) (length load-history)))

(defun romacs-help--list (pred)
  "Help's candidates for PRED, kept in a minibuffer while the prompt lasts
\(`romacs-help--candidates')."
  (if (not (minibufferp)) (romacs-help--names pred)
    (let ((key (romacs-help--key pred)))
      (unless (equal key (car romacs-help--candidates))
        (setq romacs-help--candidates (list key (romacs-help--names pred))))
      (cadr romacs-help--candidates))))

(defun romacs-help--exact-p (string pred)
  "Whether STRING names a symbol PRED accepts now, and matches
`completion-regexp-list'."
  (and (let ((case-fold-search completion-ignore-case))
         (seq-every-p (lambda (regexp) (string-match-p regexp string)) completion-regexp-list))
       (let ((symbol (intern-soft string))) (and symbol (or (null pred) (funcall pred symbol))))
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
   (t (complete-with-action action (romacs-help--list pred) string nil))))

(provide 'romacs-help-completion)
;;; romacs-help-completion.el ends here
