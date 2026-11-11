;;; hash-eq.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe hash-eq :n 50000
  :setup nil
  :body (let ((table (make-hash-table :test #'eq)))
          (dotimes (i n) (puthash i i table))
          (dotimes (i n) (gethash i table))
          (dotimes (i n) (remhash i table))))
