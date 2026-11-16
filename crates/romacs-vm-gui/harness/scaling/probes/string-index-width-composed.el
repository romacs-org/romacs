;;; string-index-width-composed.el --- scaling probe  -*- lexical-binding: t -*-

;; String-width of one character at increasing starts over a string of N
;; emoji sequences (VS-16), each a composition: a call decodes and matches
;; only what it reads, so the scan is linear in N, not quadratic.
(scaling-probe string-index-width-composed :n 2000
  :setup (apply #'concat (make-list n "a\N{HEAVY BLACK HEART}\N{VARIATION SELECTOR-16}"))
  :body (let ((len (length it)) (i 0)) (while (< i len) (string-width it i (1+ i)) (setq i (1+ i)))))
