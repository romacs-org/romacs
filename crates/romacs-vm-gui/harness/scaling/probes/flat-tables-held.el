;;; flat-tables-held.el --- scaling probe  -*- lexical-binding: t -*-

;; N syntax tables, each read under in turn, a case-insensitive compare
;; after each, twice round, one of them written between the rounds.  Then
;; every table read once more, and N times one table written and read
;; under at once: each write drops the views, and dropping them costs the
;; few views kept, not the N tables ever read.  A table's view (its ASCII
;; row, and a page for the Greek) is made by the first read under it, so
;; a read costs its own table's rows whatever the number of tables.
(scaling-probe flat-tables-held :n 1000
  :setup (with-current-buffer (get-buffer-create " scaling-flat-tables")
           (erase-buffer)
           (insert "word αβγ other_words")
           (let (tables)
             (dotimes (_ n) (push (make-syntax-table) tables))
             (cons (current-buffer) tables)))
  :body (with-current-buffer (car it)
          (dotimes (_ 2)
            (dolist (table (cdr it))
              (with-syntax-table table
                (goto-char (point-min))
                (skip-syntax-forward "w")
                (skip-syntax-forward "^w")
                (skip-syntax-forward "w"))
              (compare-strings "Flat Table" nil nil "fLAT tABLE" nil nil t))
            (modify-syntax-entry ?_ "w" (cadr it)))
          (dolist (table (cdr it))
            (with-syntax-table table (char-syntax ?α)))
          (dolist (table (cdr it))
            (modify-syntax-entry ?x "w" table)
            (with-syntax-table table (char-syntax ?α)))))
