;;; gc-free-lists.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe gc-free-lists :n 200000
  ;; Rounds of: garbage in every table, a cons, closure, cell and string
  ;; kept above it, a collection, then as much garbage again.  The sweep leaves free
  ;; entries in proportion to N below the kept ones and the allocation
  ;; takes them all back, so the work is linear in N; an allocation that
  ;; scanned a free list, a sweep that listed an entry twice or a table
  ;; never cut back would make it grow faster.
  :setup nil
  :body (dotimes (_ 4)
          (scaling--churn n)
          (setq scaling--keeper (let ((y 0)) (list n (lambda () (setq y (1+ y))) (make-string 16 ?y))))
          (garbage-collect)
          (scaling--churn n)))
