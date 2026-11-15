;;; string-index-compare.el --- scaling probe  -*- lexical-binding: t -*-

;; Compare-strings at increasing starts, as a wake parser scans a line over a multibyte string of 2N characters:
;; each read goes through the string's own character/byte cursor, so the
;; scan is linear in N, not quadratic (#202).
(scaling-probe string-index-compare :n 4000
  :setup (apply #'concat (make-list n "aλ"))
  :body (let ((len (length it)) (i 0)) (while (< i len) (compare-strings it i (1+ i) "λ" 0 1) (setq i (1+ i)))))
