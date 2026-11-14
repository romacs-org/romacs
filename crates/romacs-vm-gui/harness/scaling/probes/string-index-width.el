;;; string-index-width.el --- scaling probe  -*- lexical-binding: t -*-

;; String-width of one character at increasing starts over a multibyte string of 2N characters:
;; each read goes through the string's own character/byte cursor, so the
;; scan is linear in N, not quadratic (#202).
(scaling-probe string-index-width :n 4000
  :setup (apply #'concat (make-list n "aλ"))
  :body (let ((len (length it)) (i 0)) (while (< i len) (string-width it i (1+ i)) (setq i (1+ i)))))
