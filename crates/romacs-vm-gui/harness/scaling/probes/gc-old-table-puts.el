;;; gc-old-table-puts.el --- scaling probe  -*- lexical-binding: t -*-

;; Puts into a strong hash table that survived a collection (table.rs
;; `Barrier'): each logs its key and value for the next collection, which
;; empties the log.  N puts of fresh conses, then that collection: a put
;; costs the same however many the log already holds.
(scaling-probe gc-old-table-puts :n 20000
  :setup (let ((table (make-hash-table :test #'eq))) (garbage-collect) table)
  :body (progn (dotimes (i n) (puthash i (list i) it))
               (garbage-collect)))
