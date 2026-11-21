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
;; A definition in a library not loaded yet comes from the index a worker
;; makes (romacs-help-index.el), once it has; the library of the name a
;; help prompt returns loads then, before help describes it
;; (ledger/divergences/VM2-HELP-NO-LOAD.md).

;;; Code:

(require 'romacs-help-index)

(declare-function romacs-command-epoch "prims/completion.rs" ())
(declare-function help--symbol-completion-table-affixation "help-fns" (completions))

(defvar-local romacs-help--candidates nil
  "Help's candidates in this minibuffer, as (KEY HISTORY NAMES FOLDED),
KEY as `romacs-help--key' makes it, HISTORY the head of `load-history'
then, NAMES a hash table of their names and FOLDED, once asked for, one
from each name's downcased form to the names that have it.  Local to the
minibuffer, so each prompt lists them anew.")

(defun romacs-help--kinds (pred)
  "The kinds of indexed definition PRED accepts, asked of a function, a
command, a variable and a face that are nothing else."
  (if (null pred) '(function command variable face)
    (append (and (funcall pred 'car) '(function))
            (and (funcall pred 'ignore) '(command))
            (and (funcall pred 'load-path) '(variable))
            (and (funcall pred 'default) '(face)))))

(defun romacs-help--indexed-p (entry kinds)
  "Whether ENTRY, an index entry, defines a kind of KINDS."
  (seq-some (lambda (kind) (assq kind entry)) kinds))

(defun romacs-help--names (pred)
  "A hash table of the names of the symbols PRED accepts, and of the
indexed definitions of the kinds it accepts.  It asks a worker for the
index of definitions not loaded yet (`romacs-help-index-request')."
  (romacs-help-index-request)
  (let ((names (make-hash-table :test 'equal :size 100000)) (kinds (romacs-help--kinds pred)))
    (mapatoms (lambda (symbol)
                (when (or (null pred) (funcall pred symbol))
                  (puthash (symbol-name symbol) t names))))
    (when (and romacs-help-index kinds)
      (maphash (lambda (name entry)
                 (when (romacs-help--indexed-p entry kinds) (puthash name t names)))
               romacs-help-index))
    names))

(defun romacs-help--key (pred)
  "What decides help's candidates for PRED within a prompt, besides the
files loaded: a function defined or the index arriving lists them again."
  (list pred (romacs-command-epoch) romacs-help-index))

(defun romacs-help--list (pred)
  "Help's candidates for PRED, kept in a minibuffer while the prompt lasts
and no function is defined or file loaded (`romacs-help--candidates')."
  (if (not (minibufferp)) (romacs-help--names pred)
    (let ((key (romacs-help--key pred)) (kept romacs-help--candidates))
      (unless (and (equal key (car kept)) (eq (car load-history) (nth 1 kept)))
        (setq romacs-help--candidates (list key (car load-history) (romacs-help--names pred) nil)))
      (nth 2 romacs-help--candidates))))

(defun romacs-help--folded (pred)
  "The names of help's candidates for PRED whose downcased form is a
key, as a hash table: kept with the prompt's candidates, made once."
  (let ((names (romacs-help--list pred)))
    (if (and (minibufferp) (nth 3 romacs-help--candidates))
        (nth 3 romacs-help--candidates)
      (let ((folded (make-hash-table :test 'equal :size (hash-table-count names))))
        (maphash (lambda (name _) (push name (gethash (downcase name) folded))) names)
        (when (minibufferp) (setcar (nthcdr 3 romacs-help--candidates) folded))
        folded))))

(defun romacs-help--accepts-p (name pred)
  "Whether NAME names a symbol PRED accepts now, or an indexed definition
of a kind it accepts."
  (or (let ((symbol (intern-soft name)))
        (and (or symbol (equal name "nil")) (or (null pred) (funcall pred symbol))))
      (let ((entry (and romacs-help-index (gethash name romacs-help-index))))
        (and entry (romacs-help--indexed-p entry (romacs-help--kinds pred)) t))))

(defun romacs-help--exact-p (string pred)
  "Whether STRING names a symbol PRED accepts now, and matches
`completion-regexp-list'.  Under `completion-ignore-case', a listed name
that differs from STRING in case alone answers for it."
  (and (let ((case-fold-search completion-ignore-case))
         (seq-every-p (lambda (regexp) (string-match-p regexp string)) completion-regexp-list))
       (or (romacs-help--accepts-p string pred)
           (and completion-ignore-case
                (seq-some (lambda (name)
                            (and (eq t (compare-strings name nil nil string nil nil t))
                                 (romacs-help--accepts-p name pred)))
                          (gethash (downcase string) (romacs-help--folded pred)))))
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

(defun romacs-help--load-chosen (name pred)
  "Load the library defining NAME as a kind PRED accepts, when only the
index knows it so: NAME as a symbol does not satisfy PRED yet."
  (when-let* ((entry (and romacs-help-index (gethash name romacs-help-index)))
              (symbol (intern name))
              ((not (and pred (funcall pred symbol))))
              (library (seq-some (lambda (kind) (alist-get kind entry)) (romacs-help--kinds pred))))
    (with-demoted-errors "while loading: %S"
      (load library 'noerror 'nomessage))))

(defun romacs-help--completing-read (read prompt collection &optional pred &rest args)
  "READ (`completing-read'), then, when COLLECTION is help's table, load
what defines the name it returned (`romacs-help--load-chosen'): after a
help prompt returns, not as it quits."
  (let ((value (apply read prompt collection pred args)))
    (when (and (eq collection #'help--symbol-completion-table) (stringp value))
      (romacs-help--load-chosen value pred))
    value))

(advice-add 'completing-read :around #'romacs-help--completing-read)

(provide 'romacs-help-completion)
;;; romacs-help-completion.el ends here
