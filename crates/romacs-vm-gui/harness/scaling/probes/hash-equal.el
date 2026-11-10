;;; hash-equal.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe hash-equal :n 20000
  :setup (scaling--names "scaling-key" n)
  :body (let ((table (make-hash-table :test #'equal)))
          (dolist (key it) (puthash key t table))
          (dolist (key it) (gethash key table))
          (dolist (key it) (remhash key table))))
