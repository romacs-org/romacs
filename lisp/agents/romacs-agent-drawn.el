;;; romacs-agent-drawn.el --- A chat's finished turns, kept as drawn  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Each finished turn of a chat is kept as the chat drew it: its text and
;; its text properties, as data, in the `turn_drawn' table of the agents'
;; sessions.sqlite, beside its turn row (romacs-agent-turns.el).  A chat
;; opens by inserting these forms; no renderer runs for a finished turn.
;;
;; A form is printed Lisp data, (TEXT PROPS INTERVALS): TEXT without
;; properties, PROPS a vector of the distinct property lists in it, and
;; INTERVALS a vector of START END INDEX, flattened.  A property's value
;; is data only (symbols, strings, numbers, lists of them): a face is its
;; name, so a theme applies when it is shown, and a keymap is a symbol
;; whose function cell holds the map (`romacs-agent-drawn-keymaps').
;;
;; Each form keeps the drawing version it was made by
;; (`romacs-agent-drawn-version'): an older one is shown, and drawn again
;; by a worker.  Forms are made where turn rows are: by the workers that
;; write them, and by the editor for a turn it drew live.

;;; Code:

(require 'cl-lib)

(defconst romacs-agent-drawn-version 2
  "The version of what a finished turn draws: raised with any change to
it (a renderer, the chat's drawing, a face's name), so older forms are
drawn again.")

(defconst romacs-agent-drawn--schema
  "create table if not exists turn_drawn (
     session text not null, first_seq integer not null, version integer not null,
     form text not null, primary key (session, first_seq)) without rowid"
  "The table of the drawn forms.")

(defvar romacs-agent-drawn-keymaps nil
  "The keymaps a drawn form may name: symbols whose value is a keymap a
chat puts in its text, each also its own function definition, so the
text finds its map through the symbol (`romacs-agent-drawn-keymap').")

(defun romacs-agent-drawn-keymap (symbol)
  "Let drawn text name SYMBOL's keymap, its value: SYMBOL, which the text
holds."
  (fset symbol (symbol-value symbol))
  (cl-pushnew symbol romacs-agent-drawn-keymaps)
  symbol)

;;; Forms

(defun romacs-agent-drawn--data-p (value)
  "Whether VALUE is data: what reads back as it printed."
  (cond ((or (null value) (symbolp value) (stringp value) (numberp value)) t)
        ((consp value) (and (romacs-agent-drawn--data-p (car value)) (romacs-agent-drawn--data-p (cdr value))))
        ((vectorp value) (cl-every #'romacs-agent-drawn--data-p value))))

(defun romacs-agent-drawn--value (property value)
  "PROPERTY's VALUE as a form keeps it, or `none' when it keeps none:
a keymap by its symbol, a call by its tool's name, a string without its
own properties, any other data as it is."
  (cond ((eq property 'keymap)
         (cond ((and (symbolp value) (memq value romacs-agent-drawn-keymaps)) value)
               ((cl-find-if (lambda (symbol) (eq (symbol-value symbol) value)) romacs-agent-drawn-keymaps))
               (t 'none)))
        ;; The call's input is its turn row's, read when its body is opened.
        ((eq property 'romacs-agent-call) (if (consp value) (list (car value)) 'none))
        ;; A run's row: that it is one; what it counted lives while it is open.
        ((eq property 'romacs-agent-run-head) (and value t))
        ((stringp value) (substring-no-properties value))
        ((romacs-agent-drawn--data-p value) value)
        (t 'none)))

(defun romacs-agent-drawn-form (start end)
  "The current buffer's text from START to END as a drawn form, printed."
  (let ((table (make-hash-table :test #'equal)) (props nil) (flat nil) (at start))
    (while (< at end)
      (let ((next (next-property-change at nil end)) (plist nil))
        (cl-loop for (property value) on (text-properties-at at) by #'cddr
                 do (let ((kept (romacs-agent-drawn--value property value)))
                      (unless (eq kept 'none) (setq plist (cons kept (cons property plist))))))
        (when plist
          (setq plist (nreverse plist))
          (let ((index (or (gethash plist table)
                           (prog1 (puthash plist (hash-table-count table) table) (push plist props)))))
            (push (- at start) flat) (push (- next start) flat) (push index flat)))
        (setq at next)))
    (let ((print-escape-newlines t) (print-length nil) (print-level nil))
      (prin1-to-string (list (buffer-substring-no-properties start end) (vconcat (nreverse props))
                             (vconcat (nreverse flat)))))))

(defun romacs-agent-drawn--shared (props)
  "PROPS, a form's property lists, each string value one object across
them as it was when drawn: a run, a call, a turn is told by its string
being the same (`eq'), as text property changes see it."
  (let ((strings (make-hash-table :test #'equal)))
    (mapcar (lambda (plist)
              (cl-loop for (property value) on plist by #'cddr
                       append (list property (if (stringp value)
                                                 (or (gethash value strings) (puthash value value strings))
                                               value))))
            props)))

(defun romacs-agent-drawn-insert (form &optional before-markers)
  "Insert FORM (`romacs-agent-drawn-form''s, printed) at point, with its
properties, before the markers there with BEFORE-MARKERS; point after it."
  (pcase-let ((`(,text ,props ,flat) (car (read-from-string form))) (start (point)) (i 0))
    (setq props (vconcat (romacs-agent-drawn--shared (append props nil))))
    (if before-markers (insert-before-markers text) (insert text))
    (while (< i (length flat))
      (set-text-properties (+ start (aref flat i)) (+ start (aref flat (1+ i))) (aref props (aref flat (+ i 2))))
      (setq i (+ i 3)))))

;;; The table

(defvar romacs-agent-drawn--made nil "The databases the table was made in.")

(defun romacs-agent-drawn--table (db)
  "DB, its table made once."
  (unless (memq db romacs-agent-drawn--made)
    (sqlite-execute db romacs-agent-drawn--schema)
    (push db romacs-agent-drawn--made))
  db)

(defun romacs-agent-drawn-put (db session first-seq form)
  "Keep FORM as the drawn turn of SESSION (an id) from FIRST-SEQ in DB, at
this version: no transaction of its own, so it may be in its writer's."
  (sqlite-execute (romacs-agent-drawn--table db)
                  "insert or replace into turn_drawn values (?, ?, ?, ?)"
                  (list session first-seq romacs-agent-drawn-version form)))

(defun romacs-agent-drawn-get (db session first-seqs)
  "SESSION's drawn forms in DB of its turns from FIRST-SEQS: a table from
first seq to (VERSION . FORM)."
  (let ((found (make-hash-table)))
    (when first-seqs
      (dolist (row (sqlite-select (romacs-agent-drawn--table db)
                                  (format "select first_seq, version, form from turn_drawn where session = ? and first_seq in (%s)"
                                          (mapconcat (lambda (_) "?") first-seqs ","))
                                  (cons session first-seqs)))
        (puthash (nth 0 row) (cons (nth 1 row) (nth 2 row)) found)))
    found))

(defun romacs-agent-drawn-stale (db &optional limit)
  "The turns in DB drawn by an older version: (SESSION FIRST-SEQ)
lists, at most LIMIT."
  (sqlite-select (romacs-agent-drawn--table db)
                 (concat "select session, first_seq from turn_drawn where version < ?"
                         (if limit " limit ?" ""))
                 (append (list romacs-agent-drawn-version) (and limit (list limit)))))

(provide 'romacs-agent-drawn)
;;; romacs-agent-drawn.el ends here
