;;; theme-floor-memo.el --- scaling probe  -*- lexical-binding: t -*-

;; N distinct floor questions, past the memo's bound: an answer is kept or the
;; memo cleared, never the whole of it walked per question.
(scaling-probe theme-floor-memo :n 6000 :k 6
  :setup (progn
           (require 'romacs-theme)
           (clrhash romacs-color--memo)
           (mapcar (lambda (i) (format "#%06x" (* i 397))) (number-sequence 1 n)))
  :body (dolist (color it) (romacs-color-floor-on color '("#101418") 4.5)))
