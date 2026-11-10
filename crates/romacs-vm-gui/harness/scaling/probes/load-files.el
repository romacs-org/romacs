;;; load-files.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe load-files :n 50
  :setup (mapcar (lambda (i) (scaling--file 1 (lambda (_) `(provide ',(intern (format "scaling-file-%d" i))))))
                 (number-sequence 1 n))
  :body (dolist (file it) (load file nil t)))
