;;; faces-supports-queries.el --- scaling probe  -*- lexical-binding: t -*-

;; `supports' over N distinct font queries: a query's fonts are listed once.
(scaling-probe faces-supports-queries :gui t :n 50 :steps 1
  :setup (cons (switch-to-buffer (get-buffer-create "scaling-supports"))
               (mapcar (lambda (name) (list :family name :weight 'bold)) (scaling--names "Scaling Family" n)))
  :body (progn (dotimes (_ 2) (dolist (query (cdr it)) (display-supports-face-attributes-p query)))
               (with-current-buffer (car it) (insert "x"))))
