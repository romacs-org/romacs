;;; string-index-aref.el --- scaling probe  -*- lexical-binding: t -*-

;; Aref at increasing indices over a multibyte string of 2N characters:
;; each read goes through the string's own character/byte cursor, so the
;; scan is linear in N, not quadratic (#202).
(scaling-probe string-index-aref :n 4000
  :setup (apply #'concat (make-list n "aλ"))
  :body (let ((len (length it)) (i 0)) (while (< i len) (aref it i) (setq i (1+ i)))))
