;;; string-index-search.el --- scaling probe  -*- lexical-binding: t -*-

;; String-search from each match onward over a multibyte string of 2N characters:
;; each read goes through the string's own character/byte cursor, so the
;; scan is linear in N, not quadratic (#202).
(scaling-probe string-index-search :n 4000
  :setup (apply #'concat (make-list n "aλ"))
  :body (let ((i 0)) (while (setq i (string-search "a" it i)) (setq i (1+ i)))))
